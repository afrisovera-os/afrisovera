//! Devanagari shaping (SHAPING_C3_PLAN.md, phase C3): HarfBuzz's Indic behaviour for the `dev2` script, written from
//! its documented behaviour (hb-ot-shaper-indic.cc and its syllable grammar) and checked against HarfBuzz itself as the
//! oracle (tools/face/shape_expected_deva.txt). The font's lookups run in gsub.rs from the tables compiled offline into
//! shape_data_deva.rs; this file does what the script needs around them:
//!   1. each character's Indic category and position (generated from the UCD, the INDIC table), and each
//!      consonant's position from the face (does it have a below-base or post-base form?);
//!   2. syllables, by the grammar, with a dotted circle put in front of a broken cluster;
//!   3. locl and ccmp in each syllable alone; the initial reordering (the base consonant, reph, the pre-base matra,
//!      positions, a stable sort, the feature masks, the joiners' effects); the basic features (nukt to cjct);
//!   4. the final reordering (the pre-base matra after the half forms, reph to its place); the presentation features.
//!
//! The text is untrusted: every table read is checked or a bounded search, a syllable is at most the run, the
//! dotted circle is only put in when the buffer has room, and gsub.rs's budget and depth bounds still hold. No heap.
#![allow(dead_code)]

use super::gsub::{self, Gi};
use super::shape_data_deva as dv;
use super::vector;

// categories (shape_src.py I_CAT)
const X: u8 = 0;
const C: u8 = 1;
const V: u8 = 2;
const N: u8 = 3;
const H: u8 = 4;
const ZWNJ: u8 = 5;
const ZWJ: u8 = 6;
const M: u8 = 7;
const SM: u8 = 8;
const A: u8 = 9;
const PLACEHOLDER: u8 = 10;
const DOTTEDCIRCLE: u8 = 11;
const RA: u8 = 12;
const SYMBOL: u8 = 13;

// positions, HarfBuzz's order (the table's values, with FINAL_C put in before SMVD here)
const P_START: u8 = 0;
const P_RA_TO_BECOME_REPH: u8 = 1;
const P_PRE_M: u8 = 2;
const P_PRE_C: u8 = 3;
const P_BASE_C: u8 = 4;
const P_AFTER_MAIN: u8 = 5;
const P_ABOVE_C: u8 = 6;
const P_BEFORE_SUB: u8 = 7;
const P_BELOW_C: u8 = 8;
const P_AFTER_SUB: u8 = 9;
const P_BEFORE_POST: u8 = 10;
const P_POST_C: u8 = 11;
const P_AFTER_POST: u8 = 12;
const P_FINAL_C: u8 = 13;
const P_SMVD: u8 = 14;
const P_END: u8 = 15;

// syllable types
const S_CONSONANT: u16 = 0;
const S_VOWEL: u16 = 1;
const S_STANDALONE: u16 = 2;
const S_SYMBOL: u16 = 3;
const S_BROKEN: u16 = 4;
const S_OTHER: u16 = 5;

fn cat(g: &Gi) -> u8 {
    (g.aux & 0x1F) as u8
}
fn pos(g: &Gi) -> u8 {
    ((g.aux >> 5) & 0x1F) as u8
}
fn set_cat(g: &mut Gi, c: u8) {
    g.aux = (g.aux & !0x1F) | c as u16;
}
fn set_pos(g: &mut Gi, p: u8) {
    g.aux = (g.aux & !(0x1F << 5)) | ((p as u16 & 0x1F) << 5);
}
fn ligated(g: &Gi) -> bool {
    g.aux & gsub::LIGATED != 0
}
fn multiplied(g: &Gi) -> bool {
    g.aux & gsub::MULTIPLIED != 0
}
/// HarfBuzz's is_one_of: a glyph that came from a ligature has no category any more.
fn is_one_of(g: &Gi, cats: &[u8]) -> bool {
    !ligated(g) && cats.contains(&cat(g))
}
const CONSONANTS: [u8; 5] = [C, RA, V, PLACEHOLDER, DOTTEDCIRCLE];
fn is_consonant(g: &Gi) -> bool {
    is_one_of(g, &CONSONANTS)
}
fn is_halant(g: &Gi) -> bool {
    is_one_of(g, &[H])
}
fn is_joiner(g: &Gi) -> bool {
    is_one_of(g, &[ZWJ, ZWNJ])
}

/// The category and position of a code point (the generated INDIC table; X and END for anything else).
pub fn props(cp: u32) -> (u8, u8) {
    match dv::INDIC.binary_search_by_key(&cp, |r| r.0) {
        Ok(i) => {
            let (_, c, p) = dv::INDIC[i];
            // the table has no FINAL_C: its SMVD and END come one later here
            (c, if p >= P_FINAL_C { p + 1 } else { p })
        }
        Err(_) => (X, P_END),
    }
}

/// A face's feature stages (STAGE_FEAT): only the Devanagari face has them today; C4's faces add theirs here.
fn stage_feat(t: &'static gsub::Tables) -> &'static [(u16, [u8; 4])] {
    if core::ptr::eq(t, &gsub::DEVA) {
        &dv::STAGE_FEAT
    } else {
        &[]
    }
}

/// The stage of a feature in a face, if the face has it.
fn stage_of(t: &'static gsub::Tables, tag: &[u8; 4]) -> Option<usize> {
    stage_feat(t).iter().find(|r| &r.1 == tag).map(|r| r.0 as usize)
}

fn would(t: &'static gsub::Tables, tag: &[u8; 4], glyphs: &[u16]) -> bool {
    match stage_of(t, tag) {
        Some(si) => gsub::would_substitute(t, si, glyphs),
        None => false,
    }
}

/// A face's key for the caches below: glyph ids mean something only within one face (the architect's C3b fix).
fn face_key(t: &'static gsub::Tables) -> usize {
    t as *const gsub::Tables as usize
}

/// A consonant's position from the face (HarfBuzz's consonant_position_from_face): below the base if it has a
/// below-base form with the virama on either side, after it if it has a post-base or pre-base-reordering form.
/// Kept per glyph after the first ask (mainline only), for one face at a time: a new face clears it.
static mut CPOS: [u8; 1024] = [0; 1024];
static mut CPOS_FACE: usize = 0;

fn consonant_position(t: &'static gsub::Tables, g: u16, virama: u16) -> u8 {
    // SAFETY: mainline only; the shaper's re-entry guard covers it.
    let cache = unsafe { &mut *core::ptr::addr_of_mut!(CPOS) };
    let face = unsafe { &mut *core::ptr::addr_of_mut!(CPOS_FACE) };
    if *face != face_key(t) {
        cache.fill(0);
        *face = face_key(t);
    }
    if let Some(&p) = cache.get(g as usize) {
        if p != 0 {
            return p - 1;
        }
    }
    let a = [virama, g];
    let b = [g, virama];
    let p = if would(t, b"blwf", &a) || would(t, b"blwf", &b) || would(t, b"vatu", &a) || would(t, b"vatu", &b) {
        P_BELOW_C
    } else if would(t, b"pstf", &a) || would(t, b"pstf", &b) || would(t, b"pref", &a) || would(t, b"pref", &b) {
        P_POST_C
    } else {
        P_BASE_C
    };
    if let Some(c) = cache.get_mut(g as usize) {
        *c = p + 1;
    }
    p
}

/// Does the face's rphf make a reph of these two glyphs? Asked once per (face, pair) and kept (C3b: it was asked for
/// every syllable). Keyed on the face because glyph ids are per face. When full, the oldest slot is replaced.
static mut REPH: [(usize, u16, u16, u8); 4] = [(0, 0, 0, 0); 4];
static mut REPH_NEXT: usize = 0;

pub fn reph_forms(t: &'static gsub::Tables, a: u16, h: u16) -> bool {
    // SAFETY: mainline only; the shaper's re-entry guard covers it.
    let cache = unsafe { &mut *core::ptr::addr_of_mut!(REPH) };
    let next = unsafe { &mut *core::ptr::addr_of_mut!(REPH_NEXT) };
    let f = face_key(t);
    for c in cache.iter() {
        if c.3 != 0 && c.0 == f && c.1 == a && c.2 == h {
            return c.3 == 2;
        }
    }
    let yes = would(t, b"rphf", &[a, h]);
    let k = *next % cache.len();
    cache[k] = (f, a, h, if yes { 2 } else { 1 });
    *next = k + 1;
    yes
}

// ---- the syllable grammar (HarfBuzz's Ragel machine for Indic, the parts Devanagari reaches) ----

struct Cats<'a>(&'a [u8]);

impl Cats<'_> {
    fn is(&self, i: usize, set: &[u8]) -> bool {
        self.0.get(i).map_or(false, |c| set.contains(c))
    }
    fn opt(&self, i: usize, set: &[u8]) -> usize {
        if self.is(i, set) {
            i + 1
        } else {
            i
        }
    }
    /// n = (N N?)?
    fn n(&self, i: usize) -> usize {
        let j = self.opt(i, &[N]);
        if j > i {
            self.opt(j, &[N])
        } else {
            i
        }
    }
    /// cn = c ZWJ? n
    fn cn(&self, i: usize) -> Option<usize> {
        if !self.is(i, &[C, RA]) {
            return None;
        }
        let j = self.opt(i + 1, &[ZWJ]);
        Some(self.n(j))
    }
    /// halant_group = z? H (ZWJ N?)?
    fn halant_group(&self, i: usize) -> Option<usize> {
        let j = self.opt(i, &[ZWJ, ZWNJ]);
        if !self.is(j, &[H]) {
            return None;
        }
        let k = j + 1;
        if self.is(k, &[ZWJ]) {
            Some(self.opt(k + 1, &[N]))
        } else {
            Some(k)
        }
    }
    /// final_halant_group = halant_group | H ZWNJ
    fn final_halant_group(&self, i: usize) -> Option<usize> {
        let a = self.halant_group(i);
        let b = if self.is(i, &[H]) && self.is(i + 1, &[ZWNJ]) { Some(i + 2) } else { None };
        a.max(b)
    }
    /// matra_group = z* M N? H?
    fn matra_group(&self, i: usize) -> Option<usize> {
        let mut j = i;
        while self.is(j, &[ZWJ, ZWNJ]) {
            j += 1;
        }
        if !self.is(j, &[M]) {
            return None;
        }
        let k = self.opt(j + 1, &[N]);
        Some(self.opt(k, &[H]))
    }
    /// syllable_tail = (z? SM SM? ZWNJ?)? A*
    fn syllable_tail(&self, i: usize) -> usize {
        let mut j = i;
        let z = self.opt(j, &[ZWJ, ZWNJ]);
        if self.is(z, &[SM]) {
            j = self.opt(z + 1, &[SM]);
            j = self.opt(j, &[ZWNJ]);
        }
        while self.is(j, &[A]) {
            j += 1;
        }
        j
    }
    /// complex_syllable_tail = (halant_group cn)* halant_or_matra_group syllable_tail, the longest way
    fn complex_tail(&self, i: usize) -> usize {
        let mut j = i;
        loop {
            match self.halant_group(j).and_then(|h| self.cn(h)) {
                Some(k) if k > j => j = k,
                _ => break,
            }
        }
        // halant_or_matra_group: a final halant group, or any number of matra groups (possibly none)
        let mut best = self.syllable_tail(j);
        if let Some(k) = self.final_halant_group(j) {
            best = best.max(self.syllable_tail(k));
        }
        let mut k = j;
        while let Some(m) = self.matra_group(k) {
            if m <= k {
                break;
            }
            k = m;
            best = best.max(self.syllable_tail(k));
        }
        best
    }
    fn reph(&self, i: usize) -> usize {
        if self.is(i, &[RA]) && self.is(i + 1, &[H]) {
            i + 2
        } else {
            i
        }
    }
    /// The longest syllable starting at `i`: (end, type).
    fn syllable(&self, i: usize) -> (usize, u16) {
        let mut best = (i + 1, S_OTHER);
        let mut take = |e: usize, t: u16, best: &mut (usize, u16)| {
            if e > best.0 || (e == best.0 && best.1 == S_OTHER && e > i) {
                *best = (e, t);
            }
        };
        // consonant_syllable = cn complex_syllable_tail
        if let Some(k) = self.cn(i) {
            take(self.complex_tail(k), S_CONSONANT, &mut best);
        }
        // vowel_syllable = reph? V n (ZWJ | complex_syllable_tail)
        for r in [i, self.reph(i)] {
            if self.is(r, &[V]) {
                let k = self.n(r + 1);
                let e = self.complex_tail(k).max(if self.is(k, &[ZWJ]) { k + 1 } else { k });
                take(e, S_VOWEL, &mut best);
            }
        }
        // standalone_cluster = (PLACEHOLDER | reph? DOTTEDCIRCLE) n complex_syllable_tail
        if self.is(i, &[PLACEHOLDER]) {
            take(self.complex_tail(self.n(i + 1)), S_STANDALONE, &mut best);
        }
        for r in [i, self.reph(i)] {
            if self.is(r, &[DOTTEDCIRCLE]) {
                take(self.complex_tail(self.n(r + 1)), S_STANDALONE, &mut best);
            }
        }
        // symbol_cluster = SYMBOL N? syllable_tail
        if self.is(i, &[SYMBOL]) {
            take(self.syllable_tail(self.opt(i + 1, &[N])), S_SYMBOL, &mut best);
        }
        // broken_cluster = reph? n complex_syllable_tail (not empty)
        for r in [i, self.reph(i)] {
            let e = self.complex_tail(self.n(r));
            if e > i {
                take(e, S_BROKEN, &mut best);
            }
        }
        best
    }
}

/// The syllable starting at `i` of a run whose categories are `cats`: (its end, its type). For fuzz-host's
/// deva_text target, which checks that the syllables tile the run exactly.
pub fn syllable_at(cats: &[u8], i: usize) -> (usize, u16) {
    Cats(cats).syllable(i)
}

/// The category of a code point (for the fuzz target).
pub fn category(cp: u32) -> u8 {
    props(cp).0
}

/// Shape a run of Devanagari glyphs in place: run.b[..run.n] hold the face 2 nominal glyphs, one per character,
/// with `cl` set and `mask` = M_ALL | M_CJCT; `cps[i]` is each glyph's character. Afterwards the run holds the
/// shaped glyphs in logical order, ready for kerning and marks.
pub fn shape(run: &mut gsub::Run, cps: &[u32]) {
    let n0 = run.n.min(cps.len());
    run.n = n0;
    let virama = vector::lookup_deva(0x094D).unwrap_or(0);
    // 1. categories and positions
    for i in 0..n0 {
        let (mut c, mut p) = props(cps[i]);
        if (c == C || c == RA) && p == P_BASE_C {
            p = consonant_position(run.t, run.b[i].g, virama);
        }
        if c == X && cps[i] == 0x20 {
            c = X;
            p = P_END;
        }
        set_cat(&mut run.b[i], c);
        set_pos(&mut run.b[i], p);
    }
    // 2. syllables (index << 4 | type), and a dotted circle before each broken cluster
    let mut cats = [X; gsub::MAXG];
    for i in 0..n0 {
        cats[i] = cat(&run.b[i]);
    }
    let dotted = vector::lookup_deva(0x25CC);
    let mut i = 0;
    let mut serial = 0u16;
    let mut out = 0usize; // glyphs after this syllable, in run.b (moving as dotted circles go in)
    while i < n0 {
        let (e, ty) = Cats(&cats[..n0]).syllable(i);
        let e = e.min(n0).max(i + 1);
        let len = e - i;
        let mut ty = ty;
        if ty == S_BROKEN {
            if let Some(dc) = dotted {
                if run.n < run.b.len().min(gsub::MAXG) {
                    // put the dotted circle at the syllable's start, in the run (out is where this syllable starts)
                    let at = out;
                    run.b.copy_within(at..run.n, at + 1);
                    run.n += 1;
                    let mut d = run.b[at + 1];
                    d.g = dc;
                    d.aux = 0;
                    set_cat(&mut d, DOTTEDCIRCLE);
                    set_pos(&mut d, P_BASE_C);
                    run.b[at] = d;
                    ty = S_STANDALONE;
                    for k in at..at + len + 1 {
                        run.b[k].syl = serial << 4 | ty;
                    }
                    out += len + 1;
                    i = e;
                    serial = serial.wrapping_add(1);
                    continue;
                }
            }
        }
        for k in out..out + len {
            run.b[k].syl = serial << 4 | ty;
        }
        out += len;
        i = e;
        serial = serial.wrapping_add(1);
    }
    // 3. the per-syllable stages (locl, ccmp), then the initial reordering
    let nstages = dv::STAGES.len();
    let mut si = 0;
    while si < nstages && dv::STAGE_PER_SYLLABLE.get(si).copied().unwrap_or(false) {
        let mut s = 0;
        while s < run.n {
            let e = syllable_end(run, s);
            let ne = run.stage(si, s, e);
            s = ne.max(s + 1);
        }
        si += 1;
    }
    let mut s = 0;
    while s < run.n {
        let e = syllable_end(run, s);
        let ty = run.b[s].syl & 0xF;
        if matches!(ty, S_CONSONANT | S_VOWEL | S_STANDALONE | S_BROKEN) {
            initial_reordering(run, s, e, virama);
        }
        s = e;
    }
    // the basic features, each its own stage, over the whole run (the masks keep each to its glyphs)
    let pres = first_presentation_stage();
    while si < pres.min(nstages) {
        let n = run.n;
        run.stage(si, 0, n);
        si += 1;
    }
    // 4. the final reordering, then the presentation features
    let mut s = 0;
    while s < run.n {
        let e = syllable_end(run, s);
        let ty = run.b[s].syl & 0xF;
        if matches!(ty, S_CONSONANT | S_VOWEL | S_STANDALONE | S_BROKEN) {
            final_reordering(run, s, e, virama);
        }
        s = e;
    }
    while si < nstages {
        let n = run.n;
        run.stage(si, 0, n);
        si += 1;
    }
}

fn syllable_end(run: &gsub::Run, s: usize) -> usize {
    let id = run.b[s].syl;
    let mut e = s + 1;
    while e < run.n && run.b[e].syl == id {
        e += 1;
    }
    e
}

/// The first stage after the basic features (init, pres, abvs, blws, psts, haln and the horizontal features).
fn first_presentation_stage() -> usize {
    let tags: [&[u8; 4]; 10] = [b"init", b"pres", b"abvs", b"blws", b"psts", b"haln", b"calt", b"clig", b"liga", b"rclt"];
    dv::STAGE_FEAT.iter().filter(|r| tags.contains(&&r.1)).map(|r| r.0 as usize).min().unwrap_or(dv::STAGES.len())
}

fn initial_reordering(run: &mut gsub::Run, start: usize, end: usize, virama: u16) {
    let t = run.t;
    let b = &mut *run.b;
    // reph: Ra and halant at the start, when the font would make them a reph
    let mut base = end;
    let mut has_reph = false;
    let mut limit = start;
    if start + 3 <= end && !is_joiner(&b[start + 2]) && reph_forms(t, b[start].g, b[start + 1].g) {
        limit += 2;
        while limit < end && is_joiner(&b[limit]) {
            limit += 1;
        }
        base = start;
        has_reph = true;
    }
    // the base consonant: from the end, the last consonant without a below-base or post-base form
    {
        let mut i = end;
        let mut seen_below = false;
        while i > limit {
            i -= 1;
            if is_consonant(&b[i]) {
                if pos(&b[i]) != P_BELOW_C && (pos(&b[i]) != P_POST_C || seen_below) {
                    base = i;
                    break;
                }
                if pos(&b[i]) == P_BELOW_C {
                    seen_below = true;
                }
                base = i;
            } else if start < i && cat(&b[i]) == ZWJ && cat(&b[i - 1]) == H {
                break;
            }
        }
    }
    if has_reph && base == start && limit - base <= 2 {
        has_reph = false;
    }
    // positions
    for i in start..base.min(end) {
        let p = pos(&b[i]).min(P_PRE_C);
        set_pos(&mut b[i], p);
    }
    if base < end {
        set_pos(&mut b[base], P_BASE_C);
    }
    // a consonant after a matra is a final consonant
    for i in base + 1..end {
        if cat(&b[i]) == M {
            for j in i + 1..end {
                if is_consonant(&b[j]) {
                    set_pos(&mut b[j], P_FINAL_C);
                    break;
                }
            }
            break;
        }
    }
    if has_reph {
        set_pos(&mut b[start], P_RA_TO_BECOME_REPH);
    }
    // joiners, nukta and halant take the position of what is before them
    {
        let mut last_pos = P_START;
        for i in start..end {
            if [ZWJ, ZWNJ, N, H].contains(&cat(&b[i])) {
                set_pos(&mut b[i], last_pos);
                if cat(&b[i]) == H && pos(&b[i]) == P_PRE_M {
                    let mut j = i;
                    while j > start {
                        if pos(&b[j - 1]) != P_PRE_M {
                            let p = pos(&b[j - 1]);
                            set_pos(&mut b[i], p);
                            break;
                        }
                        j -= 1;
                    }
                }
            } else if pos(&b[i]) != P_SMVD {
                last_pos = pos(&b[i]);
            }
        }
    }
    // a post-base consonant owns what is before it since the last consonant or matra
    {
        let mut last = base;
        for i in base + 1..end {
            if is_consonant(&b[i]) {
                for j in last + 1..i {
                    if pos(&b[j]) < P_SMVD {
                        let p = pos(&b[i]);
                        set_pos(&mut b[j], p);
                    }
                }
                last = i;
            } else if cat(&b[i]) == M {
                last = i;
            }
        }
    }
    // sort by position (stable)
    for i in start + 1..end {
        let mut j = i;
        while j > start && pos(&b[j - 1]) > pos(&b[j]) {
            b.swap(j - 1, j);
            j -= 1;
        }
    }
    let mut base = end;
    for i in start..end {
        if pos(&b[i]) == P_BASE_C {
            base = i;
            break;
        }
    }
    // the masks
    let mut i = start;
    while i < end && pos(&b[i]) == P_RA_TO_BECOME_REPH {
        b[i].mask |= dv::M_RPHF;
        i += 1;
    }
    for i in start..base.min(end) {
        b[i].mask |= dv::M_HALF | dv::M_BLWF;
    }
    for i in base.saturating_add(1)..end {
        b[i].mask |= dv::M_BLWF | dv::M_ABVF | dv::M_PSTF;
    }
    // the joiners: ZWJ and ZWNJ after the base turn conjuncts off before them, ZWNJ half forms too
    for i in base.saturating_add(1)..end {
        if is_joiner(&b[i]) {
            let non_joiner = cat(&b[i]) == ZWNJ;
            let mut j = i;
            loop {
                j -= 1;
                b[j].mask &= !dv::M_CJCT;
                if non_joiner {
                    b[j].mask &= !dv::M_HALF;
                }
                if !(j > start && !is_consonant(&b[j])) {
                    break;
                }
            }
        }
    }
    let _ = virama;
}

fn final_reordering(run: &mut gsub::Run, start: usize, end: usize, virama: u16) {
    let b = &mut *run.b;
    // a virama the font split off a ligature counts as a halant again
    for i in start..end {
        if b[i].g == virama && ligated(&b[i]) && multiplied(&b[i]) {
            set_cat(&mut b[i], H);
            b[i].aux &= !(gsub::LIGATED | gsub::MULTIPLIED);
        }
    }
    // the base again
    let mut base = start;
    while base < end && pos(&b[base]) < P_BASE_C {
        base += 1;
    }
    if base == end && start < base && is_one_of(&b[base - 1], &[ZWJ]) {
        base -= 1;
    }
    if base < end {
        while start < base && is_one_of(&b[base], &[N, H]) {
            base -= 1;
        }
    }
    // the pre-base matra: after the last halant before the base, if one is left
    if start + 1 < end && start < base {
        let mut new_pos = if base == end { base.saturating_sub(2) } else { base - 1 };
        loop {
            while new_pos > start && !is_one_of(&b[new_pos], &[M, H]) {
                new_pos -= 1;
            }
            if is_halant(&b[new_pos]) && pos(&b[new_pos]) != P_PRE_M {
                if new_pos + 1 < end && cat(&b[new_pos + 1]) == ZWJ && new_pos > start {
                    new_pos -= 1;
                    continue;
                }
            } else {
                new_pos = start;
            }
            break;
        }
        if start < new_pos && pos(&b[new_pos]) != P_PRE_M {
            let mut i = new_pos;
            while i > start {
                if pos(&b[i - 1]) == P_PRE_M {
                    let old = i - 1;
                    if old < base && base <= new_pos {
                        base -= 1;
                    }
                    let tmp = b[old];
                    b.copy_within(old + 1..new_pos + 1, old);
                    b[new_pos] = tmp;
                    new_pos -= 1;
                }
                i -= 1;
            }
        }
    }
    // reph: from the start to its place (Devanagari: before post-base, by steps 2, 5 and 6 of the spec)
    if start + 1 < end && pos(&b[start]) == P_RA_TO_BECOME_REPH && (ligated(&b[start]) && !multiplied(&b[start])) {
        let mut new_reph = start + 1;
        while new_reph < base && !is_halant(&b[new_reph]) {
            new_reph += 1;
        }
        let found = new_reph < base && is_halant(&b[new_reph]);
        if found {
            if new_reph + 1 < base && is_joiner(&b[new_reph + 1]) {
                new_reph += 1;
            }
        } else {
            new_reph = end - 1;
            while new_reph > start && pos(&b[new_reph]) == P_SMVD {
                new_reph -= 1;
            }
            if is_halant(&b[new_reph]) {
                for i in base + 1..new_reph {
                    if is_one_of(&b[i], &[M]) {
                        new_reph -= 1;
                    }
                }
            }
        }
        let reph = b[start];
        b.copy_within(start + 1..new_reph + 1, start);
        b[new_reph] = reph;
    }
}
