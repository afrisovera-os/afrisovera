//! The desk's text shaper, phase C1 of SHAPING_PLAN.md: Arabic shaped by the font's own lookups (gsub.rs: joining
//! forms, the dotless letters and their dots, lam with alef, marks placed on their letters), right to left and mixed
//! lines by the bidi algorithm (bidi.rs), mirrored brackets. The tables are generated offline from the pinned Noto
//! Sans Arabic file and the Unicode Character Database (shape_data.rs, tools/face/shape_src.py); nothing here parses a
//! font or a data file.
//!
//! Only strings that need it come here: `needed` is the fast test that `vtext` and `vtext_w` both make, so a string
//! with no right-to-left or shaped character takes the section 26 path unchanged and every Latin screen stays as it
//! was, pixel for pixel.
//!
//! The text is untrusted (app window titles, notices, file names), so (the architect's conditions): every table read
//! is a bounded search or a checked `get`; a line is at most MAX characters, the rest cut and counted; the explicit
//! bidi controls are dropped and counted; no heap; the fuzz target `shape_text` (fuzz-host) runs any bytes through
//! `layout`. Mainline only, like all of the desk's drawing; a second entry panics rather than mixing two lines.
//!
//! Positions are kept in font units (both faces have 1000 units an em) and turned into pixels once, at the end.
#![allow(dead_code)]

use super::bidi;
use super::gsub::{self, Gi, MAXG};
use super::shape_data as sd;
use super::vector;

pub const MAX: usize = bidi::MAX;

// joining types, in shape_data's order
const JU: u8 = 0;
const JR: u8 = 1;
const JD: u8 = 2;
const JC: u8 = 3;
const JL: u8 = 4;
const JT: u8 = 5;

/// One glyph of a shaped line, given to `layout`'s caller.
#[derive(Clone, Copy)]
pub struct Glyph {
    /// vector::LATIN or vector::ARABIC
    pub face: u8,
    pub g: u16,
    /// no glyph for the character: draw the box
    pub boxed: bool,
    /// advance in font units (0 for a mark)
    pub adv: i32,
    /// the character (after mirroring) the glyph is for, and its index in the line
    pub cp: u32,
    pub ch: u16,
}

/// All zero, so the buffers and the line cache below are .bss, not .data in the image.
const NONE: Glyph = Glyph { face: 0, g: 0, boxed: false, adv: 0, cp: 0, ch: 0 };

/// (explicit controls dropped, characters cut past MAX, lines shaped) since boot, for `desk shape probe`.
pub static mut STATS: (u64, u64, u64) = (0, 0, 0);

static mut CPS: [u32; MAX] = [0; MAX];
static mut LEVELS: [u8; MAX] = [0; MAX];
static mut FACE: [u8; MAX] = [0; MAX];
static mut RUN: [Gi; MAXG] = [gsub::EMPTY; MAXG];
static mut ATT: [Option<(u16, i16, i16)>; MAXG] = [None; MAXG];
static mut KADJ: [(i32, i32); MAXG] = [(0, 0); MAXG];
/// Each glyph's kerning shift of where it is drawn (x placement), font units.
static mut GXO: [i32; MAXG] = [0; MAXG];
static mut GL: [Glyph; MAXG] = [NONE; MAXG];
static mut GLV: [u8; MAXG] = [0; MAXG];
static mut GATT: [Option<(u16, i16, i16)>; MAXG] = [None; MAXG];
static mut ORDER: [u16; MAXG] = [0; MAXG];
static mut XS: [i32; MAXG] = [0; MAXG];
static mut YS: [i32; MAXG] = [0; MAXG];

static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The fast test. False (today's path) for a string with no byte at or above 0xD6 (the lead byte of U+0590), without
/// decoding it; otherwise the string is decoded once and is true only if it holds a right-to-left or Arabic
/// character or an explicit bidi control. So Yoruba's dotted vowels and every other Latin string stay on the old
/// path. `vtext` and `vtext_w` both decide by this, so a measured width always matches the drawn one.
pub fn needed(s: &[u8]) -> bool {
    if !s.iter().any(|&b| b >= 0xD6) {
        return false;
    }
    let mut i = 0;
    while i < s.len() {
        let cp = vector::next_char(s, &mut i);
        if cp >= 0x0590
            && (bidi::is_explicit(cp)
                || matches!(bidi::class(cp), bidi::R | bidi::AL | bidi::AN)
                || vector::lookup_arabic(cp).is_some())
        {
            return true;
        }
    }
    false
}

fn joining(cp: u32) -> u8 {
    let i = sd::JOIN.partition_point(|&(s, _)| s <= cp);
    match i.checked_sub(1).and_then(|i| sd::JOIN.get(i)) {
        Some(&(_, t)) => t,
        None => JU,
    }
}

/// HarfBuzz's modified combining class of a haraka (hb-unicode.hh): shadda 27 sorts before the vowels 28 to 33 and
/// sukun 34. None for anything else.
fn haraka_class(cp: u32) -> Option<u8> {
    match cp {
        0x0651 => Some(27),
        0x064B..=0x0650 => Some(28 + (cp - 0x064B) as u8),
        0x0652 => Some(34),
        0x0670 => Some(35),
        0x0653 | 0x0654 => Some(230),
        0x0655 => Some(220),
        _ => None,
    }
}

/// HarfBuzz's Arabic "modifier combining marks" (hb-ot-shaper-arabic.cc): after the sort they move to the front of
/// the marks, those of class 220 first, then 230, so a hamza is applied before the vowels.
fn is_mcm(cp: u32) -> bool {
    matches!(cp, 0x0654 | 0x0655 | 0x0658 | 0x06DC | 0x06E3 | 0x06E7 | 0x06E8)
}

/// Each run of Arabic marks: a stable sort by modified class, then the hamza marks to the front, as HarfBuzz does
/// (runs are short; at most MAX characters).
fn reorder_harakat(cps: &mut [u32]) {
    let mut s = 0;
    while s < cps.len() {
        if haraka_class(cps[s]).is_none() {
            s += 1;
            continue;
        }
        let mut e = s;
        while e < cps.len() && haraka_class(cps[e]).is_some() {
            e += 1;
        }
        for i in s + 1..e {
            let mut j = i;
            while j > s && haraka_class(cps[j - 1]) > haraka_class(cps[j]) {
                cps.swap(j - 1, j);
                j -= 1;
            }
        }
        let mut start = s;
        for cc in [220u8, 230] {
            let mut i = start;
            while i < e && haraka_class(cps[i]).map_or(0, |c| c) < cc {
                i += 1;
            }
            if i == e || haraka_class(cps[i]) != Some(cc) {
                continue;
            }
            let mut j = i;
            while j < e && haraka_class(cps[j]) == Some(cc) && is_mcm(cps[j]) {
                j += 1;
            }
            if i == j {
                continue;
            }
            cps[start..j].rotate_right(j - i);
            start += j - i;
        }
        s = e;
    }
}

/// The canonical decomposition of a character face 1 lacks (a bounded binary search of a generated table).
fn decompose(cp: u32) -> Option<(u32, u32)> {
    let i = sd::DECOMP.binary_search_by_key(&cp, |r| r.0).ok()?;
    sd::DECOMP.get(i).map(|r| (r.1, r.2))
}

fn arabic_script(cp: u32) -> bool {
    (0x0600..=0x06FF).contains(&cp)
}

/// Shape `s` (UTF-8, untrusted) at a cap height and lay it out. `rtl`: the paragraph direction when the caller knows
/// it, else from the first strong character (left to right when none). `f` gets each glyph with where to draw it:
/// x in 1/256 pixel from the line's start, y in whole pixels down from the baseline. Glyphs come in visual order.
/// Returns the line's advance in 1/256 pixel.
pub fn layout(s: &[u8], cap: i32, rtl: Option<bool>, f: &mut dyn FnMut(&Glyph, i32, i32)) -> i32 {
    if BUSY.swap(true, core::sync::atomic::Ordering::Acquire) {
        panic!("shape::layout entered twice");
    }
    let cap = vector::clamp_cap(cap);
    let mut emit = |g: &Glyph, x: i32, y: i32| f(g, vector::px256(x, cap), -vector::px_round(y, cap));
    let w = match cache_find(s, cap, rtl) {
        Some((w, recs)) => {
            for r in recs {
                emit(&r.0, r.1, r.2);
            }
            w
        }
        None => {
            // SAFETY: mainline only (BUSY); the scratch list is only touched here.
            let tmp = unsafe { &mut *core::ptr::addr_of_mut!(TMP) };
            let mut k = 0;
            let w = layout_inner(s, cap, rtl, &mut |g, x, y| {
                if k < tmp.len() {
                    tmp[k] = (*g, x, y);
                }
                k += 1;
                emit(g, x, y);
            });
            if k <= tmp.len() {
                cache_put(s, cap, rtl, w, &tmp[..k]);
            }
            w
        }
    };
    BUSY.store(false, core::sync::atomic::Ordering::Release);
    vector::px256(w, cap)
}

// The shaped-line cache. The desk draws the same titles, labels and notices every frame, so a line is shaped once
// and its glyphs and places kept: a slot (the bytes' hash, the cap, the direction, where its entry starts) and, in
// the arena, the line's bytes (compared whole on a hit, so a hash collision cannot show one string as another), its
// width and its glyphs. When the arena or the slots are full, all is emptied and refilled, as the glyph cache does.
const CSLOTS: usize = 64;
const CARENA: usize = 1536; // entries of (Glyph, x, y); a line's bytes take ceil(len / 16) entries
const CMAXB: usize = 1024; // longer lines are shaped every time
#[derive(Clone, Copy)]
struct CSlot {
    hash: u64,
    cap: i32,
    rtl: u8,
    len: u16,
    off: u16,
    nglyph: u16,
    w: i32,
}
const CEMPTY: CSlot = CSlot { hash: 0, cap: 0, rtl: 0, len: 0, off: 0, nglyph: 0, w: 0 };
static mut CSLOT: [CSlot; CSLOTS] = [CEMPTY; CSLOTS];
static mut CNS: usize = 0;
static mut CARN: [(Glyph, i32, i32); CARENA] = [(NONE, 0, 0); CARENA];
static mut CUSED: usize = 0;
static mut TMP: [(Glyph, i32, i32); MAXG] = [(NONE, 0, 0); MAXG];
/// (hits, misses, emptied) since boot.
pub static mut CSTATS: (u64, u64, u64) = (0, 0, 0);

fn chash(s: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF29CE484222325;
    for &b in s {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001B3);
    }
    h
}

fn rtl_key(rtl: Option<bool>) -> u8 {
    match rtl {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

/// The line's bytes as stored: 16 bytes in each entry's glyph and places (packed into the `cp`, `x` and `y` words
/// and the glyph id), so the arena holds one kind of entry.
fn pack(b: &[u8]) -> (Glyph, i32, i32) {
    let mut w = [0u8; 16];
    w[..b.len()].copy_from_slice(b);
    let u = |i: usize| u32::from_le_bytes([w[i], w[i + 1], w[i + 2], w[i + 3]]);
    (
        Glyph { face: 0, g: u16::from_le_bytes([w[12], w[13]]), boxed: false, adv: u(4) as i32, cp: u(0), ch: u16::from_le_bytes([w[14], w[15]]) },
        u(8) as i32,
        0,
    )
}

fn cache_find(s: &[u8], cap: i32, rtl: Option<bool>) -> Option<(i32, &'static [(Glyph, i32, i32)])> {
    if s.len() > CMAXB {
        return None;
    }
    let h = chash(s);
    // SAFETY: mainline only (BUSY).
    let (slots, ns, arena) = unsafe { (&*core::ptr::addr_of!(CSLOT), CNS, &*core::ptr::addr_of!(CARN)) };
    for sl in slots.iter().take(ns) {
        if sl.hash != h || sl.cap != cap || sl.rtl != rtl_key(rtl) || sl.len as usize != s.len() {
            continue;
        }
        let nb = (s.len() + 15) / 16;
        let off = sl.off as usize;
        let stored = arena.get(off..off + nb)?;
        if stored.iter().zip(s.chunks(16)).all(|(e, c)| {
            let p = pack(c);
            e.0.cp == p.0.cp && e.0.adv == p.0.adv && e.1 == p.1 && e.0.g == p.0.g && e.0.ch == p.0.ch
        }) {
            // SAFETY: a scalar counter.
            unsafe { CSTATS.0 += 1 };
            return arena.get(off + nb..off + nb + sl.nglyph as usize).map(|r| (sl.w, r));
        }
    }
    // SAFETY: a scalar counter.
    unsafe { CSTATS.1 += 1 };
    None
}

fn cache_put(s: &[u8], cap: i32, rtl: Option<bool>, w: i32, recs: &[(Glyph, i32, i32)]) {
    if s.len() > CMAXB {
        return;
    }
    let nb = (s.len() + 15) / 16;
    let need = nb + recs.len();
    if need > CARENA {
        return;
    }
    // SAFETY: mainline only (BUSY).
    unsafe {
        if CUSED + need > CARENA || CNS >= CSLOTS {
            reset_cache();
        }
        let arena = &mut *core::ptr::addr_of_mut!(CARN);
        let off = CUSED;
        for (k, c) in s.chunks(16).enumerate() {
            arena[off + k] = pack(c);
        }
        arena[off + nb..off + need].copy_from_slice(recs);
        CUSED += need;
        (&mut *core::ptr::addr_of_mut!(CSLOT))[CNS] =
            CSlot { hash: chash(s), cap, rtl: rtl_key(rtl), len: s.len() as u16, off: off as u16, nglyph: recs.len() as u16, w };
        CNS += 1;
    }
}

/// Empty the shaped-line cache (for the probe's measurements; it also empties itself when full).
pub fn reset_cache() {
    // SAFETY: mainline only.
    unsafe {
        CNS = 0;
        CUSED = 0;
        CSTATS.2 += 1;
    }
}

/// The same in font units (x from the line's start, y up). Returns the advance in font units.
fn layout_inner(s: &[u8], cap: i32, rtl: Option<bool>, f: &mut dyn FnMut(&Glyph, i32, i32)) -> i32 {
    // SAFETY: mainline only, and BUSY keeps this the only user of the buffers.
    // SAFETY: mainline only (BUSY), touched only here.
    let gxo = unsafe { &mut *core::ptr::addr_of_mut!(GXO) };
    let (cps, levels, face, run, att, gl, glv, gatt, order, xs, ys) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(CPS),
            &mut *core::ptr::addr_of_mut!(LEVELS),
            &mut *core::ptr::addr_of_mut!(FACE),
            &mut *core::ptr::addr_of_mut!(RUN),
            &mut *core::ptr::addr_of_mut!(ATT),
            &mut *core::ptr::addr_of_mut!(GL),
            &mut *core::ptr::addr_of_mut!(GLV),
            &mut *core::ptr::addr_of_mut!(GATT),
            &mut *core::ptr::addr_of_mut!(ORDER),
            &mut *core::ptr::addr_of_mut!(XS),
            &mut *core::ptr::addr_of_mut!(YS),
        )
    };
    // 1. decode, dropping the explicit controls; at most MAX characters
    let (mut n, mut i, mut dropped, mut cut) = (0usize, 0usize, 0u64, 0u64);
    while i < s.len() {
        let cp = vector::next_char(s, &mut i);
        if bidi::is_explicit(cp) {
            dropped += 1;
            continue;
        }
        // a character face 1 lacks but whose canonical parts it has is drawn as its parts (as HarfBuzz decomposes)
        let parts = if vector::lookup_arabic(cp).is_none() { decompose(cp) } else { None };
        match parts {
            Some((a, b)) if n + 2 <= MAX => {
                cps[n] = a;
                cps[n + 1] = b;
                n += 2;
            }
            None if n < MAX => {
                cps[n] = cp;
                n += 1;
            }
            _ => cut += 1,
        }
    }
    // SAFETY: mainline only.
    unsafe {
        STATS.0 += dropped;
        STATS.1 += cut;
        STATS.2 += 1;
    }
    // 1b. the harakat in each run of them sorted as HarfBuzz sorts them (its modified combining classes put shadda
    // first, so the font's shadda-with-vowel ligatures meet their parts in the order they expect); a stable sort
    reorder_harakat(&mut cps[..n]);
    // 2. levels, and the characters drawn mirrored
    bidi::resolve(&cps[..n], rtl, &mut levels[..n]);
    for k in 0..n {
        let lv = levels[k];
        if lv != bidi::REMOVED && lv & 1 == 1 {
            if let Some(m) = bidi::mirror(cps[k]) {
                cps[k] = m;
            }
        }
    }
    // 3. which face: Arabic characters the face has are face 1; a space between two of them is too (HarfBuzz's run
    // would hold it); everything else face 0
    for k in 0..n {
        let cp = cps[k];
        face[k] = (arabic_script(cp) && vector::lookup_arabic(cp).is_some()) as u8;
    }
    if vector::lookup_arabic(0x20).is_some() {
        for k in 0..n {
            if cps[k] == 0x20 && face[k] == 0 {
                let before = (0..k).rev().find(|&j| cps[j] != 0x20).map_or(false, |j| face[j] == 1);
                let after = (k + 1..n).find(|&j| cps[j] != 0x20).map_or(false, |j| face[j] == 1);
                if before && after {
                    face[k] = 2;
                }
            }
        }
        for v in face[..n].iter_mut() {
            if *v == 2 {
                *v = 1;
            }
        }
    }
    // 4. joining: each Arabic character's form mask, from its nearest non-transparent neighbours
    let mut jts = [JU; MAX];
    for k in 0..n {
        jts[k] = if face[k] == 1 { joining(cps[k]) } else { JU };
    }
    let mut form = [0u16; MAX];
    {
        let mut prev_of = [None::<u8>; MAX];
        let mut last = None;
        for k in 0..n {
            prev_of[k] = last;
            if jts[k] != JT {
                last = Some(jts[k]);
            }
        }
        let mut next = None;
        for k in (0..n).rev() {
            let jt = jts[k];
            if matches!(jt, JR | JD | JL | JC) {
                let to_prev = matches!(jt, JD | JR | JC) && matches!(prev_of[k], Some(JD | JL | JC));
                let to_next = matches!(jt, JD | JL | JC) && matches!(next, Some(JD | JR | JC));
                form[k] = match (to_prev, to_next) {
                    (true, true) => sd::M_MEDI,
                    (true, false) => sd::M_FINA,
                    (false, true) => sd::M_INIT,
                    (false, false) => sd::M_ISOL,
                };
            }
            if jt != JT {
                next = Some(jt);
            }
        }
    }
    // 5. the glyphs in logical order: face 0 one per character; each run of face 1 characters at one level shaped
    // by the font's lookups, its marks hung on their glyphs
    let mut m = 0usize;
    let mut k = 0usize;
    while k < n {
        if levels[k] == bidi::REMOVED {
            k += 1;
            continue;
        }
        if face[k] == 1 {
            let lv = levels[k];
            let mut rn = 0;
            while k < n && face[k] == 1 && (levels[k] == lv || levels[k] == bidi::REMOVED) {
                if levels[k] != bidi::REMOVED && rn < MAXG {
                    let g = vector::lookup_arabic(cps[k]).unwrap_or(0);
                    let mask = sd::M_ALL | if lv & 1 == 1 { sd::M_RTL } else { 0 } | form[k];
                    run[rn] = Gi { g, cl: k as u16, mask };
                    rn += 1;
                }
                k += 1;
            }
            let mut r = gsub::Run::new(run, rn);
            r.substitute();
            r.attach(att);
            // SAFETY: mainline only (BUSY), touched only here.
            let kadj = unsafe { &mut *core::ptr::addr_of_mut!(KADJ) };
            r.kern(kadj);
            let rn = r.n;
            let base = m;
            for j in 0..rn {
                if m >= MAXG {
                    break;
                }
                let gi = run[j];
                let adv = if gsub::class(gi.g) == 3 { 0 } else { vector::units(vector::ARABIC, gi.g).saturating_add(kadj[j].1) };
                let ch = (gi.cl as usize).min(n.saturating_sub(1));
                gl[m] = Glyph { face: vector::ARABIC, g: gi.g, boxed: false, adv, cp: cps[ch], ch: gi.cl };
                gxo[m] = if gsub::class(gi.g) == 3 { 0 } else { kadj[j].0 };
                glv[m] = levels[ch];
                gatt[m] = att[j].and_then(|(t, dx, dy)| {
                    let t = base + t as usize;
                    (t < m).then_some((t as u16, dx, dy))
                });
                m += 1;
            }
            continue;
        }
        if m < MAXG {
            let cp = cps[k];
            gl[m] = match vector::lookup_at(cp, cap) {
                Some(g) => Glyph { face: vector::LATIN, g, boxed: false, adv: vector::units(vector::LATIN, g), cp, ch: k as u16 },
                None => Glyph { face: vector::LATIN, g: 0, boxed: true, adv: vector::box_units(cap), cp, ch: k as u16 },
            };
            glv[m] = levels[k];
            gatt[m] = None;
            gxo[m] = 0;
            m += 1;
        }
        k += 1;
    }
    // 6. visual order (L2) over the glyphs, then the pens of the glyphs that hang on nothing
    let cnt = bidi::reorder(&glv[..m], &mut order[..m]);
    let mut pen = 0i32;
    for &j in &order[..cnt] {
        let j = j as usize;
        xs[j] = pen.saturating_add(gxo[j]);
        ys[j] = 0;
        pen = pen.saturating_add(gl[j].adv);
    }
    // marks, in logical order (the glyph a mark hangs on always comes before it)
    for j in 0..m {
        if let Some((t, dx, dy)) = gatt[j] {
            let t = t as usize;
            xs[j] = xs[t].saturating_add(dx as i32);
            ys[j] = ys[t].saturating_add(dy as i32);
        }
    }
    for &j in &order[..cnt] {
        let j = j as usize;
        f(&gl[j], xs[j], ys[j]);
    }
    pen
}

/// The oracle hash of a line (tools/face/shape_expected.txt): FNV-1a 64 over (glyph u16, x i32, y i32) in font
/// units of every glyph in visual order, x where it is drawn. A face 0 glyph or a box counts as glyph 0xFFFF, so a
/// line that should have been all Arabic cannot match by accident.
pub fn oracle_hash(s: &[u8]) -> u64 {
    if BUSY.swap(true, core::sync::atomic::Ordering::Acquire) {
        panic!("shape::layout entered twice");
    }
    let mut h: u64 = 0xCBF29CE484222325;
    layout_inner(s, 9, None, &mut |g, x, y| {
        let gid = if g.face == vector::ARABIC && !g.boxed { g.g } else { 0xFFFF };
        for b in gid.to_le_bytes().iter().chain(x.to_le_bytes().iter()).chain(y.to_le_bytes().iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001B3);
        }
    });
    BUSY.store(false, core::sync::atomic::Ordering::Release);
    h
}
