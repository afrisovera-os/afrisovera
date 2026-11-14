//! Smooth text that scales (docs/design/afriui.md section 26): the desk's outline font, our own rasterizer and its
//! glyph cache. The outlines are compiled in (outline_data.rs, made offline by tools/face/outline_src.py); nothing
//! here parses a font file, so no font a disk or an app gives can reach the kernel.
//!
//! The rasterizer is integer fixed point (1/256 pixel), so a glyph gives the same coverage bytes on ARM and PC:
//! each contour is cut into short lines, every line adds its signed area to the pixel cells it crosses (one row at
//! a time), and a running sum along the row turns the cells into 256 levels of coverage. Vertically the outline is
//! bent a little so the baseline, the x-height and the cap height land on whole pixels, which keeps horizontal
//! strokes in one pixel row at small sizes; horizontally it keeps its smoothing and is placed to a quarter pixel.
//!
//! Sizes are asked for by cap height in pixels (the bitmap face's 9 rows at UI text size 1), clamped to
//! MIN_CAP..=MAX_CAP, which is 6 to 63 pixels an em. Mainline only, like all of the desk's drawing.
#![allow(dead_code)]

#[path = "outline_data.rs"]
mod data;

/// Face 1, the Arabic face (SHAPING_PLAN.md C1): its outlines are in the shaper's generated tables.
use super::shape_data as f1;

/// The faces (SHAPING_PLAN.md C1, the architect's Q2): face 0 is AfriSovera Sans exactly as section 26 made it; face
/// 1 is the Arabic face, drawn at face 0's pixels per em with only the baseline on a whole pixel (it has no x-height
/// to bend). A glyph is (face, index); `glyph` and `advance` are face 0's, as before.
pub const LATIN: u8 = 0;
pub const ARABIC: u8 = 1;

type Rows = &'static [(u16, u32, u16, u16, u8)];

fn tables(face: u8) -> (Rows, &'static [(u16, i16, i16)], &'static [(i16, i16)], &'static [u8]) {
    if face == ARABIC {
        (&f1::GLYPHS, &f1::COMPS, &f1::PTS, &f1::KIND)
    } else {
        (&data::GLYPHS, &data::COMPS, &data::PTS, &data::KIND)
    }
}

/// Cap heights in pixels: the one place a size enters, clamped here (the architect's condition: a fixed range, and
/// the cell buffers below are sized for the largest).
pub const MIN_CAP: i32 = 4;
pub const MAX_CAP: i32 = 45;

/// Fixed point: 256 to a pixel.
const ONE: i32 = 256;

/// The largest glyph box at MAX_CAP (an em is about 63 pixels; marks above capitals and descenders stay well inside).
const MAXW: usize = 128;
const MAXH: usize = 128;

/// The lines one glyph is cut into, in 1/256 pixel, y down from the baseline, x from the pen's whole pixel.
const MAX_SEGS: usize = 2048;
static mut SEGS: [(i32, i32, i32, i32); MAX_SEGS] = [(0, 0, 0, 0); MAX_SEGS];
static mut NSEG: usize = 0;
/// More lines than MAX_SEGS were asked for (never with the shipped table; counted, the rest dropped).
static mut SEG_OVER: u32 = 0;

/// One row of cells: (cover, area), FreeType's names. One more than the widest box, for a closing edge on its right.
static mut COVER: [i32; MAXW + 1] = [0; MAXW + 1];
static mut AREA: [i32; MAXW + 1] = [0; MAXW + 1];

/// The glyph cache: coverage bytes in one static arena (no heap), and a table of where each glyph is.
const ARENA: usize = 256 * 1024;
static mut ARENA_BUF: [u8; ARENA] = [0; ARENA];
static mut USED: usize = 0;
const SLOTS: usize = 1024;
#[derive(Clone, Copy)]
struct Slot {
    key: u32, // 0 = empty
    off: u32,
    w: u16,
    h: u16,
    left: i16,
    top: i16,
}
const EMPTY: Slot = Slot { key: 0, off: 0, w: 0, h: 0, left: 0, top: 0 };
static mut SLOT: [Slot; SLOTS] = [EMPTY; SLOTS];
static mut NSLOT: usize = 0;

/// Counters for `desk font probe`: glyphs rasterized, cache hits, cache resets.
pub static mut STATS: (u64, u64, u64) = (0, 0, 0);

pub fn clamp_cap(cap: i32) -> i32 {
    cap.clamp(MIN_CAP, MAX_CAP)
}

/// The glyph for a code point: a bounded binary search of the sorted table (never an index from the code point).
pub fn lookup(cp: u32) -> Option<u16> {
    // printable ASCII is the table's first 95 rows, in order: one checked read instead of a search
    if (0x20..0x7F).contains(&cp) {
        if let Some(&(c, g)) = data::CMAP.get((cp - 0x20) as usize) {
            if c == cp {
                return Some(g);
            }
        }
    }
    data::CMAP.binary_search_by_key(&cp, |&(c, _)| c).ok().map(|i| data::CMAP[i].1)
}

/// The glyph to draw a code point with at a cap height: the small-size form (a longer hook) up to SMALL_CAP, else the
/// font's own. A bounded scan of a short table.
pub fn lookup_at(cp: u32, cap: i32) -> Option<u16> {
    if clamp_cap(cap) <= data::SMALL_CAP {
        if let Some(&(_, g)) = data::SMALL.iter().find(|&&(c, _)| c == cp) {
            return Some(g);
        }
    }
    lookup(cp)
}

/// A glyph's advance at a cap height, in 1/256 pixel.
pub fn advance(g: u16, cap: i32) -> i32 {
    let adv = data::GLYPHS.get(g as usize).map_or(0, |r| r.0 as i64);
    (adv * clamp_cap(cap) as i64 * ONE as i64 / data::CAP as i64) as i32
}

/// The Arabic face's glyph for a code point (a bounded binary search of its sorted table).
pub fn lookup_arabic(cp: u32) -> Option<u16> {
    f1::CMAP.binary_search_by_key(&cp, |&(c, _)| c).ok().and_then(|i| f1::CMAP.get(i)).map(|&(_, g)| g)
}

/// Face 1's font units to face 0's, so both faces share one size per em.
fn to_f0(u: i64) -> i64 {
    u * data::UPEM as i64 / f1::UPEM.max(1) as i64
}

/// A glyph's advance in face 0's font units (1000 an em), for the shaper, which keeps positions in font units.
pub fn units(face: u8, g: u16) -> i32 {
    if face == ARABIC {
        to_f0(f1::GLYPHS.get(g as usize).map_or(0, |r| r.0 as i64)) as i32
    } else {
        data::GLYPHS.get(g as usize).map_or(0, |r| r.0 as i32)
    }
}

/// Face 0 font units to 1/256 pixel at a cap height (the scale `advance` uses).
pub fn px256(u: i32, cap: i32) -> i32 {
    (u as i64 * clamp_cap(cap) as i64 * ONE as i64 / data::CAP as i64) as i32
}

/// Face 0 font units to whole pixels at a cap height, rounded to the nearest.
pub fn px_round(u: i32, cap: i32) -> i32 {
    let v = u as i64 * clamp_cap(cap) as i64;
    let d = data::CAP as i64;
    (if v >= 0 { (v + d / 2) / d } else { -((-v + d / 2) / d) }) as i32
}

/// The missing-character box's advance in face 0 font units at a cap height.
pub fn box_units(cap: i32) -> i32 {
    box_size(cap).1 * data::CAP / clamp_cap(cap)
}

/// A glyph's advance in a face at a cap height, in 1/256 pixel.
pub fn advance_in(face: u8, g: u16, cap: i32) -> i32 {
    if face != ARABIC {
        return advance(g, cap);
    }
    let adv = to_f0(f1::GLYPHS.get(g as usize).map_or(0, |r| r.0 as i64));
    (adv * clamp_cap(cap) as i64 * ONE as i64 / data::CAP as i64) as i32
}

/// The box drawn for a character the table does not have: (width, advance) in pixels at a cap height.
pub fn box_size(cap: i32) -> (i32, i32) {
    let cap = clamp_cap(cap);
    let w = (cap * 5 / 9).max(3);
    (w, w + (cap / 4).max(2))
}

/// The next character of `s` from byte `*i` (UTF-8), moving `*i` past it. A bad or cut-short sequence gives one
/// U+FFFD for its lead byte and the continuation bytes that were valid, and never reads past the string.
pub fn next_char(s: &[u8], i: &mut usize) -> u32 {
    let b0 = match s.get(*i) {
        Some(&b) => b,
        None => return 0xFFFD,
    };
    *i += 1;
    let (n, min, mut cp) = match b0 {
        0x00..=0x7F => return b0 as u32,
        0xC2..=0xDF => (1, 0x80, (b0 & 0x1F) as u32),
        0xE0..=0xEF => (2, 0x800, (b0 & 0x0F) as u32),
        0xF0..=0xF4 => (3, 0x10000, (b0 & 0x07) as u32),
        _ => return 0xFFFD,
    };
    for k in 0..n {
        match s.get(*i) {
            Some(&b) if b & 0xC0 == 0x80 => {
                // the second byte's range rules out overlongs, surrogates and code points past U+10FFFF early
                if k == 0 {
                    let ok = match b0 {
                        0xE0 => b >= 0xA0,
                        0xED => b < 0xA0,
                        0xF0 => b >= 0x90,
                        0xF4 => b < 0x90,
                        _ => true,
                    };
                    if !ok {
                        return 0xFFFD;
                    }
                }
                cp = (cp << 6) | (b & 0x3F) as u32;
                *i += 1;
            }
            _ => return 0xFFFD,
        }
    }
    if cp < min {
        0xFFFD
    } else {
        cp
    }
}

/// Font units to 1/256 pixel, y up, bent so that the x-height and the cap height land on whole pixels.
struct Scale {
    cap: i64,
    xh: i64,
    arabic: bool,
}

impl Scale {
    fn new(cap: i32) -> Scale {
        let cap = clamp_cap(cap) as i64;
        let (fc, fx) = (data::CAP as i64, data::XH as i64);
        let xh = ((fx * cap * 2 + fc) / (2 * fc)).clamp(1, cap - 1);
        Scale { cap, xh, arabic: false }
    }
    fn x(&self, u: i32) -> i32 {
        if self.arabic {
            return (to_f0(u as i64) * self.cap * ONE as i64 / data::CAP as i64) as i32;
        }
        (u as i64 * self.cap * ONE as i64 / data::CAP as i64) as i32
    }
    /// y up in font units to y DOWN in 1/256 pixel from the baseline.
    fn y(&self, u: i32) -> i32 {
        if self.arabic {
            return -((to_f0(u as i64) * self.cap * ONE as i64 / data::CAP as i64) as i32);
        }
        let (fc, fx, u) = (data::CAP as i64, data::XH as i64, u as i64);
        let one = ONE as i64;
        let v = if u <= 0 {
            u * self.cap * one / fc
        } else if u <= fx {
            u * self.xh * one / fx
        } else if u <= fc {
            self.xh * one + (u - fx) * (self.cap - self.xh) * one / (fc - fx)
        } else {
            self.cap * one + (u - fc) * self.cap * one / fc
        };
        -(v as i32)
    }
}

fn push_line(a: (i32, i32), b: (i32, i32)) {
    if a.1 == b.1 {
        return; // a level line adds no area
    }
    // SAFETY: mainline only.
    unsafe {
        if NSEG < MAX_SEGS {
            (&mut *core::ptr::addr_of_mut!(SEGS))[NSEG] = (a.0, a.1, b.0, b.1);
            NSEG += 1;
        } else {
            SEG_OVER += 1;
        }
    }
}

fn isqrt(v: i64) -> i64 {
    if v <= 0 {
        return 0;
    }
    let mut r = 1i64 << ((64 - v.leading_zeros()) / 2 + 1);
    loop {
        let n = (r + v / r) / 2;
        if n >= r {
            return r;
        }
        r = n;
    }
}

/// A quadratic curve as short lines: enough that no line is more than about 1/8 pixel off the curve.
fn push_quad(a: (i32, i32), c: (i32, i32), b: (i32, i32)) {
    let dx = (a.0 as i64 - 2 * c.0 as i64 + b.0 as i64).abs();
    let dy = (a.1 as i64 - 2 * c.1 as i64 + b.1 as i64).abs();
    // the curve is at most dev/4 from its chord; split in n, the error falls by n squared
    let n = (1 + isqrt(dx.max(dy) / 128)).clamp(1, 16);
    let mut prev = a;
    for k in 1..=n {
        let (t, u, nn) = (k, n - k, n * n);
        let p = |a: i32, c: i32, b: i32| ((u * u * a as i64 + 2 * t * u * c as i64 + t * t * b as i64) / nn) as i32;
        let q = (p(a.0, c.0, b.0), p(a.1, c.1, b.1));
        push_line(prev, q);
        prev = q;
    }
}

fn mid(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    ((a.0 + b.0) / 2, (a.1 + b.1) / 2)
}

/// One glyph with points (no components), moved by (dx, dy) font units, as lines.
fn outline(face: u8, g: usize, dx: i32, dy: i32, sc: &Scale, sub: i32) {
    let (rows, _, all_pts, all_kind) = tables(face);
    let (_, p0, np, _, _) = match rows.get(g) {
        Some(&r) => r,
        None => return,
    };
    let (p0, np) = (p0 as usize, np as usize);
    let (pts, kind) = match (all_pts.get(p0..p0 + np), all_kind.get(p0..p0 + np)) {
        (Some(p), Some(k)) => (p, k),
        _ => return,
    };
    let at = |i: usize| (sc.x(pts[i].0 as i32 + dx) + sub, sc.y(pts[i].1 as i32 + dy));
    let mut s = 0;
    while s < np {
        let mut e = s + 1;
        while e < np && kind[e] & 4 == 0 {
            e += 1;
        }
        // the contour is points s..e; TrueType's rules: two control points in a row have an on-curve point
        // halfway between them, and a contour may start on a control point
        let n = e - s;
        let on = |j: usize| kind[s + j] & 1 == 1;
        let first = (0..n).find(|&j| on(j));
        let (start, base) = match first {
            Some(j) => (at(s + j), j),
            None => (mid(at(s + n - 1), at(s)), n - 1),
        };
        let mut cur = start;
        let mut ctrl: Option<(i32, i32)> = None;
        for j in 1..=n {
            let k = (base + j) % n;
            let p = at(s + k);
            if on(k) {
                match ctrl {
                    Some(c) => push_quad(cur, c, p),
                    None => push_line(cur, p),
                }
                cur = p;
                ctrl = None;
            } else {
                if let Some(c) = ctrl {
                    let m = mid(c, p);
                    push_quad(cur, c, m);
                    cur = m;
                }
                ctrl = Some(p);
            }
        }
        match ctrl {
            Some(c) => push_quad(cur, c, start),
            None => push_line(cur, start),
        }
        s = e;
    }
}

/// Add the piece (xa, ya)-(xb, yb) of one row, y from 0 to 256 within the row, to the cells it crosses.
fn row_piece(xa: i32, ya: i32, xb: i32, yb: i32, cover: &mut [i32], area: &mut [i32]) {
    let mut cell = |col: i32, fx0: i32, y0: i32, fx1: i32, y1: i32| {
        if let (Some(c), Some(a)) = (cover.get_mut(col.max(0) as usize), area.get_mut(col.max(0) as usize)) {
            if col >= 0 {
                let d = y1 - y0;
                *c += d;
                *a += d * (fx0 + fx1);
            }
        }
    };
    if xa == xb {
        let col = xa.div_euclid(ONE);
        let f = xa - col * ONE;
        cell(col, f, ya, f, yb);
        return;
    }
    let dir = if xb > xa { 1 } else { -1 };
    let mut col = if dir > 0 { xa.div_euclid(ONE) } else { (xa + ONE - 1).div_euclid(ONE) - 1 };
    let (mut x, mut y) = (xa, ya);
    for _ in 0..(MAXW as i32 + 2) {
        let bx = if dir > 0 { (col + 1) * ONE } else { col * ONE };
        if (dir > 0 && xb <= bx) || (dir < 0 && xb >= bx) {
            cell(col, x - col * ONE, y, xb - col * ONE, yb);
            return;
        }
        let yi = ya + ((yb - ya) as i64 * (bx - xa) as i64 / (xb - xa) as i64) as i32;
        cell(col, x - col * ONE, y, bx - col * ONE, yi);
        x = bx;
        y = yi;
        col += dir;
    }
}

/// Rasterize the lines in SEGS into `out` (w x h coverage bytes), the box's top-left at pixel (bx, by).
fn raster(bx: i32, by: i32, w: usize, h: usize, out: &mut [u8]) {
    // SAFETY: mainline only; nothing else holds these buffers while a glyph is made.
    let (segs, cover, area) = unsafe {
        (
            &(&*core::ptr::addr_of!(SEGS))[..NSEG],
            &mut *core::ptr::addr_of_mut!(COVER),
            &mut *core::ptr::addr_of_mut!(AREA),
        )
    };
    for r in 0..h {
        cover[..w + 1].fill(0);
        area[..w + 1].fill(0);
        let top = (by + r as i32) * ONE;
        let bot = top + ONE;
        for &(x0, y0, x1, y1) in segs {
            let (lo, hi) = (y0.min(y1), y0.max(y1));
            if hi <= top || lo >= bot {
                continue;
            }
            // the part of the line inside this row, x where it enters and leaves
            let xat = |y: i32| x0 + ((x1 - x0) as i64 * (y - y0) as i64 / (y1 - y0) as i64) as i32;
            let (ya, yb) = (y0.clamp(top, bot), y1.clamp(top, bot));
            let (xa, xb) = (if ya == y0 { x0 } else { xat(ya) }, if yb == y1 { x1 } else { xat(yb) });
            row_piece(xa - bx * ONE, ya - top, xb - bx * ONE, yb - top, &mut cover[..w + 1], &mut area[..w + 1]);
        }
        let row = &mut out[r * w..(r + 1) * w];
        let mut acc = 0i32;
        for (x, o) in row.iter_mut().enumerate() {
            acc += cover[x];
            let v = ((acc * 2 * ONE - area[x]).abs() >> 9).min(255);
            *o = v as u8;
        }
    }
}

/// Cut glyph `g` into lines at a cap height and a quarter-pixel offset, components included. Returns the box in
/// pixels, relative to the pen's whole pixel and the baseline: (left, top, w, h); w or h 0 for an empty glyph.
fn build(face: u8, g: u16, cap: i32, sub: i32) -> (i32, i32, usize, usize) {
    // SAFETY: mainline only.
    unsafe { NSEG = 0 };
    let mut sc = Scale::new(cap);
    sc.arabic = face == ARABIC;
    let sub = sub * (ONE / 4);
    let (rows, comps, _, _) = tables(face);
    match rows.get(g as usize) {
        Some(&(_, _, _, c0, nc)) if nc > 0 => {
            for &(cg, dx, dy) in comps.get(c0 as usize..c0 as usize + nc as usize).unwrap_or(&[]) {
                outline(face, cg as usize, dx as i32, dy as i32, &sc, sub);
            }
        }
        Some(_) => outline(face, g as usize, 0, 0, &sc, sub),
        None => {}
    }
    // SAFETY: mainline only.
    let segs = unsafe { &(&*core::ptr::addr_of!(SEGS))[..NSEG] };
    if segs.is_empty() {
        return (0, 0, 0, 0);
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for &(a, b, c, d) in segs {
        x0 = x0.min(a.min(c));
        x1 = x1.max(a.max(c));
        y0 = y0.min(b.min(d));
        y1 = y1.max(b.max(d));
    }
    let (l, t) = (x0.div_euclid(ONE), y0.div_euclid(ONE));
    let (r, b) = ((x1 + ONE - 1).div_euclid(ONE), (y1 + ONE - 1).div_euclid(ONE));
    let w = ((r - l).max(0) as usize).min(MAXW);
    let h = ((b - t).max(0) as usize).min(MAXH);
    (l, t, w, h)
}

/// The cache key. Face 0's keys are what they were before faces (so its cache behaves byte for byte the same);
/// face 1 sets bit 15, which no face 0 key uses (the cap is at most 45, in bits 2 to 7).
fn key(face: u8, g: u16, cap: i32, sub: i32) -> u32 {
    let k = ((g as u32 + 1) << 16) | ((clamp_cap(cap) as u32) << 2) | (sub as u32 & 3);
    if face == ARABIC {
        k | 1 << 15
    } else {
        k
    }
}

/// Empty the cache. Nothing drawn from it is held across this: `glyph` hands its bytes to the caller and the
/// caller is done with them before the next `glyph`.
pub fn reset() {
    // SAFETY: mainline only.
    unsafe {
        (&mut *core::ptr::addr_of_mut!(SLOT)).fill(EMPTY);
        NSLOT = 0;
        USED = 0;
        STATS.2 += 1;
    }
}

/// Glyph `g` at a cap height and quarter-pixel offset `sub` (0 to 3), from the cache or made now, given to `f` as
/// (left, top, w, h, coverage): the box relative to the pen's whole pixel and the baseline, and w x h bytes.
pub fn glyph(g: u16, cap: i32, sub: i32, f: &mut dyn FnMut(i32, i32, usize, usize, &[u8])) {
    glyph_in(LATIN, g, cap, sub, f)
}

/// `glyph` in a face.
pub fn glyph_in(face: u8, g: u16, cap: i32, sub: i32, f: &mut dyn FnMut(i32, i32, usize, usize, &[u8])) {
    // the buffers below are mainline only: no interrupt draws text (the PC screen tick and the PS/2 and xHCI handlers
    // only post keys and copy frames). If that ever changes, stop here rather than mix two glyphs' lines.
    if BUSY.swap(true, core::sync::atomic::Ordering::Acquire) {
        panic!("vector::glyph entered twice (text drawn from an interrupt?)");
    }
    glyph_inner(face, g, cap, sub, f);
    BUSY.store(false, core::sync::atomic::Ordering::Release);
}

/// `glyph` is running (the guard above).
static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn glyph_inner(face: u8, g: u16, cap: i32, sub: i32, f: &mut dyn FnMut(i32, i32, usize, usize, &[u8])) {
    let cap = clamp_cap(cap);
    let sub = sub & 3;
    let k = key(face, g, cap, sub);
    let mut i = (k.wrapping_mul(2654435761) >> 22) as usize % SLOTS;
    // SAFETY: mainline only; the table and the arena are touched only here and in reset.
    unsafe {
        let slots = &mut *core::ptr::addr_of_mut!(SLOT);
        for _ in 0..SLOTS {
            let s = slots[i];
            if s.key == k {
                STATS.1 += 1;
                let (o, n) = (s.off as usize, s.w as usize * s.h as usize);
                f(s.left as i32, s.top as i32, s.w as usize, s.h as usize, &(&*core::ptr::addr_of!(ARENA_BUF))[o..o + n]);
                return;
            }
            if s.key == 0 {
                break;
            }
            i = (i + 1) % SLOTS;
        }
    }
    let (l, t, w, h) = build(face, g, cap, sub);
    // SAFETY: mainline only.
    unsafe {
        STATS.0 += 1;
        // room for the bytes and a free slot (a table at most three quarters full); else start again, before
        // anything of this glyph is written
        if USED + w * h > ARENA || NSLOT * 4 >= SLOTS * 3 {
            reset();
        }
        let o = USED;
        let arena = &mut *core::ptr::addr_of_mut!(ARENA_BUF);
        raster(l, t, w, h, &mut arena[o..o + w * h]);
        USED += w * h;
        let slots = &mut *core::ptr::addr_of_mut!(SLOT);
        let mut i = (k.wrapping_mul(2654435761) >> 22) as usize % SLOTS;
        while slots[i].key != 0 {
            i = (i + 1) % SLOTS;
        }
        slots[i] = Slot { key: k, off: o as u32, w: w as u16, h: h as u16, left: l as i16, top: t as i16 };
        NSLOT += 1;
        f(l, t, w, h, &arena[o..o + w * h]);
    }
}

/// The gamma table for a text colour: light text on the dark desk gets slightly heavier edges, dark text on a
/// light chip slightly lighter ones.
pub fn gamma_for(rgb: u32) -> &'static [u8; 256] {
    let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
    if r * 299 + g * 587 + b * 114 >= 128 * 1000 {
        &data::GAMMA_LIGHT
    } else {
        &data::GAMMA_DARK
    }
}

/// (cells used, slots used, glyphs made, hits, resets, lines dropped) for `desk font probe`.
pub fn stats() -> (usize, usize, u64, u64, u64, u32) {
    // SAFETY: scalar reads, mainline only.
    unsafe { (USED, NSLOT, STATS.0, STATS.1, STATS.2, SEG_OVER) }
}
