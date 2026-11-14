//! The Sovereign UI's paint (docs/design/afriui.md section 14): the theme (palette,
//! cloths, cut-corner primitives, text, icons) and every drawing function of the PC desktop.
//! It reads the model in desk.rs (`use super::*`: the window state, geometry and app data) and
//! never changes it, except for its own frame count and timing; the model never draws. When the
//! phone moves over, it keeps the model and gets its paint here (afriui.md section 9).

use super::*;

#[path = "face.rs"]
mod face; // the Sovereign face (afriui.md section 16)

#[path = "look.rs"]
pub(super) mod look; // the premium look, step 2: corners, soft shadows, see-through panels (afriui.md section 30)

#[path = "vector.rs"]
pub(super) mod vector; // smooth text that scales (afriui.md section 26)

#[path = "shape_data.rs"]
pub(super) mod shape_data; // the shaper's generated tables (SHAPING_PLAN.md C1)

#[path = "bidi.rs"]
pub(super) mod bidi; // right to left and mixed lines (UAX 9)

#[path = "gsub.rs"]
pub(super) mod gsub; // the font's substitution and mark lookups, compiled offline

#[path = "shape.rs"]
pub(super) mod shape; // Arabic shaped by the font's lookups, laid out by bidi

/// Which face the desk's proportional text uses: the outline font (true, the default) or the Sovereign bitmap face
/// (`desk font bitmap`, kept as the fallback until premium step 5). The Terminal and cell windows keep the bitmap
/// face either way until step 3.
pub(super) static VECTOR: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);

fn vector_on() -> bool {
    VECTOR.load(core::sync::atomic::Ordering::Relaxed)
}

const NIGHT: u32 = 0x10_0C_0A; // the Spine: warm black, not grey

const WALL_TOP: u32 = 0x1B_15_12; // soil gradient

const WALL_BOT: u32 = 0x0A_08_07;

const MUD: u32 = 0x24_1C_17; // mud-cloth marks, one shade above the soil

const SURFACE: u32 = 0x1E_18_14;

const SURFACE2: u32 = 0x2A_21_1B; // selected rows, lit tiles

const TITLE_ON: u32 = 0x2A_21_1B; // the lintel (title bar) with focus

const TITLE_OFF: u32 = 0x1C_16_13;

const SURFACE_OFF: u32 = 0x16_12_10; // unfocused window bodies sink back (review D1)

const LINE: u32 = 0x4A_3B_2F; // lifted from 3A2E25 (review S1: window edges must separate)

const TEXT: u32 = 0xF2_EA_DC; // shea

const DIM: u32 = 0xB0_A3_91;

const GOLD: u32 = 0xF2_C1_4E;

const GOLD_DEEP: u32 = 0xC8_96_3A;

const GOLD_DARK: u32 = 0x5E_47_21;

const GOLD_PALE: u32 = 0xFF_EB_B8;

const RED: u32 = 0xC1_12_1F;

const GREEN: u32 = 0x2E_9E_6A; // malachite

const OCHRE: u32 = 0xD0_8A_2E;

const INDIGO: u32 = 0x34_50_8F; // adire

const CHARCOAL: u32 = 0x26_1F_1A;

const SHADOW: u32 = 0x05_04_03;

const TERM_BG: u32 = 0x0E_0B_09;

const TERM_TEXT: u32 = 0xD8_CE_BE;

/// The system cloth (the Spine's woven edge): the 8 Nov kente band, kept.
const KENTE: [u32; 6] = [GOLD_DEEP, NIGHT, GREEN, GOLD_DEEP, RED, NIGHT];

/// Each app's cloth (section 4): its focus mark, woven along the top of its lintel.
const CLOTH: [[u32; 3]; 5] = [[OCHRE, NIGHT, GOLD], [GREEN, NIGHT, GOLD], [INDIGO, TEXT, GOLD], [GOLD, RED, GREEN], [CHARCOAL, GOLD, CHARCOAL]];

/// While an app draws its body, everything it draws is clipped to its window, and the
/// window's bottom-right cut corner stays cut: (x, y, w, h, cut). Mainline only.
static mut CLIP: Option<(i32, i32, i32, i32, i32)> = None;

/// The damaged box being redrawn (afriui.md section 15): every fill is clipped to it.
/// The whole screen when a full frame is drawn. Mainline only.
static mut DCLIP: Rect = (0, 0, i32::MAX / 2, i32::MAX / 2);

fn rect(x: i32, y: i32, w: i32, h: i32, c: u32) {
    // SAFETY: DCLIP is set and reset around a partial frame, in mainline only.
    let (dx, dy, dw, dh) = unsafe { DCLIP };
    let (x0c, y0c) = (x.max(dx), y.max(dy));
    let (w, h) = ((x + w).min(dx + dw) - x0c, (y + h).min(dy + dh) - y0c);
    if w <= 0 || h <= 0 {
        return;
    }
    let (x, y) = (x0c, y0c);
    let (mut x0, mut y0) = (x.max(0), y.max(0));
    let (mut x1, mut y1) = ((x + w).min(W()), (y + h).min(H()));
    // SAFETY: CLIP is set and cleared around an app's body drawing, in mainline only.
    if let Some((cx, cy, cw, ch, k)) = unsafe { CLIP } {
        x0 = x0.max(cx);
        y0 = y0.max(cy);
        x1 = x1.min(cx + cw);
        y1 = y1.min(cy + ch);
        let zone = cy + ch - k; // the rows the bottom-right cut runs through
        if k > 0 && y1 > zone && x1 > cx + cw - k && x1 > x0 {
            if y0 < zone {
                crate::canvas::fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (zone - y0) as usize, c);
            }
            for yy in y0.max(zone)..y1 {
                let r = (cx + cw - (yy - zone)).min(x1); // row j of the cut is inset by j + 1
                if r > x0 {
                    crate::canvas::fill_rect(x0 as usize, yy as usize, (r - x0) as usize, 1, c);
                }
            }
            return;
        }
    }
    if x1 > x0 && y1 > y0 {
        crate::canvas::fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, c);
    }
}

/// How far row j of a shape h rows high is inset on the left and on the right, for corners
/// cut at 45 degrees: tl, tr, br, bl pixels (afriui.md section 3).
fn cut_inset(j: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32) -> (i32, i32) {
    look::insets(j, h, tl, tr, br, bl)
}

/// A filled shape with its corners in the current style (afriui.md sections 3 and 27): cut at 45 degrees (hard, as
/// before, or smooth), or round.
fn cut(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    look::shape(x, y, w, h, tl, tr, br, bl, c, 255);
}

/// The same shape at alpha a over what is there (the see-through panels).
fn cut_a(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32, a: u8) {
    look::shape(x, y, w, h, tl, tr, br, bl, c, a);
}

/// The one-pixel outline of a shape. The hard style is drawn exactly as before step 2.
fn cut_frame(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    if look::corners() != look::Corners::Hard {
        look::frame(x, y, w, h, tl, tr, br, bl, c);
        return;
    }
    for j in 0..h {
        let (l, r) = cut_inset(j, h, tl, tr, br, bl);
        if j == 0 || j == h - 1 {
            rect(x + l, y + j, w - l - r, 1, c);
        } else {
            rect(x + l, y + j, 1, 1, c);
            rect(x + w - r - 1, y + j, 1, 1, c);
        }
    }
}

/// A shape's shadow: soft (step 2) or the hard relief copy 4 pixels down and right (before). None in touch mode.
fn shadow(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, hard: i32, reach: i32) {
    if st().touch {
        return;
    }
    if look::soft() {
        look::shadow(x, y, w, h, tl, tr, br, bl, reach);
    } else {
        cut(x + hard, y + hard, w, h, tl, tr, br, bl, SHADOW);
    }
}

/// A tile, button or chip: all four corners cut, k pixels.
fn chip(x: i32, y: i32, w: i32, h: i32, k: i32, c: u32) {
    cut(x, y, w, h, k, k, k, k, c);
}

/// A woven band (kente rhythm): `rows` rows of 2 px, blocks of 8 px in the cloth's colours,
/// each row offset by half a block, as strips of cloth are. Row j of the band is kept inside
/// [x + inset(j).0, x + w - inset(j).1) so it follows a cut corner.
fn weave(x: i32, y: i32, w: i32, rows: i32, cloth: &[u32], inset: &dyn Fn(i32) -> (i32, i32)) {
    for row in 0..rows {
        for j in 0..2 {
            let yy = row * 2 + j;
            let (l, r) = inset(yy);
            let (lo, hi) = (x + l, x + w - r);
            let mut bx = x - (row % 2) * 4;
            let mut k = row as usize * 3;
            while bx < hi {
                let (a, b) = (bx.max(lo), (bx + 8).min(hi));
                if b > a {
                    rect(a, y + yy, b - a, 1, cloth[k % cloth.len()]);
                }
                bx += 8;
                k += 1;
            }
        }
    }
}

/// The same weave standing up (the Spine's edge): columns of 2 px, blocks 8 px tall.
fn weave_v(x: i32, y: i32, h: i32, cols: i32, cloth: &[u32]) {
    for col in 0..cols {
        let mut by = y - (col % 2) * 4;
        let mut k = col as usize * 3;
        while by < y + h {
            let (a, b) = (by.max(y), (by + 8).min(y + h));
            rect(x + col * 2, a, 2, b - a, cloth[k % cloth.len()]);
            by += 8;
            k += 1;
        }
    }
}

fn lerp(a: u32, b: u32, t: i32) -> u32 {
    let t = t.clamp(0, 100);
    let ch = |sh: u32| -> u32 {
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        (av + (bv - av) * t / 100) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// One glyph of the Sovereign face (face.rs) at (x, y), `scale` pixels a dot: each row's runs
/// of ink are one fill each (fewer calls than a fill per dot).
fn glyph_at(x: i32, y: i32, rows: &[u8; 12], w: i32, c: u32, scale: i32) {
    for (r, &bits) in rows.iter().enumerate() {
        if bits == 0 {
            continue;
        }
        let mut col = 0;
        while col < w {
            if (bits >> (7 - col)) & 1 == 1 {
                let start = col;
                while col < w && (bits >> (7 - col)) & 1 == 1 {
                    col += 1;
                }
                rect(x + start * scale, y + r as i32 * scale, (col - start) * scale, scale, c);
            } else {
                col += 1;
            }
        }
    }
}

/// Proportional text (afriui.md sections 16 and 26). (x0, y0) is the top of the capitals, which are face::CAP rows
/// at `scale` 1; descenders hang below the baseline (face::LINE_H rows in all; marks above capitals rise a little
/// above y0 in the outline face). Returns the end x. In the outline face `s` is UTF-8; in the bitmap face a byte
/// outside ASCII draws as the face's box.
fn text(x0: i32, y0: i32, s: &[u8], c: u32, scale: i32) -> i32 {
    // a line wholly above or below the box being redrawn: only its width is needed
    // SAFETY: mainline only.
    let (_, dy, _, dh) = unsafe { DCLIP };
    let rise = if vector_on() { 4 * scale } else { 0 };
    if !s.is_empty() && (y0 - rise >= dy + dh || y0 + face::LINE_H * scale + rise <= dy) {
        return x0 + text_w(s, scale) + if vector_on() { 0 } else { scale };
    }
    if vector_on() {
        return vtext(x0, y0, s, c, face::CAP * scale);
    }
    let mut x = x0;
    for &b in s {
        let (w, rows) = face::glyph(b);
        glyph_at(x, y0, rows, w, c, scale);
        x += (w + 1) * scale;
    }
    x
}

fn text_w(s: &[u8], scale: i32) -> i32 {
    if vector_on() {
        return vtext_w(s, face::CAP * scale);
    }
    let w: i32 = s.iter().map(|&b| face::glyph(b).0 + 1).sum();
    ((w - 1) * scale).max(0)
}

/// The width of UTF-8 text in the outline face at a cap height, in whole pixels (rounded up).
pub(super) fn vtext_w(s: &[u8], cap: i32) -> i32 {
    if shape::needed(s) {
        // the same decision as vtext, so a measured width always matches the drawn one
        return (shape::layout(s, cap, None, &mut |_, _, _| {}) + 255) >> 8;
    }
    let mut pen = 0;
    let mut i = 0;
    while i < s.len() {
        let cp = vector::next_char(s, &mut i);
        pen += match vector::lookup_at(cp, cap) {
            Some(g) => vector::advance(g, cap),
            None => vector::box_size(cap).1 * 256,
        };
    }
    (pen + 255) >> 8
}

/// UTF-8 text in the outline face, (x0, y0) the top of the capitals, `cap` the cap height in pixels. Each glyph is
/// placed to a quarter pixel; a character the font does not have draws as a box. Returns the end x.
pub(super) fn vtext(x0: i32, y0: i32, s: &[u8], c: u32, cap: i32) -> i32 {
    let cap = vector::clamp_cap(cap);
    if shape::needed(s) {
        return vshaped(x0, y0, s, c, cap);
    }
    let base = y0 + cap;
    let gamma = vector::gamma_for(c);
    if let Some((l, t, w, h, off, adv)) = run(s, cap) {
        // SAFETY: mainline only; the run's bytes are not touched until the next `run`.
        let buf = unsafe { &(&*core::ptr::addr_of!(RUN_BUF))[off..off + w * h] };
        cover(x0 + l, base + t, w, h, buf, c, gamma);
        return x0 + ((adv + 255) >> 8);
    }
    let mut pen = x0 * 256;
    let mut i = 0;
    while i < s.len() {
        let cp = vector::next_char(s, &mut i);
        match vector::lookup_at(cp, cap) {
            Some(g) => {
                let px = pen >> 8;
                vector::glyph(g, cap, (pen >> 6) & 3, &mut |l, t, w, h, cov| cover(px + l, base + t, w, h, cov, c, gamma));
                pen += vector::advance(g, cap);
            }
            None => {
                let (bw, adv) = vector::box_size(cap);
                let bx = (pen >> 8) + (adv - bw) / 2;
                rect(bx, y0, bw, 1, c);
                rect(bx, base - 1, bw, 1, c);
                rect(bx, y0, 1, cap, c);
                rect(bx + bw - 1, y0, 1, cap, c);
                pen += adv * 256;
            }
        }
    }
    (pen + 255) >> 8
}

/// A line that needs shaping (right to left, Arabic): shaped and ordered by shape::layout as one isolated paragraph,
/// its direction from its own first strong character, so a call's text never reorders the text of another call (the
/// architect's rule for titles: the app's name and the desk's own marks are separate calls). Returns the end x.
fn vshaped(x0: i32, y0: i32, s: &[u8], c: u32, cap: i32) -> i32 {
    let base = y0 + cap;
    let gamma = vector::gamma_for(c);
    let start = x0 * 256;
    let w = shape::layout(s, cap, None, &mut |g, x, y| {
        let p = start + x;
        if g.boxed {
            let (bw, adv) = vector::box_size(cap);
            let bx = (p >> 8) + (adv - bw) / 2;
            rect(bx, y0, bw, 1, c);
            rect(bx, base - 1, bw, 1, c);
            rect(bx, y0, 1, cap, c);
            rect(bx + bw - 1, y0, 1, cap, c);
        } else {
            let px = p >> 8;
            vector::glyph_in(g.face, g.g, cap, (p >> 6) & 3, &mut |l, t, w, h, cov| cover(px + l, base + y + t, w, h, cov, c, gamma));
        }
    });
    (start + w + 255) >> 8
}

// ---- The text run cache (afriui.md section 30; the architect's follow-up 2 of step 1, and his conditions)

/// Runs longer than this are drawn glyph by glyph and never cached.
const RUN_MAX: usize = 120;
const RUN_SLOTS: usize = 128;
const RUN_ARENA: usize = 64 * 1024;
#[derive(Clone, Copy)]
struct Run {
    hash: u32,
    cap: i32,
    len: usize,
    bytes: [u8; RUN_MAX],
    off: u32,
    w: u16,
    h: u16,
    left: i16,
    top: i16,
    adv: i32,
    used: bool,
}
const NO_RUN: Run = Run { hash: 0, cap: 0, len: 0, bytes: [0; RUN_MAX], off: 0, w: 0, h: 0, left: 0, top: 0, adv: 0, used: false };
static mut RUNS: [Run; RUN_SLOTS] = [NO_RUN; RUN_SLOTS];
static mut RUN_BUF: [u8; RUN_ARENA] = [0; RUN_ARENA];
static mut RUN_USED: usize = 0;
static mut RUN_N: usize = 0;
/// (hits, runs made, times emptied), for the probe.
pub(super) static mut RUN_STATS: (u64, u64, u64) = (0, 0, 0);
/// The cache is on (`desk look runs on|off`), and a test switch that makes every key's hash collide (the
/// architect's condition: a hit compares the whole string, the cap height and the start, never the hash alone).
pub(super) static RUN_ON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);
pub(super) static RUN_WEAK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn run_hash(s: &[u8], cap: i32) -> u32 {
    let h = s.iter().fold(0x811c_9dc5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193)) ^ (cap as u32).wrapping_mul(0x9e37_79b9);
    if RUN_WEAK.load(core::sync::atomic::Ordering::Relaxed) {
        h & 1
    } else {
        h
    }
}

/// Empty the run cache (before a new run when it is full, and from the probe). Nothing drawn from it is held.
pub(super) fn run_reset() {
    // SAFETY: mainline only.
    unsafe {
        for r in (&mut *core::ptr::addr_of_mut!(RUNS)).iter_mut() {
            r.used = false;
        }
        RUN_N = 0;
        RUN_USED = 0;
        RUN_STATS.2 += 1;
    }
}

/// The run for `s` at a cap height, from the cache or made now: (left, top, w, h, offset in RUN_BUF, advance in
/// 1/256 pixel), the box relative to the pen's start and the baseline. None: not cached (too long, a character the
/// font lacks, too big, or the cache is off); the caller draws glyph by glyph. The pen always starts on a whole pixel
/// (callers pass whole x), so the quarter pixel of the start is always 0 and is not part of the key.
fn run(s: &[u8], cap: i32) -> Option<(i32, i32, usize, usize, usize, i32)> {
    if !RUN_ON.load(core::sync::atomic::Ordering::Relaxed) || s.is_empty() || s.len() > RUN_MAX {
        return None;
    }
    let hash = run_hash(s, cap);
    let mut i = (hash.wrapping_mul(2654435761) >> 25) as usize % RUN_SLOTS;
    // SAFETY: mainline only; the slots and the arena are touched only here and in run_reset.
    let runs = unsafe { &mut *core::ptr::addr_of_mut!(RUNS) };
    for _ in 0..RUN_SLOTS {
        let r = &runs[i];
        if !r.used {
            break;
        }
        // a hit compares the whole key: the hash first, then the cap height, the length and every byte
        if r.hash == hash && r.cap == cap && r.len == s.len() && &r.bytes[..r.len] == s {
            // SAFETY: as above.
            unsafe { RUN_STATS.0 += 1 };
            return Some((r.left as i32, r.top as i32, r.w as usize, r.h as usize, r.off as usize, r.adv));
        }
        i = (i + 1) % RUN_SLOTS;
    }
    // a miss: first the run's box, then its coverage, glyph by glyph as `vtext` places them
    let (mut x0, mut y0, mut x1, mut y1, mut pen) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN, 0i32);
    let mut k = 0;
    while k < s.len() {
        let g = vector::lookup_at(vector::next_char(s, &mut k), cap)?;
        let px = pen >> 8;
        vector::glyph(g, cap, (pen >> 6) & 3, &mut |l, t, w, h, _| {
            if w > 0 && h > 0 {
                x0 = x0.min(px + l);
                y0 = y0.min(t);
                x1 = x1.max(px + l + w as i32);
                y1 = y1.max(t + h as i32);
            }
        });
        pen += vector::advance(g, cap);
    }
    if x1 <= x0 || y1 <= y0 {
        return None; // spaces only: nothing to cache
    }
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w * h > RUN_ARENA / 4 {
        return None;
    }
    // SAFETY: mainline only.
    unsafe {
        if RUN_USED + w * h > RUN_ARENA || RUN_N * 4 >= RUN_SLOTS * 3 {
            // empty it before any byte of the new run is written (as the glyph cache does)
            for r in runs.iter_mut() {
                r.used = false;
            }
            RUN_N = 0;
            RUN_USED = 0;
            RUN_STATS.2 += 1;
        }
        let off = RUN_USED;
        let buf = &mut (&mut *core::ptr::addr_of_mut!(RUN_BUF))[off..off + w * h];
        buf.fill(0);
        let (mut pen, mut k) = (0i32, 0);
        while k < s.len() {
            let g = vector::lookup_at(vector::next_char(s, &mut k), cap)?;
            let px = pen >> 8;
            vector::glyph(g, cap, (pen >> 6) & 3, &mut |l, t, gw, gh, cov| {
                for r in 0..gh {
                    let row = (t + r as i32 - y0) as usize * w + (px + l - x0) as usize;
                    for c in 0..gw {
                        let d = &mut buf[row + c];
                        *d = d.saturating_add(cov[r * gw + c]);
                    }
                }
            });
            pen += vector::advance(g, cap);
        }
        RUN_USED += w * h;
        RUN_N += 1;
        RUN_STATS.1 += 1;
        // a free slot: the table is at most three quarters full, so the probe ends
        let mut i = (hash.wrapping_mul(2654435761) >> 25) as usize % RUN_SLOTS;
        while runs[i].used {
            i = (i + 1) % RUN_SLOTS;
        }
        let mut bytes = [0u8; RUN_MAX];
        bytes[..s.len()].copy_from_slice(s);
        runs[i] = Run { hash, cap, len: s.len(), bytes, off: off as u32, w: w as u16, h: h as u16, left: x0 as i16, top: y0 as i16, adv: pen, used: true };
        Some((x0, y0, w, h, off, pen))
    }
}

/// Mix a glyph's coverage (w x h bytes) into the screen at (x, y), clipped as `rect` clips: to the box being
/// redrawn, to the screen, and to an app's window with its bottom-right corner cut.
fn cover(x: i32, y: i32, w: usize, h: usize, cov: &[u8], c: u32, gamma: &[u8; 256]) {
    cover_stride(x, y, w, h, cov, w, c, gamma);
}

/// One pixel of colour c at alpha a (1 to 254), clipped as `rect` clips (the box redrawn, the screen, an app's window
/// and its cut corner): the edge pixels of corners and outlines, without `cover`'s slices.
fn mix1(x: i32, y: i32, a: u8, c: u32) {
    // SAFETY: mainline only.
    let (dx, dy, dw, dh) = unsafe { DCLIP };
    if x < dx.max(0) || y < dy.max(0) || x >= (dx + dw).min(W()) || y >= (dy + dh).min(H()) {
        return;
    }
    // SAFETY: mainline only.
    if let Some((cx, cy, cw, ch, k)) = unsafe { CLIP } {
        if x < cx || y < cy || x >= cx + cw || y >= cy + ch {
            return;
        }
        let zone = cy + ch - k;
        if k > 0 && y >= zone && x >= cx + cw - (y - zone) {
            return;
        }
    }
    crate::canvas::mix(x as usize, y as usize, c, a as u32);
}

/// `cover` with the coverage `stride` bytes a row; a stride of 0 reads the same row for every row (a flat alpha).
fn cover_stride(x: i32, y: i32, w: usize, h: usize, cov: &[u8], stride: usize, c: u32, gamma: &[u8; 256]) {
    let (w, h, st) = (w as i32, h as i32, stride as i32);
    // SAFETY: mainline only.
    let (dx, dy, dw, dh) = unsafe { DCLIP };
    let (mut x0, mut y0) = (x.max(dx).max(0), y.max(dy).max(0));
    let (mut x1, mut y1) = ((x + w).min(dx + dw).min(W()), (y + h).min(dy + dh).min(H()));
    let mut cutk = None;
    // SAFETY: mainline only.
    if let Some((cx, cy, cw, ch, k)) = unsafe { CLIP } {
        x0 = x0.max(cx);
        y0 = y0.max(cy);
        x1 = x1.min(cx + cw);
        y1 = y1.min(cy + ch);
        if k > 0 && y1 > cy + ch - k && x1 > cx + cw - k {
            cutk = Some((cx + cw, cy + ch - k));
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let at = |xx: i32, yy: i32| ((yy - y) * st + (xx - x)) as usize;
    // black with no gamma (the soft shadows) takes the darkening path: the same pixels, half the work
    let dark = c == 0 && core::ptr::eq(gamma, &look::IDENT);
    let put = |x: usize, y: usize, w: usize, h: usize, a: &[u8]| {
        if dark {
            crate::canvas::shade(x, y, w, h, a, stride);
        } else {
            crate::canvas::blend(x, y, w, h, a, stride, c, gamma);
        }
    };
    match cutk {
        None => put(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, &cov[at(x0, y0)..]),
        Some((right, zone)) => {
            // row j of the cut is inset by j + 1, as in `rect`
            for yy in y0..y1 {
                let r = if yy >= zone { x1.min(right - (yy - zone)) } else { x1 };
                if r > x0 {
                    put(x0 as usize, yy as usize, (r - x0) as usize, 1, &cov[at(x0, yy)..]);
                }
            }
        }
    }
}

/// The marks a person relies on are never see-through and never mixed (the architect, section 30): when a
/// see-through panel (the Spine, a toast, the notifications list, the Circle) lies over the cell window's title, its
/// file name, "unsigned" mark and Close anyway are drawn again over the panel, opaque, on the lintel's own ground.
/// Not where a window above the cell window hides them. Within the box being redrawn only.
fn marks_on_top() {
    if !look::glass() {
        return;
    }
    let s = st();
    let c = App::Cell as usize;
    if !s.wins[c].open || s.wins[c].min || cell_owner().is_none() {
        return;
    }
    let (x, y, _, _) = win_rect(c);
    let (_, _, ux, uw, ax, aw, _, _) = lintel_parts();
    let nx = x + 12 + 12 + 8;
    let right = (ux + uw).max(ax + aw);
    let th = title_h();
    let mb: Rect = (nx - 2, y + 6, right - nx + 4, th - 6);
    if mb.2 <= 0 {
        return;
    }
    let over = touches(dock_box(), mb)
        || toast_box().is_some_and(|b| touches(b, mb))
        || (s.notes_open && touches(panel_box(notes_box()), mb))
        || (s.launcher && touches(panel_box(launcher_box()), mb));
    if !over {
        return;
    }
    if let Some(p) = (0..s.nz).position(|k| s.z[k] == c) {
        if (p + 1..s.nz).any(|k| touches(win_rect(s.z[k]), mb)) {
            return;
        }
    }
    // SAFETY: mainline only; restored right after.
    let keep = unsafe { DCLIP };
    let (x0, y0) = (keep.0.max(mb.0), keep.1.max(mb.1));
    let (x1, y1) = ((keep.0 + keep.2).min(mb.0 + mb.2), (keep.1 + keep.3).min(mb.1 + mb.3));
    if x1 > x0 && y1 > y0 {
        // SAFETY: as above.
        unsafe { DCLIP = (x0, y0, x1 - x0, y1 - y0) };
        window_whole(c);
        // SAFETY: as above.
        unsafe { DCLIP = keep };
    }
}

/// Where the cell window's last drawn title put its parts: (window x, width, mark x, mark width, Close anyway x and
/// width, buttons x, name cut), for `desk cell titlecheck`. Mainline only.
static mut LINTEL: (i32, i32, i32, i32, i32, i32, i32, bool) = (0, 0, 0, 0, 0, 0, 0, false);

pub(super) fn lintel_parts() -> (i32, i32, i32, i32, i32, i32, i32, bool) {
    // SAFETY: mainline only.
    unsafe { LINTEL }
}

/// The narrowest lintel that holds an app's title whole: the icon, a name cut to "..", the unsigned mark, Close
/// anyway and the title buttons, with their gaps (afriui.md section 26, the architect's follow-up 1).
pub(super) fn lintel_min_w() -> i32 {
    lintel_min_w_in(lang::current())
}

/// The narrowest lintel in a given language: its words for the unsigned mark and Close anyway are measured, as the
/// title drawing draws them (the architect's condition 3, 13 Nov), so no language's mark is ever cut.
pub(super) fn lintel_min_w_in(l: lang::Lang) -> i32 {
    let touch = st().touch;
    let ts = if touch { 2 } else { 1 };
    let isz = if touch { 16 } else { 12 };
    let bw = if touch { 44 } else { 22 };
    let uw = text_w(lang::text_in(l, lang::Msg::TitleUnsigned), ts) + 10 * ts;
    let dots = text_w(b"..", ts) + ts;
    let buttons = if touch { bw + 4 } else { 2 * bw + 8 };
    12 + isz + 8 + dots + 8 * ts + uw + 6 * ts + anyway_w(l, ts) + 8 * ts + buttons
}

/// The width of the red Close anyway button: its word with 2 px a side at scale `ts`, and never narrower than the
/// 84 px (x ts) it has always had, so the English title is the same as before.
pub(super) fn anyway_w(l: lang::Lang, ts: i32) -> i32 {
    (text_w(lang::text_in(l, lang::Msg::TitleAnyway), ts) + 4 * ts).max(84 * ts)
}

/// The longest start of `s` (whole characters) that fits in `room` pixels with ".." after it; (s, false) when all
/// of `s` fits, and an empty start when not even ".." would.
fn fit(s: &[u8], room: i32, ts: i32) -> (&[u8], bool) {
    if text_w(s, ts) <= room {
        return (s, false);
    }
    let dots = text_w(b"..", ts) + ts;
    let (mut i, mut best) = (0, 0);
    while i < s.len() {
        let mut j = i;
        if vector_on() {
            vector::next_char(s, &mut j);
        } else {
            j += 1;
        }
        if text_w(&s[..j], ts) + dots > room {
            break;
        }
        best = j;
        i = j;
    }
    (&s[..best], true)
}

/// Monospace text (the Terminal, the file preview), clipped to `cols` cells of 6 px: every
/// glyph of the face is at most 5 wide, centred in its cell, so the columns stay aligned.
fn mono(x: i32, y: i32, s: &[u8], c: u32, cols: usize) {
    // SAFETY: mainline only.
    let (_, dy, _, dh) = unsafe { DCLIP };
    if y >= dy + dh || y + face::LINE_H <= dy {
        return; // a line outside the box being redrawn
    }
    for (i, &b) in s.iter().take(cols).enumerate() {
        let (w, rows) = face::glyph(b);
        glyph_at(x + i as i32 * 6 + (5 - w) / 2, y, rows, w, c, 1);
    }
}

fn monument(cx: i32, top: i32, h: i32, c_top: u32, c_bot: u32) {
    for t in 0..h {
        let f = t * 100 / h.max(1);
        let outer = (6 + (128 * f * f) / 10000 + (30 * f) / 100) * h / 288;
        let outer = outer.max(1);
        let seam = (10 * (100 - f) / 100) * h / 288;
        let c = lerp(c_top, c_bot, f);
        let y = top + t;
        if cx - seam > cx - outer {
            rect(cx - outer, y, outer - seam, 1, c);
        }
        if cx + outer > cx + seam {
            rect(cx + seam, y, outer - seam, 1, c);
        }
    }
    let pw = (300 * h / 288).max(6);
    let ph = (6 * h / 288).max(1);
    rect(cx - pw / 2, top + h, pw, ph, c_top);
}

/// Andika's tile (section 32): the editor's indigo cloth, three short lines of cells and a gold cursor block. Dim: the
/// same drawing in the dark tones, while the shell is busy.
fn andika_icon(x: i32, y: i32, s: i32, dim: bool) {
    let u = |v: i32| v * s / 28;
    let r = (s / 7).max(2);
    let (ground, line, cursor) = if dim { (0x22_2A_40, 0x4A_50_60, 0x6A_5A_34) } else { (INDIGO, 0xB8_C0_E0, GOLD) };
    chip(x, y, s, s, r, ground);
    let t = u(2).max(1);
    for (k, w) in [(0, 16), (1, 11), (2, 7)] {
        rect(x + u(5), y + u(8) + k * u(5), u(w), t, line);
    }
    let ch = u(4).max(3);
    rect(x + u(14), y + u(18) + t - ch, u(3).max(2), ch, cursor); // the cursor after the last line
}

fn icon(app: Option<App>, x: i32, y: i32, s: i32) {
    let u = |v: i32| v * s / 28; // design units on a 28 px grid
    let r = (s / 7).max(2); // the tile's cut corners
    match app {
        None => {
            // the Circle button: the monument on night, in a gold-dark cut frame
            chip(x, y, s, s, r, NIGHT);
            cut_frame(x, y, s, s, r, r, r, r, GOLD_DARK);
            monument(x + s / 2, y + u(4), u(17), GOLD_PALE, GOLD_DEEP);
        }
        Some(App::Files) => {
            chip(x, y, s, s, r, 0x33_22_12);
            rect(x + u(4), y + u(7), u(9), u(3), OCHRE); // tab
            cut(x + u(4), y + u(9), u(20), u(13), 0, (s / 14).max(1), (s / 14).max(1), 0, OCHRE);
            rect(x + u(4), y + u(12), u(20), u(1).max(1), 0xE8_A8_52); // fold highlight
        }
        Some(App::Terminal) => {
            chip(x, y, s, s, r, CHARCOAL);
            rect(x + u(3), y + u(4), u(22), u(1).max(1), LINE);
            // ">" then "_"
            let t = u(2).max(1);
            for k in 0..u(5).max(2) {
                rect(x + u(6) + k, y + u(10) + k, t, t, GOLD);
                rect(x + u(6) + k, y + u(10) + 2 * u(5).max(2) - k - 1, t, t, GOLD);
            }
            rect(x + u(14), y + u(19), u(8), t, GOLD);
        }
        Some(App::Settings) if s < 20 => {
            // at title-bar size: two bold sliders (three thin ones turn to mush, review D1)
            chip(x, y, s, s, r, INDIGO);
            rect(x + 2, y + s / 3, s - 4, 1, 0xB8_C0_E0);
            rect(x + 2, y + 2 * s / 3, s - 4, 1, 0xB8_C0_E0);
            rect(x + s / 4, y + s / 3 - 2, 3, 5, GOLD);
            rect(x + s - s / 4 - 3, y + 2 * s / 3 - 2, 3, 5, GOLD);
        }
        Some(App::Settings) => {
            chip(x, y, s, s, r, INDIGO);
            for (k, at) in [(0, 9), (1, 16), (2, 6)] {
                let yy = y + u(7) + k * u(7);
                rect(x + u(5), yy, u(18), u(1).max(1), 0xB8_C0_E0);
                rect(x + u(5) + u(at), yy - u(2), u(3).max(2), u(5).max(3), GOLD);
            }
        }
        Some(App::Cell) => {
            // an app's cell window: a charcoal tile with three rows of cells, the middle one gold
            chip(x, y, s, s, r, CHARCOAL);
            for (k, c) in [(0, DIM), (1, GOLD), (2, DIM)] {
                let yy = y + u(8) + k * u(5);
                rect(x + u(5), yy, u(18 - 4 * (k % 2)), u(2).max(1), c);
            }
        }
        Some(App::About) => {
            // the About tile wears the brand's kente cloth along its foot
            chip(x, y, s, s, r, 0x1A_14_10);
            monument(x + s / 2, y + u(4), u(15), GOLD_PALE, GOLD_DEEP);
            if s >= 20 {
                let k = r;
                weave(x, y + s - u(5), s, 1, &CLOTH[3], &|j: i32| {
                    let h = u(5);
                    cut_inset(j + s - h, s, k, k, k, k)
                });
            }
        }
    }
}

fn wallpaper() {
    let sw = spine_w();
    // SAFETY: mainline only.
    let (cx0, cy0, cw, ch) = unsafe { DCLIP };
    let (y0, y1) = (cy0.max(0), (cy0 + ch).min(H()));
    for y in y0..y1 {
        rect(sw, y, W() - sw, 1, lerp(WALL_TOP, WALL_BOT, y * 100 / H()));
    }
    if st().wallpaper {
        // mud cloth (bogolanfini): a grid of small marks, a dot and a chevron in turn, one
        // shade above the soil, so they are felt more than seen (afriui.md section 6);
        // only the marks near the clip (a partial frame) are visited
        let (jy0, ix0) = (((y0 - 12 - 4) / 24).max(0), ((cx0 - sw - 12 - 4) / 24).max(0));
        let (mut y, mut j) = (12 + jy0 * 24, jy0);
        while y < (H() - 8).min(y1 + 4) {
            let (mut x, mut i) = (sw + 12 + ix0 * 24, ix0);
            while x < (W() - 8).min(cx0 + cw + 4) {
                if (i + j) % 2 == 0 {
                    rect(x, y, 2, 2, MUD);
                } else {
                    for k in 0..4 {
                        rect(x - 3 + k, y - 1 + k, 1, 1, MUD);
                        rect(x + 3 - k, y - 1 + k, 1, 1, MUD);
                    }
                }
                x += 24;
                i += 1;
            }
            y += 24;
            j += 1;
        }
        // the monument, faint, standing behind the windows; a warm glow at its foot
        let cx = sw + (W() - sw) / 2;
        let mh = (H() * 55 / 100).max(60);
        let top = (H() - mh) / 2 - 4;
        for k in 0..24 {
            let c = lerp(0x24_18_0E, WALL_BOT, k * 100 / 24);
            rect(cx - 200 + k * 4, top + mh + 6 + k / 3, 400 - k * 8, 1, c);
        }
        monument(cx, top, mh, 0x40_30_17, 0x20_17_0C);
        let word = b"AfriSovera";
        let tw = text_w(word, 2);
        text(cx - tw / 2, top + mh + 14, word, 0x33_27_15, 2);
    }
}

/// The Spine: the Circle button, the app tiles with their beads, then at the foot the
/// network, power, the bell and the clock; a woven edge on its right (section 6).
fn spine() {
    let s = st();
    if lying() {
        return bar();
    }
    let body = spine_w() - WEAVE;
    // the Spine stays opaque: it is the desk's edge, the wallpaper is not drawn under it, so a see-through Spine
    // would mix again over itself on every partial frame (found by desk verify on the first build)
    rect(0, 0, body, H(), NIGHT);
    weave_v(body, 0, H(), 2, &KENTE);
    // the Circle button
    let (tx, ty, ts) = dock_tile(0);
    if s.launcher {
        chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, GOLD_DEEP);
    }
    icon(None, tx, ty, ts);
    rect(8, ty + ts + 6, body - 16, 1, LINE);
    // the apps, each with a bead on the left edge: long and gold with focus, short when
    // open, a dot when minimised
    let foc = focused();
    for i in 1..=NTILES {
        let (tx, ty, ts) = dock_tile(i);
        let a = i - 1;
        if foc == Some(a) {
            chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, SURFACE2);
        }
        icon(Some(APPS[a]), tx, ty, ts);
        let w = s.wins[a];
        if w.open {
            let focus = foc == Some(a);
            let bh = if focus { ts - 8 } else if w.min { 4 } else { 10 };
            chip(0, ty + (ts - bh) / 2, 3, bh, 1, if focus { GOLD } else { GOLD_DEEP });
        }
    }

    // the foot, bottom up: clock, bell, power, network
    let mid = body / 2;
    let secs = clock_secs();
    let (hh, mm) = ((secs / 3600) % 24, (secs / 60) % 60);
    let mut l = Line::new();
    let mut under = Line::new();
    if s.clock24 {
        l.two(hh).s(b":").two(mm);
        under.s(lang::t(lang::Msg::ClockUtc));
    } else {
        let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
        l.u(h12).s(b":").two(mm);
        under.s(lang::t(if hh < 12 { lang::Msg::ClockAm } else { lang::Msg::ClockPm }));
    }
    rect(8, H() - 30, body - 16, 1, LINE);
    text(mid - text_w(l.get(), 1) / 2, H() - 23, l.get(), GOLD, 1);
    text(mid - text_w(under.get(), 1) / 2, H() - 12, under.get(), DIM, 1);

    // bell, with the count of unread notices
    let (_, by, _, _) = bell_box();
    let gx = mid - 13;
    let gy = by - 3;
    let bc = if s.notes_open { GOLD } else { TEXT };
    rect(gx + 9, gy + 6, 8, 7, bc);
    rect(gx + 8, gy + 12, 10, 2, bc);
    rect(gx + 11, gy + 15, 4, 2, bc);
    if s.unread > 0 {
        chip(gx + 17, gy + 2, 10, 11, 2, RED);
        let mut t = [0u8; 20];
        text(gx + 20, gy + 3, num(s.unread as u64, &mut t), TEXT, 1);
    }
    power_net(mid - 11, H() - 78, mid - 10, H() - 100);
    // the power button: a ring broken at the top with a bar through the gap
    let (_, py, _, ph) = power_btn_box();
    let (cx, cy) = (mid, py + ph / 2);
    let c = if s.power > 0 { GOLD } else { DIM };
    cut_frame(cx - 7, cy - 6, 14, 14, 4, 4, 4, 4, c);
    rect(cx - 2, cy - 7, 4, 3, NIGHT); // the ring's gap
    rect(cx - 1, cy - 9, 2, 8, c); // the bar
}

/// The power panel: the menu (Shut down, Restart) or its confirm, beside the Spine's foot.
fn power_panel() {
    let s = st();
    let (x, y, w, h) = power_box();
    shadow(x, y, w, h, CUT, 0, CUT, 0, 4, look::REACH); // soft, as every panel (opaque itself)
    cut(x, y, w, h, CUT, 0, CUT, 0, 0x16_11_0E);
    cut_frame(x, y, w, h, CUT, 0, CUT, 0, GOLD_DARK);
    let (b1, b2) = power_buttons();
    let button = |b: (i32, i32, i32, i32), label: &[u8], c: u32, tc: u32| {
        chip(b.0, b.1, b.2, b.3, 3, c);
        text(b.0 + (b.2 - text_w(label, 1)) / 2, b.1 + (b.3 - face::CAP) / 2, label, tc, 1);
    };
    match s.power {
        1 => {
            text(x + 14, y + 10, lang::t(lang::Msg::PowerTitle), GOLD, 1);
            button(b1, lang::t(lang::Msg::PowerOff), SURFACE2, TEXT);
            button(b2, lang::t(lang::Msg::PowerRestart), SURFACE2, TEXT);
        }
        _ => {
            let off = s.power == 2;
            text(x + 14, y + 10, lang::t(if off { lang::Msg::PowerOffAsk } else { lang::Msg::PowerRestartAsk }), GOLD, 1);
            text(x + 14, y + 28, lang::t(lang::Msg::PowerUnsaved), DIM, 1);
            button(b1, lang::t(if off { lang::Msg::PowerOff } else { lang::Msg::PowerRestart }), RED, TEXT);
            button(b2, lang::t(lang::Msg::PowerCancel), SURFACE2, TEXT);
        }
    }
}

/// The Spine lying down as a bar along the bottom (a phone held upright, afriui-phone-plan.md
/// P1): the woven edge on top, the Circle button and the apps in a row with their beads along
/// the bottom edge, then network, power, the bell and the clock at the right end.
fn bar() {
    let s = st();
    let top = H() - bar_h();
    rect(0, top, W(), bar_h(), NIGHT);
    weave(0, top, W(), 2, &KENTE, &|_| (0, 0));
    let (tx, ty, ts) = dock_tile(0);
    if s.launcher {
        chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, GOLD_DEEP);
    }
    icon(None, tx, ty, ts);
    let (t1, _, _) = dock_tile(1);
    rect((tx + ts + t1) / 2, ty + 6, 1, ts - 12, LINE);
    let foc = focused();
    for i in 1..=NTILES {
        let (tx, ty, ts) = dock_tile(i);
        let a = i - 1;
        if foc == Some(a) {
            chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, SURFACE2);
        }
        icon(Some(APPS[a]), tx, ty, ts);
        let w = s.wins[a];
        if w.open {
            let focus = foc == Some(a);
            let bw = if focus { ts - 8 } else if w.min { 4 } else { 10 };
            chip(tx + (ts - bw) / 2, H() - 4, bw, 3, 1, if focus { GOLD } else { GOLD_DEEP });
        }
    }
    // the clock at the right end, hours and minutes over UTC (or am/pm)
    let secs = clock_secs();
    let (hh, mm) = ((secs / 3600) % 24, (secs / 60) % 60);
    let (mut l, mut under) = (Line::new(), Line::new());
    if s.clock24 {
        l.two(hh).s(b":").two(mm);
        under.s(lang::t(lang::Msg::ClockUtc));
    } else {
        let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
        l.u(h12).s(b":").two(mm);
        under.s(lang::t(if hh < 12 { lang::Msg::ClockAm } else { lang::Msg::ClockPm }));
    }
    let (cx, cy, cw, _) = clock_box();
    let mid = cx + cw / 2;
    rect(cx, cy + WEAVE + 8, 1, 36, LINE);
    text(mid - text_w(l.get(), 1) / 2, cy + WEAVE + 10, l.get(), GOLD, 1);
    text(mid - text_w(under.get(), 1) / 2, cy + WEAVE + 23, under.get(), DIM, 1);
    // the bell with its count, left of the clock
    let (bx, by, bw, bh) = bell_box();
    let (gx, gy) = (bx + bw / 2 - 13, by + bh / 2 - 11);
    let bc = if s.notes_open { GOLD } else { TEXT };
    rect(gx + 9, gy + 6, 8, 7, bc);
    rect(gx + 8, gy + 12, 10, 2, bc);
    rect(gx + 11, gy + 15, 4, 2, bc);
    if s.unread > 0 {
        chip(gx + 17, gy + 2, 10, 11, 2, RED);
        let mut t = [0u8; 20];
        text(gx + 20, gy + 3, num(s.unread as u64, &mut t), TEXT, 1);
    }
    let mid_y = top + WEAVE + (bar_h() - WEAVE) / 2;
    power_net(W() - 150, mid_y - 6, W() - 190, mid_y - 6);
}

/// The power glyph at (px, py) and the network glyph at (nx, ny), on the Spine or the bar.
fn power_net(px: i32, py: i32, nx: i32, ny: i32) {
    // power: no battery driver yet, so a plug and "AC" (no fake levels)
    rect(px, py + 3, 7, 6, TEXT);
    rect(px + 1, py, 1, 3, TEXT);
    rect(px + 5, py, 1, 3, TEXT);
    rect(px + 3, py + 9, 1, 3, TEXT);
    text(px + 10, py + 2, lang::t(lang::Msg::PowerAc), DIM, 1);
    // network: a link glyph, gold with a lease, dim without, crossed without a device
    let (present, ip) = (crate::virtio::net_present(), crate::virtio::dhcp_ip());
    let c = if !present {
        LINE
    } else if ip != [0u8; 4] {
        GOLD
    } else {
        DIM
    };
    rect(nx, ny + 1, 8, 6, c);
    rect(nx + 12, ny + 7, 8, 6, c);
    rect(nx + 3, ny + 7, 2, 4, c);
    rect(nx + 3, ny + 10, 12, 2, c);
    rect(nx + 15, ny + 4, 2, 4, c);
    if !present {
        for k in 0..14 {
            rect(nx + 3 + k, ny + k, 2, 1, RED);
        }
    }
}

fn window(a: usize) {
    // motion (afriui.md section 17): while it unfolds, the window is clipped to its lintel and
    // a third of its body per frame; what lies below (wallpaper, lower windows) is already drawn
    if let Some(rows) = unfold_rows(a) {
        let (x, y, w, _) = win_rect(a);
        // SAFETY: mainline only; restored right after.
        let keep = unsafe { DCLIP };
        let (x0, y0) = (keep.0.max(x), keep.1.max(y));
        let (x1, y1) = ((keep.0 + keep.2).min(x + w + 4), (keep.1 + keep.3).min(y + rows));
        if x1 > x0 && y1 > y0 {
            // SAFETY: as above.
            unsafe { DCLIP = (x0, y0, x1 - x0, y1 - y0) };
            window_whole(a);
        }
        // SAFETY: as above.
        unsafe { DCLIP = keep };
        return;
    }
    window_whole(a);
}

fn window_whole(a: usize) {
    let s = st();
    let (x, y, w, h) = win_rect(a);
    let th = title_h();
    let focus = focused() == Some(a);
    let k = win_cut();
    let ins = |j: i32| cut_inset(j, h, k, 0, k, 0);
    // the shadow (soft, or the hard relief of a carving), then the window over it
    // the focused window casts the full shadow; the others a short one (afriui.md section 30: they sink back, and
    // their shadows were most of the ARM frame's cost)
    shadow(x, y, w, h, k, 0, k, 0, 4, if focus { look::REACH } else { 3 });
    cut(x, y, w, h, k, 0, k, 0, if focus { SURFACE } else { SURFACE_OFF });
    // the lintel (title bar), with its corner's edge pixels in the lintel's colour
    for j in 0..th {
        let (l, r) = ins(j);
        let c = if focus { TITLE_ON } else { TITLE_OFF };
        rect(x + l, y + j, w - l - r, 1, c);
        look::edges(x, y, w, h, j, k, 0, k, 0, c, 255);
    }
    rect(x, y + th, w, 1, LINE);
    let band = 6; // the cloth: three woven rows (4 px was too thin to read focus at a glance, review S1)
    let isz = if s.touch { 16 } else { 12 };
    icon(Some(APPS[a]), x + 12, y + band + (th - band - isz) / 2, isz);
    let ts = if s.touch { 2 } else { 1 };
    // an app's cell window wears its file name, and "unsigned" when it is not bundled or verified: the desk draws
    // both, and the app cannot set or cover them (afriui.md section 25, security-model.md section 4)
    let (name, signed) = match (APPS[a], cell_owner()) {
        (App::Cell, Some(o)) => o,
        _ => (APPS[a].name(), true),
    };
    let ty = y + band + (th - band - face::CAP * ts) / 2;
    // the name gets what is left of the lintel after the "unsigned" mark, the Close anyway button and the title
    // buttons: the mark always shows whole, and a name too long for the rest is cut with ".."
    let nx = x + 12 + isz + 8;
    let uw = if signed { 0 } else { text_w(lang::t(lang::Msg::TitleUnsigned), ts) + 10 * ts };
    let (tcl, tmn) = title_buttons(a);
    let right = if APPS[a] == App::Cell && cell_offer() { anyway_box(a).0 } else if s.touch { tcl.0 } else { tmn.0 };
    let room = right - 6 * ts - nx - if signed { 0 } else { uw + 8 * ts };
    let (shown, cut) = fit(name, room, ts);
    let mut end = text(nx, ty, shown, if focus { TEXT } else { DIM }, ts);
    if cut {
        end = text(end + ts, ty, b"..", DIM, ts);
    }
    let mut ux = 0;
    if !signed {
        ux = if shown.is_empty() && !cut { nx } else { end + 8 * ts };
        chip(ux, ty - 3 * ts, uw, (face::CAP + 6) * ts, 2, OCHRE);
        text(ux + 5 * ts, ty, lang::t(lang::Msg::TitleUnsigned), TERM_BG, ts);
    }
    if APPS[a] == App::Cell {
        let (ax, _, aw, _) = if cell_offer() { anyway_box(a) } else { (right, 0, 0, 0) };
        // SAFETY: mainline only.
        unsafe { LINTEL = (x, w, ux, uw, ax, aw, if s.touch { tcl.0 } else { tmn.0 }, cut) };
    }
    if APPS[a] == App::Cell && cell_offer() {
        let (ax, ay, aw, ah) = anyway_box(a);
        chip(ax, ay, aw, ah, 3, RED);
        let ca = lang::t(lang::Msg::TitleAnyway);
        text(ax + (aw - text_w(ca, ts)) / 2, ay + (ah - face::CAP * ts) / 2, ca, TEXT, ts);
    }
    // buttons: minimise (pointer only) and close, cut-corner chips
    let (cl, mn) = title_buttons(a);
    let hot = s.hover_close == Some(a);
    if hot {
        chip(cl.0, cl.1, cl.2, cl.3, 3, RED);
    }
    let c = if focus || hot { TEXT } else { DIM };
    let (cx, cy) = (cl.0 + cl.2 / 2, cl.1 + cl.3 / 2);
    let arm = if s.touch { 6 } else { 4 };
    for k in -arm..=arm {
        rect(cx + k, cy + k, 2, 1, c);
        rect(cx + k, cy - k, 2, 1, c);
    }
    if !s.touch {
        rect(mn.0 + mn.2 / 2 - 4, mn.1 + mn.3 / 2 + 2, 9, 2, c);
    }
    // body: whatever the app draws stays inside the window and keeps its cut corner
    let (bx, by, bw, bh) = (x + 1, y + th + 1, w - 2, h - th - 2);
    // SAFETY: mainline only; cleared right after the body is drawn.
    unsafe { CLIP = Some((bx, by, bw, bh, (k - 1).max(0))) };
    match APPS[a] {
        App::Files => files_app(a, bx, by, bw, bh),
        App::Terminal => terminal_app(bx, by, bw, bh),
        App::Settings => settings_app(a, bx, by, bw, bh),
        App::About => about_app(bx, by, bw, bh),
        App::Cell => cell_app(bx, by, bw, bh),
    }
    // SAFETY: as above.
    unsafe { CLIP = None };
    // the outline, then the cloth along the top of the focused window's lintel
    cut_frame(x, y, w, h, k, 0, k, 0, if focus { GOLD_DEEP } else { LINE }); // review S1: gold-dark was too muddy
    if focus {
        weave(x, y, w, band / 2, &CLOTH[a], &ins);
    }
}

/// The 16 colour roles of a cell window (afriui.md section 24), in Andika's order: text, dim, keyword, string,
/// comment, number, punct, gutter, gutter-current, selection, status, status-dim, cursor-line, error, ok,
/// accent. As a background, role 0 is the body itself. The contrast review is stage 4.
/// Every text role reads at 4.5:1 or more on the body (TERM_BG), the cursor line (12) and the selection (9): the
/// architect's floor (section 32, 14 Nov), graded by desk-cell from `desk cell roles`. Dim, string, punct, gutter and
/// error were lifted in lightness with their hues kept; the selection stays as it was (the native apps engineer's
/// choice: it is the ground people steer by while editing).
const CELL_ROLES: [u32; 16] = [
    TERM_TEXT,   // 0 text
    0xA3_98_8B,  // 1 dim (was 8A7E6E)
    GOLD,        // 2 keyword
    0x33_AE_75,  // 3 string, malachite lifted (was GREEN 2E9E6A)
    DIM,         // 4 comment
    OCHRE,       // 5 number
    0xDC_81_5D,  // 6 punct, terracotta lifted (was C4562B)
    0xA5_98_89,  // 7 gutter (was 6E6254)
    GOLD_DEEP,   // 8 gutter-current
    0x3A_2E_22,  // 9 selection
    TEXT,        // 10 status
    DIM,         // 11 status-dim
    0x1C_16_12,  // 12 cursor-line
    0xE8_79_6F,  // 13 error (was E04A3C)
    0x4F_C0_8A,  // 14 ok
    0x7A_9C_E0,  // 15 accent
];

/// `desk cell roles`: the colours the contrast grade reads (the body, the 16 roles, and the title's two marks on
/// their own grounds).
pub(super) fn cell_roles_report() {
    crate::kprint!("[desk] cell roles: body {:06X};", TERM_BG);
    for (i, c) in CELL_ROLES.iter().enumerate() {
        crate::kprint!(" {}={:06X}", i, c);
    }
    crate::kprintln!("; unsigned {:06X} on {:06X}, close anyway {:06X} on {:06X}", TERM_BG, OCHRE, TEXT, RED);
}

/// A cell window's body: role 0 behind, then each row's cells (only the rows inside the box being redrawn),
/// then the cursor in gold.
fn cell_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, TERM_BG);
    let (cols, rows) = cell_grid();
    let g = cell_cells();
    let (cc, cr, shape) = cell_cursor_at();
    // SAFETY: mainline only.
    let (_, dy, _, dh) = unsafe { DCLIP };
    for r in 0..rows {
        let (_, top, _, _) = cell_box(0, r);
        if top >= dy + dh || top + CELL_H <= dy {
            continue; // a row outside the box being redrawn
        }
        for c in 0..cols {
            let (cx, _, _, _) = cell_box(c, r);
            let at = (r * CELL_COLS + c) * 2;
            let (ch, attr) = (g[at], g[at + 1]);
            let bg = (attr >> 4) as usize;
            if bg != 0 {
                rect(cx, top, CELL_W, CELL_H, CELL_ROLES[bg]);
            }
            let mut fg = CELL_ROLES[(attr & 15) as usize];
            let here = c == cc && r == cr;
            if here && shape == 2 {
                rect(cx, top, CELL_W, CELL_H, GOLD);
                fg = TERM_BG;
            }
            cell_glyph(cx, top + 1, ch, fg);
            if here && shape == 1 {
                rect(cx, top + 1, 1, face::CAP, GOLD);
            } else if here && shape == 3 {
                rect(cx, top + 1 + face::CAP + 1, CELL_W, 1, GOLD);
            }
        }
    }
}

/// One cell's character: 33 to 126 from the face, centred in the 6 px cell as `mono` does; a space nothing;
/// every other byte (0x80, the letter the font lacks, and the rest) a hollow box the size of a capital.
fn cell_glyph(cx: i32, y: i32, ch: u8, c: u32) {
    match ch {
        b' ' => {}
        33..=126 => {
            let (w, rows) = face::glyph(ch);
            glyph_at(cx + (5 - w) / 2, y, rows, w, c, 1);
        }
        _ => {
            rect(cx, y, 5, 1, c);
            rect(cx, y + face::CAP - 1, 5, 1, c);
            rect(cx, y, 1, face::CAP, c);
            rect(cx + 4, y, 1, face::CAP, c);
        }
    }
}

fn files_app(a: usize, x: i32, y: i32, w: i32, h: i32) {
    let s = st();
    let split = files_split(a);
    rect(x, y, split, 20, SURFACE2);
    text(x + 10, y + 6, lang::t(lang::Msg::FilesName), DIM, 1);
    let sz = lang::t(lang::Msg::FilesSize);
    text(x + split - 10 - text_w(sz, 1).max(34), y + 6, sz, DIM, 1);
    rect(x + split, y, 1, h, LINE);
    let mut row = 0;
    for i in 0..crate::vfs::MAX_FILES {
        if !crate::vfs::is_used(i) {
            continue;
        }
        let rh = file_row_h();
        let ry = y + 22 + row * rh;
        if ry + rh > y + h {
            break;
        }
        let sel = s.files_sel == Some(i);
        if sel {
            rect(x + 2, ry, split - 4, rh - 1, 0x2C_25_14);
            rect(x + 2, ry, 2, rh - 1, GOLD);
        }
        let name = crate::vfs::name_of(i).as_bytes();
        let wasm = name.ends_with(b".wasm");
        // a small page icon, ochre for data, gold for programs
        let cy = ry + (rh - 16) / 2; // the row's content, centred in a touch row
        if st().touch && !sel && row > 0 {
            rect(x + 10, ry, split - 20, 1, LINE); // touch rows are separated, as on a phone list
        }
        rect(x + 10, cy + 3, 7, 9, if wasm { GOLD_DEEP } else { OCHRE });
        rect(x + 14, cy + 3, 3, 3, if sel { 0x2C_25_14 } else { SURFACE });
        text(x + 23, cy + 4, name, if sel { GOLD } else { TEXT }, 1);
        let mut t = [0u8; 20];
        let n = num(crate::vfs::size_of(i) as u64, &mut t);
        text(x + split - 10 - text_w(n, 1), cy + 4, n, DIM, 1);
        row += 1;
    }
    if row == 0 {
        text(x + 10, y + 30, lang::t(lang::Msg::FilesNone), DIM, 1);
    }
    // the preview pane
    let (px, pw) = (x + split + 1, w - split - 1);
    match s.files_sel {
        Some(i) if crate::vfs::is_used(i) => {
            let name = crate::vfs::name_of(i);
            text(px + 10, y + 8, name.as_bytes(), GOLD, 1);
            let mut l = Line::new();
            l.f(lang::Msg::FilesBytes, &[lang::Arg::N(crate::vfs::size_of(i) as u64)]);
            text(px + 10, y + 20, l.get(), DIM, 1);
            rect(px + 10, y + 32, pw - 20, 1, LINE);
            let data = crate::vfs::read_raw(name).unwrap_or(&[]);
            let printable = data.iter().take(512).all(|&c| c == b'\n' || c == b'\r' || c == b'\t' || (0x20..0x7f).contains(&c));
            if name.ends_with(".wasm") {
                text(px + 10, y + 42, lang::t(lang::Msg::FilesWasm), TEXT, 1);
                text(px + 10, y + 56, lang::t(lang::Msg::FilesRunIt), DIM, 1);
                let mut l = Line::new();
                l.s(b"wasm ").s(name.as_bytes());
                text(px + 10, y + 70, l.get(), GOLD_DEEP, 1);
            } else if printable {
                let cols = ((pw - 20) / 6).max(1) as usize;
                let mut yy = y + 42;
                'lines: for line in data.split(|&c| c == b'\n') {
                    // wrapped at the pane's width, at a space when there is one
                    let mut rest = line;
                    loop {
                        if yy + face::LINE_H > y + h {
                            break 'lines;
                        }
                        let mut cut = rest.len().min(cols);
                        if cut < rest.len() {
                            if let Some(sp) = rest[..cut].iter().rposition(|&c| c == b' ') {
                                if sp > 0 {
                                    cut = sp + 1;
                                }
                            }
                        }
                        mono(px + 10, yy, &rest[..cut], TEXT, cols);
                        yy += 13;
                        rest = &rest[cut..];
                        if rest.is_empty() {
                            break;
                        }
                    }
                }
            } else {
                text(px + 10, y + 42, lang::t(lang::Msg::FilesBinary), TEXT, 1);
            }
        }
        _ => {
            let m = lang::t(lang::Msg::FilesSelect);
            text(px + (pw - text_w(m, 1)) / 2, y + h / 2 - 4, m, DIM, 1);
        }
    }
}

fn terminal_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, TERM_BG);
    let cols = ((w - 16) / 6).max(1) as usize;
    let rows = ((h - 12) / 13).max(1) as usize;
    // the newest lines that are not desktop log lines, bottom up, from the view (afriui.md section 28)
    let view = term_view();
    let mut shown = [0usize; 128];
    let rows = rows.min(128);
    let mut n = 0;
    let mut k = view;
    while n < rows && k < TL {
        let l = term_line(k);
        if !l.starts_with(b"[desk]") {
            shown[n] = k;
            n += 1;
        }
        k += 1;
    }
    for j in 0..n {
        let line = term_line(shown[j]);
        let yy = y + 6 + (n - 1 - j) as i32 * 13;
        let prompt = line.starts_with(b"afrisovera>");
        if prompt {
            mono(x + 8, yy, &line[..11], GOLD, cols);
            mono(x + 8 + 12 * 6, yy, &line[11.min(line.len())..], TEXT, cols.saturating_sub(12));
        } else {
            mono(x + 8, yy, line, TERM_TEXT, cols);
        }
        if j == 0 && view == 0 {
            // the cursor after the line being typed
            let cx = x + 8 + (line.len().min(cols) as i32) * 6;
            rect(cx + 1, yy, 5, face::CAP, GOLD);
        }
    }
    if view > 0 {
        // scrolled back: say how far, and how to come back, at the top right
        let mut l = Line::new();
        if view == 1 {
            l.s(lang::t(lang::Msg::TermBackOne));
        } else {
            l.f(lang::Msg::TermBack, &[lang::Arg::N(view as u64)]);
        }
        let tw = text_w(l.get(), 1);
        let (cx, cy) = (x + w - tw - 22, y + 4);
        chip(cx, cy, tw + 14, face::CAP + 8, 3, GOLD_DEEP);
        text(cx + 7, cy + 4, l.get(), NIGHT, 1);
    }
}

fn switch(x: i32, y: i32, on: bool) {
    chip(x, y, 30, 14, 3, if on { GOLD_DEEP } else { LINE });
    chip(if on { x + 17 } else { x + 3 }, y + 2, 10, 10, 2, if on { NIGHT } else { DIM });
}

fn settings_app(a: usize, x: i32, y: i32, w: i32, _h: i32) {
    let s = st();
    let label = |yy: i32, t: &[u8]| {
        text(x + 14, yy, t, TEXT, 1);
    };
    let head = |yy: i32, t: &[u8]| {
        text(x + 14, yy, t, GOLD_DEEP, 1);
        rect(x + 14 + text_w(t, 1) + 6, yy + 4, w - 40 - text_w(t, 1), 1, LINE);
    };
    head(y + 10, lang::t(lang::Msg::SetDisplay));
    let rows = settings_rows();
    let mut l = Line::new();
    l.f(lang::Msg::SetPixels, &[lang::Arg::N(W() as u64), lang::Arg::N(H() as u64)]);
    text(x + 14, y + 24, l.get(), DIM, 1);
    // input profile: a two-part switch
    // the language: its own name on a chip (a click moves to the next), and an ochre "draft" mark while any of its
    // strings is a machine draft not yet checked by a fluent speaker (LANGUAGES_PLAN.md)
    label(y + rows[4] + 5, lang::t(lang::Msg::SetLanguage));
    let lx = x + settings_seg_x(a);
    // one chip with the current language's own name and a down chevron; gold under the pointer or while its list
    // is open; a click or Enter opens the list (the designer's spec, 14 Nov)
    let cur = lang::current();
    let hot = s.lang_hover || s.lang_list;
    chip(lx, y + rows[4], 128, 20, 4, if hot { GOLD_DEEP } else { SURFACE2 });
    let own = cur.own_name().as_bytes();
    let (shown, cut_) = fit(own, 128 - 10 - 7 - 14, 1);
    let end = text(lx + 10, y + rows[4] + 6, shown, if hot { NIGHT } else { TEXT }, 1);
    if cut_ {
        text(end + 1, y + rows[4] + 6, b"..", if hot { NIGHT } else { DIM }, 1);
    }
    let (vx, vy, vc) = (lx + 128 - 10 - 7, y + rows[4] + 8, if hot { NIGHT } else { DIM });
    for k in 0..4 {
        rect(vx + k, vy + k, 7 - 2 * k, 1, vc);
    }
    if lang::counts(cur).1 > 0 {
        let d = lang::t(lang::Msg::SetDraft);
        let dw = text_w(d, 1) + 10;
        chip(lx - dw - 6, y + rows[4] + 3, dw, 14, 2, OCHRE);
        text(lx - dw - 1, y + rows[4] + 6, d, TERM_BG, 1);
    }
    label(y + rows[0] + 5, lang::t(lang::Msg::SetInput));
    let sx = x + settings_seg_x(a);
    chip(sx, y + rows[0], 128, 20, 4, SURFACE2);
    let on = if s.touch { 64 } else { 0 };
    chip(sx + on + 2, y + rows[0] + 2, 60, 16, 3, GOLD_DEEP);
    text(sx + 10, y + rows[0] + 6, lang::t(lang::Msg::SetPointer), if s.touch { DIM } else { NIGHT }, 1);
    text(sx + 64 + 14, y + rows[0] + 6, lang::t(lang::Msg::SetTouch), if s.touch { NIGHT } else { DIM }, 1);

    head(y + 90, lang::t(lang::Msg::SetDesktop));
    label(y + rows[1] + 3, lang::t(lang::Msg::SetWallpaper));
    switch(x + w - 50, y + rows[1], s.wallpaper);
    label(y + rows[2] + 3, lang::t(lang::Msg::SetClock24));
    switch(x + w - 50, y + rows[2], s.clock24);
    label(y + rows[3] + 3, lang::t(lang::Msg::SetMotion));
    switch(x + w - 50, y + rows[3], s.motion);

    head(y + 182, lang::t(lang::Msg::SetNetwork));
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(lang::t(lang::Msg::SetNoDevice));
    } else if ip == [0u8; 4] {
        l.s(lang::t(lang::Msg::SetNoAddress));
    } else {
        rect(x + 14, y + 201, 5, 5, GREEN);
        let mut dotted = Line::new();
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                dotted.s(b".");
            }
            dotted.u(*o as u64);
        }
        l.f(lang::Msg::SetConnected, &[lang::Arg::S(dotted.get())]);
    }
    label(y + 198, l.get());

    head(y + 220, lang::t(lang::Msg::SetSystem));
    let mut l = Line::new();
    l.f(lang::Msg::SetCores, &[lang::Arg::N(crate::cores_found() as u64), lang::Arg::N(crate::ram_mib())]);
    label(y + 236, l.get());
    let mut l = Line::new();
    l.f(lang::Msg::SetBooted, &[lang::Arg::N(crate::config::boots() as u64), lang::Arg::N(now_ticks() / 100)]);
    text(x + 14, y + 250, l.get(), DIM, 1);
    note_uptime(a, y + 250);
    let _ = (w, a);
}

/// The line where Settings or About shows the uptime (it changes every second), as a box on
/// the desktop; the model damages only that line once a second (afriui.md section 15).
/// Each app records where it last drew its uptime line, as rows below its window's top
/// (architect's note N6: the place comes from the drawing, not a copy of its layout); -1 until
/// the app has been drawn, and then the whole window is the line's box.
static mut UPTIME_DY: [i32; NWIN] = [-1; NWIN];

fn note_uptime(a: usize, line_y: i32) {
    // SAFETY: mainline only.
    unsafe { UPTIME_DY[a] = line_y - win_rect(a).1 };
}

pub(super) fn uptime_box(a: usize) -> Rect {
    let (x, y, w, _) = win_rect(a);
    // SAFETY: mainline only.
    match unsafe { UPTIME_DY[a] } {
        dy if dy >= 0 => (x + 1, y + dy - 1, w - 2, face::LINE_H + 2),
        _ => win_box(a),
    }
}

fn about_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, 0x0C_09_07);
    let mh = (h * 38 / 100).max(40);
    monument(x + w / 2, y + 16, mh, GOLD_PALE, 0x9A_67_18);
    let mut yy = y + 16 + mh + 14;
    let t = b"AfriSovera OS";
    text(x + (w - text_w(t, 2)) / 2, yy, t, GOLD, 2);
    yy += 22;
    let sub = lang::t(lang::Msg::AboutMotto);
    text(x + (w - text_w(sub, 1)) / 2, yy, sub, GOLD_DEEP, 1);
    yy += 18;
    rect(x + 24, yy, w - 48, 1, LINE);
    yy += 10;
    let arch = lang::t(if cfg!(target_arch = "aarch64") { lang::Msg::AboutArm } else { lang::Msg::AboutPc });
    let mut rows: [(&[u8], Line); 4] = [
        (lang::t(lang::Msg::AboutKernel), Line::new()),
        (lang::t(lang::Msg::AboutDesktop), Line::new()),
        (lang::t(lang::Msg::AboutDevice), Line::new()),
        (lang::t(lang::Msg::AboutMemory), Line::new()),
    ];
    rows[0].1.s(lang::t(lang::Msg::AboutKernelIs));
    rows[1].1.s(lang::t(lang::Msg::AboutDesktopIs));
    rows[2].1.f(lang::Msg::AboutDeviceIs, &[lang::Arg::S(arch), lang::Arg::N(crate::cores_found() as u64)]);
    // the machine's memory, as Settings shows it (it showed the kernel's 64 MiB page pool here; the owner's HP, 14 Nov)
    rows[3].1.f(lang::Msg::AboutMemoryIs, &[lang::Arg::N(crate::ram_mib()), lang::Arg::N(now_ticks() / 100)]);
    let vx = rows.iter().map(|(k, _)| 30 + text_w(k, 1) + 14).max().unwrap_or(100).max(100);
    for (r, (k, v)) in rows.iter().enumerate() {
        if yy + face::LINE_H > y + h {
            break;
        }
        text(x + 30, yy, k, DIM, 1);
        text(x + vx, yy, v.get(), TEXT, 1);
        if r == 3 {
            note_uptime(App::About as usize, yy); // the uptime is on this row
        }
        yy += 14;
    }
}

/// `desk lang fit` (LANGUAGES_PLAN.md stage 2): every string of language `l` measured against the room its place on
/// the desk gives it, at the windows' home sizes and with the widest numbers it can show. Each string wider than its
/// room goes to `bad(id, width, room)`. The rooms follow the drawing code above; a change there changes them here.
pub(super) fn fit_report(l: lang::Lang, bad: &mut dyn FnMut(&str, i32, i32)) {
    let name = |m: lang::Msg| lang::IDS[m as usize];
    let w_of = |m: lang::Msg| text_w(lang::text_in(l, m), 1);
    let filled = |m: lang::Msg, args: &[lang::Arg]| {
        let mut line = Line::new();
        lang::fill_text(lang::text_in(l, m), args, &mut |b: &[u8]| {
            line.s(b);
        });
        text_w(line.get(), 1)
    };
    let mut check = |id: &str, w: i32, room: i32| {
        if w > room {
            bad(id, w, room);
        }
    };
    // Settings (home width; the chip and the switches as settings_app draws them)
    let sw = HOME[App::Settings as usize].2;
    let seg = sw - 150;
    let dw = w_of(lang::Msg::SetDraft) + 10;
    check(name(lang::Msg::SetDraft), dw, 40);
    check(name(lang::Msg::SetLanguage), w_of(lang::Msg::SetLanguage), seg - dw - 6 - 14 - 6);
    check(name(lang::Msg::SetInput), w_of(lang::Msg::SetInput), seg - 14 - 6);
    for m in [lang::Msg::SetWallpaper, lang::Msg::SetClock24, lang::Msg::SetMotion] {
        check(name(m), w_of(m), sw - 50 - 14 - 6);
    }
    for m in [lang::Msg::SetPointer, lang::Msg::SetTouch] {
        check(name(m), w_of(m), 60 - 12);
    }
    // the language list's rows: every language's own name whole beside the draft chip (200 - 14 - 8 - the chip); the
    // closed chip cuts a long name with ".." beside its chevron, so it is not a fit error there
    let dmark = text_w(lang::text_in(l, lang::Msg::SetDraft), 1) + 10;
    for &o in lang::LANGS.iter() {
        check(o.code(), text_w(o.own_name().as_bytes(), 1), 200 - 14 - 8 - dmark - 6);
    }
    for m in [lang::Msg::SetDisplay, lang::Msg::SetDesktop, lang::Msg::SetNetwork, lang::Msg::SetSystem] {
        check(name(m), w_of(m), sw - 60);
    }
    for m in [lang::Msg::SetNoDevice, lang::Msg::SetNoAddress] {
        check(name(m), w_of(m), sw - 28);
    }
    check(name(lang::Msg::SetPixels), filled(lang::Msg::SetPixels, &[lang::Arg::N(3840), lang::Arg::N(2160)]), sw - 28);
    check(name(lang::Msg::SetConnected), filled(lang::Msg::SetConnected, &[lang::Arg::S(b"255.255.255.255")]), sw - 28);
    check(name(lang::Msg::SetCores), filled(lang::Msg::SetCores, &[lang::Arg::N(64), lang::Arg::N(65536)]), sw - 28);
    check(name(lang::Msg::SetBooted), filled(lang::Msg::SetBooted, &[lang::Arg::N(99999), lang::Arg::N(9999999)]), sw - 28);
    // About
    let aw = HOME[App::About as usize].2;
    check(name(lang::Msg::AboutMotto), w_of(lang::Msg::AboutMotto), aw - 20);
    let keys = [lang::Msg::AboutKernel, lang::Msg::AboutDesktop, lang::Msg::AboutDevice, lang::Msg::AboutMemory];
    let vx = keys.iter().map(|&m| 30 + w_of(m) + 14).max().unwrap_or(100).max(100);
    check(name(lang::Msg::AboutKernelIs), w_of(lang::Msg::AboutKernelIs), aw - vx - 10);
    check(name(lang::Msg::AboutDesktopIs), w_of(lang::Msg::AboutDesktopIs), aw - vx - 10);
    for arch in [lang::Msg::AboutArm, lang::Msg::AboutPc] {
        let a = lang::text_in(l, arch);
        check(name(lang::Msg::AboutDeviceIs), filled(lang::Msg::AboutDeviceIs, &[lang::Arg::S(a), lang::Arg::N(64)]), aw - vx - 10);
    }
    check(name(lang::Msg::AboutMemoryIs), filled(lang::Msg::AboutMemoryIs, &[lang::Arg::N(16384), lang::Arg::N(999999)]), aw - vx - 10);
    // Files: the header and the preview pane
    let fw = HOME[App::Files as usize].2;
    let split = (fw * 46 / 100).max(150);
    check(name(lang::Msg::FilesName), w_of(lang::Msg::FilesName), split - 10 - w_of(lang::Msg::FilesSize).max(34) - 10 - 6);
    check(name(lang::Msg::FilesSize), w_of(lang::Msg::FilesSize), split - 60);
    check(name(lang::Msg::FilesNone), w_of(lang::Msg::FilesNone), split - 14);
    let pane = fw - split - 1 - 20;
    for m in [lang::Msg::FilesWasm, lang::Msg::FilesRunIt, lang::Msg::FilesBinary, lang::Msg::FilesSelect] {
        check(name(m), w_of(m), pane);
    }
    check(name(lang::Msg::FilesBytes), filled(lang::Msg::FilesBytes, &[lang::Arg::N(99999999)]), pane);
    // the title bar's red button, the window names under the Circle's tiles and on the lintel
    // the narrowest app title (icon, a cut name, the unsigned mark, Close anyway, the buttons) in this language fits
    // the cell window's home width, so the window can always be made wide enough for the mark to stay whole
    check(name(lang::Msg::TitleUnsigned), lintel_min_w_in(l), HOME[App::Cell as usize].2);
    for m in [lang::Msg::AppFiles, lang::Msg::AppTerminal, lang::Msg::AppSettings, lang::Msg::AppAbout] {
        check(name(m), w_of(m), 84);
    }
    // the clock's second line on the Spine
    for m in [lang::Msg::ClockUtc, lang::Msg::ClockAm, lang::Msg::ClockPm] {
        check(name(m), w_of(m), SPINE_P - 6);
    }
    // the Circle's information box (on the current screen)
    let (_, _, iw, _) = circle_info_box();
    let sum = filled(lang::Msg::CircleSummary, &[lang::Arg::S(b"192.168.10.200"), lang::Arg::N(16384), lang::Arg::N(64)]);
    // the summary is cut with ".." when it is too wide (launcher), so only the label is a fit error here; the
    // widest line is printed by `desk lang fit` as a note
    let _ = sum;
    check(name(lang::Msg::CircleDevice), w_of(lang::Msg::CircleDevice), iw - 28 - 60);
    check(name(lang::Msg::CircleUp), filled(lang::Msg::CircleUp, &[lang::Arg::N(999999)]), iw - 28);
    // the power panel (the designer's sizes, 14 Nov: power_box and power_buttons at UI scale 1): 240 px, text 14 px
    // in, so 212 px a line; the menu's buttons are 212 px with the word centred; the confirm's two are 101 px
    for m in [lang::Msg::PowerTitle, lang::Msg::PowerOffAsk, lang::Msg::PowerRestartAsk, lang::Msg::PowerUnsaved] {
        check(name(m), w_of(m), 240 - 28);
    }
    for m in [lang::Msg::PowerOff, lang::Msg::PowerRestart] {
        check(name(m), w_of(m), 240 - 28 - 8);
    }
    let pbw = (240 - 28 - 10) / 2;
    for m in [lang::Msg::PowerOff, lang::Msg::PowerRestart, lang::Msg::PowerCancel] {
        check(name(m), w_of(m), pbw - 8);
    }
    // the Terminal's scrolled-back chip (top right, sized to its text) within the body at its home width
    let tw = HOME[App::Terminal as usize].2 - 120;
    check(name(lang::Msg::TermBackOne), w_of(lang::Msg::TermBackOne) + 12, tw);
    check(name(lang::Msg::TermBack), filled(lang::Msg::TermBack, &[lang::Arg::N(2048)]) + 12, tw);
    // the notices panel
    let nw = 280;
    for m in [lang::Msg::NotesTitle, lang::Msg::NotesNone, lang::Msg::NoteWelcome] {
        check(name(m), w_of(m), nw - 38);
    }
}

/// The Circle (the launcher): an octagon with the monument at its centre and the apps
/// standing in a ring round it; under it, this device and how to close (section 6).
fn launcher() {
    let s = st();
    let (lx, ly, lw, lh) = launcher_box();
    if let Some(step) = grow_step() {
        // motion: the octagon at a third, then two thirds, of its size, centred where it will
        // stand; no tiles yet (taps already hit the final ones)
        let (gw, gh) = (lw * step / 3, lh * step / 3);
        let (gx, gy) = (lx + (lw - gw) / 2, ly + (lh - gh) / 2);
        let k = gw * 29 / 100;
        cut(gx, gy, gw, gh, k, k, k, k, 0x16_11_0E);
        cut_frame(gx, gy, gw, gh, k, k, k, k, GOLD_DARK);
        return;
    }
    let k = lw * 29 / 100; // a regular octagon cuts about 29% off each corner
    shadow(lx, ly, lw, lh, k, k, k, k, 4, look::REACH);
    cut_a(lx, ly, lw, lh, k, k, k, k, 0x16_11_0E, if look::glass() { look::GLASS_ALPHA } else { 255 });
    cut_frame(lx, ly, lw, lh, k, k, k, k, GOLD_DARK);
    cut_frame(lx + 3, ly + 3, lw - 6, lh - 6, k - 1, k - 1, k - 1, k - 1, LINE);
    let (cx, cy) = (lx + lw / 2, ly + lh / 2 - 6);
    // the ring: sixteen beads, the apps stand on it
    let r = lw * 33 / 100;
    for (c, si) in DIRS {
        chip(cx + r * c / 1000 - 2, cy + r * si / 1000 - 2, 5, 5, 1, GOLD_DARK);
    }
    let mh = lh * 16 / 100;
    monument(cx, cy - mh / 2 - 6, mh, GOLD_PALE, GOLD_DEEP);
    let word = b"AfriSovera";
    text(cx - text_w(word, 1) / 2, cy + mh / 2 + 6, word, GOLD, 1);
    let busy = circle_busy();
    for i in 0..ntile() {
        let (tx, ty, ts) = launcher_tile(i);
        if i == s.sel {
            // the keyboard walk's choice: a gold cut frame round the tile (section 14)
            cut_frame(tx - 4, ty - 4, ts + 8, ts + 8, 6, 6, 6, 6, GOLD);
            cut_frame(tx - 3, ty - 3, ts + 6, ts + 6, 5, 5, 5, 5, GOLD);
        }
        if let Some(k) = circle_app(i) {
            // a bundled app's tile (section 32): dim while the shell is busy
            andika_icon(tx, ty, ts, busy);
            let n = CIRCLE_APPS[k].1;
            text(tx + (ts - text_w(n, 1)) / 2, ty + ts + 6, n, if busy { DIM } else { TEXT }, 1);
            continue;
        }
        icon(Some(APPS[i]), tx, ty, ts);
        let n = APPS[i].name();
        text(tx + (ts - text_w(n, 1)) / 2, ty + ts + 6, n, if s.wins[i].open { GOLD } else { TEXT }, 1);
    }
    // honest use of the space: what this device is doing now (review D2)
    let (ix, iy, iw, ih) = circle_info_box();
    chip(ix, iy, iw, ih, 6, 0x16_11_0E);
    cut_frame(ix, iy, iw, ih, 6, 6, 6, 6, LINE);
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(lang::t(lang::Msg::CircleNoNet));
    } else if ip == [0u8; 4] {
        l.s(lang::t(lang::Msg::CircleNoAddress));
    } else {
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                l.s(b".");
            }
            l.u(*o as u64);
        }
    }
    let files = (0..crate::vfs::MAX_FILES).filter(|&i| crate::vfs::is_used(i)).count() as u64;
    let mut sum = Line::new();
    sum.f(lang::Msg::CircleSummary, &[lang::Arg::S(l.get()), lang::Arg::N(crate::ram_mib()), lang::Arg::N(files)]);
    let l = sum;
    let x0 = text(ix + 14, iy + 10, lang::t(lang::Msg::CircleDevice), GOLD_DEEP, 1);
    // a long address and a big memory can be wider than the box on a small screen (desk lang fit found it in
    // English at 640x480): then the line is cut with "..", never drawn past the box's edge
    let (shown, cut) = fit(l.get(), ix + iw - 14 - (x0 + 10), 1);
    let end = text(x0 + 10, iy + 10, shown, TEXT, 1);
    if cut {
        text(end + 1, iy + 10, b"..", DIM, 1);
    }
    let mut l = Line::new();
    l.f(lang::Msg::CircleUp, &[lang::Arg::N(now_ticks() / 100)]);
    text(ix + 14, iy + 26, l.get(), DIM, 1);
}

/// The language list (the designer's spec, 14 Nov): the notes panel's shape and shadow, a row a language in its own
/// name, the current one on a gold chip, the keyboard's row in a gold outline, the ochre draft mark on a draft
/// language's row, and a 2 px bar at the right edge when the list scrolls. Mirrored in a right-to-left language.
fn lang_list() {
    let s = st();
    let (x, y, w, h) = lang_list_box();
    shadow(x, y, w, h, CUT, 0, CUT, 0, 4, look::REACH); // soft, as every panel
    cut(x, y, w, h, CUT, 0, CUT, 0, 0x16_11_0E);
    cut_frame(x, y, w, h, CUT, 0, CUT, 0, GOLD_DARK);
    let rtl = lang::current().dir() == lang::Dir::Rtl;
    let cur = lang::current();
    let (top, shown) = lang_list_rows();
    let d = lang::t(lang::Msg::SetDraft);
    let dw = text_w(d, 1) + 10;
    for r in 0..shown {
        let i = top + r;
        let l = lang::LANGS[i];
        let ry = y + 8 + r as i32 * 22;
        if l == cur {
            chip(x + 6, ry + 2, w - 12, 18, 3, GOLD_DEEP);
        }
        if i == s.lang_sel {
            cut_frame(x + 6, ry + 2, w - 12, 18, 3, 3, 3, 3, GOLD);
        }
        let name = l.own_name().as_bytes();
        let nx = if rtl { x + w - 14 - text_w(name, 1) } else { x + 14 };
        text(nx, ry + 7, name, if l == cur { NIGHT } else { TEXT }, 1);
        if lang::counts(l).1 > 0 {
            // the designer's review (14 Nov): a NIGHT chip with OCHRE text on the current (gold) row, the ochre chip on
            // the keyboard's row, and plain DIM text on the others, so the names read before the marks
            let dx = if rtl { x + 8 } else { x + w - 8 - dw };
            if l == cur {
                chip(dx, ry + 4, dw, 14, 2, NIGHT);
                text(dx + 5, ry + 7, d, OCHRE, 1);
            } else if i == s.lang_sel {
                chip(dx, ry + 4, dw, 14, 2, OCHRE);
                text(dx + 5, ry + 7, d, TERM_BG, 1);
            } else {
                let tx = if rtl { x + 8 } else { x + w - 8 - text_w(d, 1) };
                text(tx, ry + 7, d, DIM, 1);
            }
        }
    }
    let n = lang::LANGS.len();
    if n > shown && n > 0 {
        let track = h - 16;
        let bx = if rtl { x + 2 } else { x + w - 4 };
        rect(bx, y + 8 + track * top as i32 / n as i32, 2, (track * shown as i32 / n as i32).max(4), DIM);
    }
}

fn notes_panel() {
    let s = st();
    let (nx, ny, nw, nh) = notes_box();
    shadow(nx, ny, nw, nh, CUT, 0, CUT, 0, 4, look::REACH);
    cut_a(nx, ny, nw, nh, CUT, 0, CUT, 0, 0x16_11_0E, if look::glass() { look::GLASS_ALPHA } else { 255 });
    cut_frame(nx, ny, nw, nh, CUT, 0, CUT, 0, GOLD_DARK);
    text(nx + 14, ny + 10, lang::t(lang::Msg::NotesTitle), GOLD, 1);
    let total = s.nnotes.min(NNOTE);
    if total == 0 {
        text(nx + 14, ny + 32, lang::t(lang::Msg::NotesNone), DIM, 1);
    }
    for k in 0..total {
        let i = (s.nnotes - 1 - k) % NNOTE;
        let yy = ny + 28 + k as i32 * 18;
        chip(nx + 14, yy + 2, 4, 4, 1, GOLD_DEEP);
        text(nx + 24, yy, &s.notes[i][..s.note_len[i]], TEXT, 1);
    }
}

/// A panel's box with what its shadow can touch (the notifications list, the Circle).
pub(super) fn panel_box(b: Rect) -> Rect {
    if look::soft() && !st().touch {
        look::shadow_box(b.0, b.1, b.2, b.3)
    } else {
        grow(b, 4)
    }
}

/// The toast's box while it shows (shadow included), else None. Paint follows the model: the toast shows while it
/// is set, and `settle` clears it when its time runs out and damages its box, so two draws with no settle between
/// them (as `desk verify` makes) never disagree about it.
pub(super) fn toast_box() -> Option<Rect> {
    toast_place()
}

/// The toast's box while it is set, whether or not its time has run out yet: the model's
/// snapshot uses this, so a toast that expires between two snapshots still damages its box
/// (a partial frame for something else could otherwise leave a strip of it on the screen).
pub(super) fn toast_place() -> Option<Rect> {
    let (x, y, w, h) = toast_rect()?;
    Some(if look::soft() && !st().touch { look::shadow_box(x, y, w, h) } else { (x, y, w + 3, h + 3) })
}

/// The toast itself, without its shadow.
fn toast_rect() -> Option<Rect> {
    let s = st();
    if s.nnotes == 0 || s.notes_open || s.toast_until == 0 {
        return None;
    }
    let i = (s.nnotes - 1) % NNOTE;
    let w = text_w(&s.notes[i][..s.note_len[i]], 1) + 40;
    // pointer: the top right corner; touch: a snackbar at the foot, never over a lintel
    // (design review D2)
    let sw = spine_w();
    let (x, y) = if s.touch { (sw + (W() - sw - w) / 2, H() - bar_h() - 46) } else { (W() - w - 10, TOP + 10) };
    Some((x, y, w, 28))
}

fn toast() {
    let s = st();
    let Some((x, y, w, _)) = toast_rect() else { return };
    let i = (s.nnotes - 1) % NNOTE;
    let msg = &s.notes[i][..s.note_len[i]];
    shadow(x, y, w, 28, 6, 0, 6, 0, 3, look::REACH);
    cut_a(x, y, w, 28, 6, 0, 6, 0, SURFACE2, if look::glass() { look::GLASS_ALPHA } else { 255 });
    cut_frame(x, y, w, 28, 6, 0, 6, 0, GOLD_DARK);
    chip(x + 12, y + 10, 8, 8, 2, GOLD);
    text(x + 28, y + 10, msg, TEXT, 1);
}

/// Draw the whole desktop into the shared framebuffer.
pub(super) fn draw() {
    let s = st();
    let t0 = crate::timer::cycles();
    wallpaper();
    for k in 0..s.nz {
        window(s.z[k]);
    }
    spine();
    toast();
    if s.notes_open {
        notes_panel();
    }
    if s.launcher {
        launcher();
    }
    if s.lang_list {
        lang_list();
    }
    // the power panel last, opaque: its words are never under glass (the architect, 14 Nov)
    if s.power > 0 {
        power_panel();
    }
    marks_on_top();
    s.frames += 1;
    let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    // SAFETY: mainline only.
    unsafe { DRAW_US = (us, DRAW_US.1.max(us)) };
    crate::canvas::mark();
}

/// The last and the worst whole-desktop draw, in microseconds (`desk perf`).
pub(super) static mut DRAW_US: (u64, u64) = (0, 0);
/// Partial frames (only the damaged boxes): last and worst time in microseconds, and how many.
pub(super) static mut PART_US: (u64, u64, u64) = (0, 0, 0);

fn touches(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Redraw only the damaged boxes (afriui.md section 15): each box on its own, every fill
/// clipped to it, and whatever does not touch it skipped whole.
pub(super) fn draw_damage(boxes: &[Rect]) {
    let s = st();
    let t0 = crate::timer::cycles();
    for &r in boxes {
        // SAFETY: mainline only; reset below.
        unsafe { DCLIP = r };
        wallpaper();
        for k in 0..s.nz {
            let a = s.z[k];
            if touches(win_box(a), r) {
                window(a);
            }
        }
        if touches(dock_box(), r) {
            spine();
        }
        if toast_box().is_some_and(|b| touches(b, r)) {
            toast();
        }
        if s.notes_open && touches(panel_box(notes_box()), r) {
            notes_panel();
        }
        if s.launcher && (touches(panel_box(launcher_box()), r) || touches(grow(circle_info_box(), 4), r)) {
            launcher();
        }
        if s.lang_list && touches(panel_box(lang_list_box()), r) {
            lang_list();
        }
        // the power panel last, opaque: its words are never under glass (the architect, 14 Nov)
        if s.power > 0 && touches(panel_box(power_box()), r) {
            power_panel();
        }
        marks_on_top();
    }
    // SAFETY: as above.
    unsafe { DCLIP = (0, 0, i32::MAX / 2, i32::MAX / 2) };
    s.frames += 1;
    let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    // SAFETY: mainline only.
    unsafe { PART_US = (us, PART_US.1.max(us), PART_US.2 + 1) };
    crate::canvas::mark();
}

/// FNV-1a over bytes, for the probe's hashes.
fn fnv(h: u64, b: &[u8]) -> u64 {
    b.iter().fold(h, |h, &x| (h ^ x as u64).wrapping_mul(0x100_0000_01b3))
}

/// A hash of the screen's pixels in (x, y, w, h), leaving out those `skip` says.
fn screen_hash(x: i32, y: i32, w: i32, h: i32, skip: &dyn Fn(i32, i32) -> bool) -> u64 {
    let (cw, ch) = (W(), H());
    let p = crate::canvas::pixels();
    let mut hsh = 0xcbf2_9ce4_8422_2325u64;
    for yy in y.max(0)..(y + h).min(ch) {
        for xx in x.max(0)..(x + w).min(cw) {
            if !skip(xx, yy) {
                // SAFETY: (xx, yy) is on the canvas, which is W() pixels a row.
                let v = unsafe { *p.add(yy as usize * cw as usize + xx as usize) };
                hsh = fnv(hsh, &v.to_le_bytes());
            }
        }
    }
    hsh
}

/// The visual order of a line as `desk shape hex` prints it (SHAPING_PLAN.md C1): characters after the explicit
/// controls are dropped, the paragraph level, each character's level (x for a boundary-neutral one), the order left
/// to right, the width at cap 9 and the oracle hash.
pub(super) fn shape_report(b: &[u8]) {
    let mut cps = [0u32; bidi::MAX];
    let (mut n, mut i, mut dropped) = (0, 0, 0);
    while i < b.len() {
        let cp = vector::next_char(b, &mut i);
        if bidi::is_explicit(cp) {
            dropped += 1;
        } else if n < bidi::MAX {
            cps[n] = cp;
            n += 1;
        }
    }
    let mut levels = [0u8; bidi::MAX];
    let para = bidi::resolve(&cps[..n], None, &mut levels[..n]);
    let mut order = [0u16; bidi::MAX];
    let m = bidi::reorder(&levels[..n], &mut order[..n]);
    crate::kprint!("[desk] shape: {} chars, {} dropped, paragraph {}, levels", n, dropped, para);
    for &lv in &levels[..n] {
        if lv == bidi::REMOVED {
            crate::kprint!(" x");
        } else {
            crate::kprint!(" {}", lv);
        }
    }
    crate::kprint!(", order");
    for &k in &order[..m] {
        crate::kprint!(" {}", k);
    }
    crate::kprintln!(", width {}, needed {}", vtext_w(b, 9), if shape::needed(b) { "yes" } else { "no" });
    crate::kprintln!("[desk] shape: oracle {:016x}", shape::oracle_hash(b));
}

/// `desk shape probe` (SHAPING_PLAN.md C1): the oracle hashes of every test string, the coverage hash of the sample
/// line (the same on ARM and PC), widths drawn against measured, the limits, the fast test, and the speed. Draws
/// over the desk, then redraws it whole.
pub(super) fn shape_probe() {
    let s = st();
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    // SAFETY: mainline only; put back at the end.
    let (keep_draw, keep_part, keep_frames) = unsafe { (DRAW_US, PART_US, s.frames) };
    let was = vector_on();
    VECTOR.store(true, core::sync::atomic::Ordering::Relaxed);
    // 1. the oracle: every test string, hashed as tools/face/shape_expected.txt hashes HarfBuzz's answer
    for (k, t) in shape_data::TESTS.iter().enumerate() {
        crate::kprintln!("[desk] shape oracle {} {:016x}", k, shape::oracle_hash(t.as_bytes()));
    }
    // 2. the sample line's coverage at cap 9 and 18, hashed with each glyph's place: the same bytes on ARM and PC
    let sample = "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} AfriSovera \u{662}\u{660}\u{662}\u{666}".as_bytes();
    let mut hs = [0u64; 2];
    for (k, cap) in [9, 18].into_iter().enumerate() {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        shape::layout(sample, cap, None, &mut |g, pen, y| {
            h = fnv(h, &[g.face, g.boxed as u8]);
            h = fnv(h, &g.g.to_le_bytes());
            h = fnv(h, &pen.to_le_bytes());
            h = fnv(h, &y.to_le_bytes());
            if !g.boxed {
                vector::glyph_in(g.face, g.g, cap, (pen >> 6) & 3, &mut |lf, tp, w, hh, cov| {
                    h = fnv(fnv(h, &[lf as u8, tp as u8, w as u8, hh as u8]), cov);
                });
            }
        });
        hs[k] = h;
    }
    crate::kprintln!("[desk] shape: coverage hash cap 9 {:016x}, cap 18 {:016x}", hs[0], hs[1]);
    // 3. drawn width against measured width, for Latin, Arabic, mixed, bad UTF-8 and explicit controls
    let lines: [&[u8]; 8] = [
        b"Settings: AfriSovera desk, a window title",
        "\u{1EB9}\u{1ECD}\u{1E63} \u{25B}\u{254}".as_bytes(),
        sample,
        "\u{644}\u{627} \u{644}\u{623} (\u{628}\u{643}) [x] 12.5%".as_bytes(),
        "abc \u{202E}\u{645}\u{631}\u{62D}\u{628}\u{627}\u{202C} def".as_bytes(),
        &[0xD8, 0xA8, 0xD8, 0x20, 0xFF, 0xD9, 0x84, 0xD8],
        "\u{5D0}\u{5D1} \u{200F}x\u{200E} \u{61C}".as_bytes(),
        "\u{645}\u{64E}\u{631}\u{652}\u{62D}\u{64E}\u{628}\u{64B}\u{627}".as_bytes(),
    ];
    let mut same = 0;
    for l in lines {
        rect(10, 30, W() - 20, 40, SURFACE);
        let end = vtext(14, 40, l, TEXT, 9);
        same += (end - 14 == vtext_w(l, 9)) as usize;
    }
    crate::kprintln!("[desk] shape: widths: {} of {} drawn as measured", same, lines.len());
    // 4. limits: a line of 300 characters (cut to 256, 44 counted), two explicit controls dropped
    let mut long = [0u8; 600];
    for p in long.chunks_mut(2) {
        p.copy_from_slice(&[0xD8, 0xA8]);
    }
    shape::reset_cache(); // the lines above are cached; measure the cut and the drop on fresh shaping
    // SAFETY: scalar reads.
    let (d0, c0) = unsafe { (shape::STATS.0, shape::STATS.1) };
    let lw = vtext_w(&long, 9);
    let _ = vtext_w(lines[4], 9);
    // SAFETY: scalar reads.
    let (d1, c1) = unsafe { (shape::STATS.0, shape::STATS.1) };
    crate::kprintln!("[desk] shape: limits: 300 characters gave {} cut and width {}, controls dropped {}", c1 - c0, lw, d1 - d0);
    // 5. the fast test: Latin and Yoruba stay on the old path; Arabic, Hebrew and a lone control do not
    let yn = |b: bool| if b { "yes" } else { "no" };
    crate::kprintln!(
        "[desk] shape: needed: latin {}, yoruba {}, arabic {}, hebrew {}, control {}",
        yn(shape::needed(lines[0])),
        yn(shape::needed(lines[1])),
        yn(shape::needed(sample)),
        yn(shape::needed(lines[6])),
        yn(shape::needed("abc\u{202E}".as_bytes()))
    );
    // 6. speed: a 40-character Arabic line and the 40-character Latin title, cold, then warm (best and worst of 20)
    let arabic40: &str = "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} \u{645}\u{631}\u{62D}\u{628}\u{627} \u{628}\u{643} \u{641}\u{64A} \u{627}\u{644}\u{625}\u{639}\u{62F}\u{627}\u{62F}\u{627}\u{62A} \u{627}\u{644}\u{645} \u{648}";
    for (name, line) in [("arabic", arabic40.as_bytes()), ("latin", &lines[0][..40])] {
        let mut t = [[0u64; 3]; 2];
        for (si, scale) in [1, 2].into_iter().enumerate() {
            rect(10, 30, W() - 20, 60, SURFACE);
            vector::reset();
            shape::reset_cache();
            let t0 = crate::timer::cycles();
            text(14, 40, line, TEXT, scale);
            let cold = us(t0);
            let (mut warm, mut worst) = (u64::MAX, 0);
            for _ in 0..20 {
                let t0 = crate::timer::cycles();
                text(14, 40, line, TEXT, scale);
                let t = us(t0);
                warm = warm.min(t);
                worst = worst.max(t);
            }
            t[si] = [cold, warm, worst];
        }
        crate::kprintln!(
            "[desk] shape: {} {} characters: cap 9 cold {} us, warm {} us (worst of 20 {} us); cap 18 cold {} us, warm {} us (worst of 20 {} us)",
            name,
            core::str::from_utf8(line).map_or(0, |v| v.chars().count()),
            t[0][0],
            t[0][1],
            t[0][2],
            t[1][0],
            t[1][1],
            t[1][2]
        );
    }
    // the Arabic line shaped every time (no line cache), warm glyphs: what a line costs the first frame it is seen
    let mut every = [0u64; 2];
    for (si, scale) in [1, 2].into_iter().enumerate() {
        let mut best = u64::MAX;
        for _ in 0..8 {
            shape::reset_cache();
            let t0 = crate::timer::cycles();
            text(14, 40, arabic40.as_bytes(), TEXT, scale);
            best = best.min(us(t0));
        }
        every[si] = best;
    }
    crate::kprintln!("[desk] shape: arabic 40 characters shaped every time: cap 9 {} us, cap 18 {} us", every[0], every[1]);
    // SAFETY: scalar reads.
    let cs = unsafe { shape::CSTATS };
    crate::kprintln!("[desk] shape: line cache hits {}, misses {}, emptied {}", cs.0, cs.1, cs.2);
    // SAFETY: scalar reads.
    let st3 = unsafe { shape::STATS };
    crate::kprintln!("[desk] shape: lines shaped {}, controls dropped {}, characters cut {}", st3.2, st3.0, st3.1);
    VECTOR.store(was, core::sync::atomic::Ordering::Relaxed);
    draw();
    // SAFETY: mainline only.
    unsafe {
        DRAW_US = keep_draw;
        PART_US = keep_part;
    }
    s.frames = keep_frames;
}

/// `desk font probe` (afriui.md section 26): the outline face's speed against the bitmap face's, and its checks.
/// Draws over the desk, then redraws it whole; leaves the face as it was.
pub(super) fn font_probe() {
    let s = st();
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    let was = vector_on();
    // SAFETY: mainline only; put back at the end.
    let (keep_draw, keep_part, keep_frames) = unsafe { (DRAW_US, PART_US, s.frames) };
    let set = |v: bool| VECTOR.store(v, core::sync::atomic::Ordering::Relaxed);
    let az: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let title: &[u8] = b"Settings: AfriSovera desk, a window title";
    // 1. text speed: the bitmap face, then the outline face from an empty cache, then warm (fastest of 8)
    for (name, line) in [("A to Z", az), ("40-character title", &title[..40])] {
        let mut t = [[0u64; 4]; 2];
        for (si, scale) in [1, 2].into_iter().enumerate() {
            rect(10, 30, W() - 20, 40, SURFACE);
            set(false);
            let mut bm = u64::MAX;
            for _ in 0..8 {
                let t0 = crate::timer::cycles();
                text(14, 40, line, TEXT, scale);
                bm = bm.min(us(t0));
            }
            set(true);
            vector::reset();
            let t0 = crate::timer::cycles();
            text(14, 40, line, TEXT, scale);
            let cold = us(t0);
            let (mut warm, mut worst) = (u64::MAX, 0);
            for _ in 0..20 {
                let t0 = crate::timer::cycles();
                text(14, 40, line, TEXT, scale);
                let t = us(t0);
                warm = warm.min(t);
                worst = worst.max(t);
            }
            t[si] = [bm, cold, warm, worst];
        }
        crate::kprintln!(
            "[desk] font: {} at cap 9: bitmap {} us, outline cold {} us, warm {} us (worst of 20 {} us); at cap 18: bitmap {} us, outline cold {} us, warm {} us (worst of 20 {} us)",
            name, t[0][0], t[0][1], t[0][2], t[0][3], t[1][0], t[1][1], t[1][2], t[1][3]
        );
    }
    // 2. the coverage of a fixed word, hashed: the same bytes on ARM and PC
    let word = "AfriSovera \u{25B}\u{254}\u{14B} \u{1EB9}\u{1ECD}\u{1E63} \u{14A}\u{190}\u{186}".as_bytes();
    let mut hs = [0u64; 2];
    for (k, cap) in [9, 18].into_iter().enumerate() {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut i = 0;
        while i < word.len() {
            if let Some(g) = vector::lookup(vector::next_char(word, &mut i)) {
                h = fnv(h, &vector::advance(g, cap).to_le_bytes());
                vector::glyph(g, cap, 0, &mut |lf, tp, w, hh, cov| {
                    h = fnv(fnv(h, &[lf as u8, tp as u8, w as u8, hh as u8]), cov);
                });
            }
        }
        hs[k] = h;
    }
    crate::kprintln!("[desk] font: coverage hash cap 9 {:016x}, cap 18 {:016x}", hs[0], hs[1]);
    // 3. half a pixel moves only the edges: no pixel full in both changes much, and the ink stays about the same
    let o = vector::lookup(0x6F).unwrap_or(0);
    let mut a = [0u8; 400];
    let mut b = [0u8; 400];
    let (mut sa, mut sb, mut dims) = (0u32, 0u32, (0, 0, 0, 0));
    vector::glyph(o, 18, 0, &mut |lf, tp, w, h, cov| {
        dims = (lf, tp, w, h);
        let n = cov.len().min(400);
        a[..n].copy_from_slice(&cov[..n]);
        sa = cov.iter().map(|&v| v as u32).sum();
    });
    let mut same_box = false;
    vector::glyph(o, 18, 2, &mut |lf, tp, w, h, cov| {
        same_box = (lf, tp, w, h) == dims;
        let n = cov.len().min(400);
        b[..n].copy_from_slice(&cov[..n]);
        sb = cov.iter().map(|&v| v as u32).sum();
    });
    let n = dims.2 * dims.3;
    let differ = (0..n.min(400)).filter(|&k| a[k] != b[k]).count();
    let full = (0..n.min(400)).filter(|&k| a[k] == 255 && b[k] == 255).count();
    crate::kprintln!(
        "[desk] font: half pixel 'o' at cap 18: ink {} and {} ({}), {} of {} pixels differ, {} full in both, same box {}",
        sa,
        sb,
        if sa.abs_diff(sb) * 50 <= sa.max(1) { "within 2%" } else { "NOT within 2%" },
        differ,
        n,
        full,
        if same_box { "yes" } else { "no" }
    );
    // 4. a character the font does not have is the box
    let pua = "\u{E000}".as_bytes();
    crate::kprintln!(
        "[desk] font: U+E000 is {} and {} pixels wide (the box is {})",
        if vector::lookup(0xE000).is_none() { "not in the font" } else { "IN the font" },
        vtext_w(pua, 9),
        vector::box_size(9).1
    );
    // 5. UTF-8: six African letters, spaces, and a sequence cut short at the very end
    let mut u = [0u8; 32];
    let src = "\u{25B} \u{254} \u{14B} \u{1EB9} \u{1ECD} \u{1E63}".as_bytes();
    u[..src.len()].copy_from_slice(src);
    u[src.len()] = 0xE1;
    u[src.len() + 1] = 0xBA;
    let ul = src.len() + 2;
    let (mut chars, mut bad, mut i, mut known) = (0, 0, 0, 0);
    while i < ul {
        let cp = vector::next_char(&u[..ul], &mut i);
        chars += 1;
        bad += (cp == 0xFFFD) as usize;
        known += vector::lookup(cp).is_some() as usize;
    }
    crate::kprintln!("[desk] font: utf-8: {} characters, {} in the font, {} bad, ended at byte {} of {}", chars, known, bad, i, ul);
    // 6. the cache: fill it until it empties itself, then the same text draws the same pixels
    let probe_line = "Files \u{2022} Settings \u{1EB9}\u{1ECD}".as_bytes();
    let runs_were = RUN_ON.swap(false, core::sync::atomic::Ordering::Relaxed);
    rect(10, 80, 300, 30, SURFACE);
    vtext(14, 88, probe_line, GOLD, 9);
    let h1 = screen_hash(10, 80, 300, 30, &|_, _| false);
    let r0 = vector::stats().4;
    let mut asked = 0u32;
    'fill: for cap in vector::MIN_CAP..=vector::MAX_CAP {
        for sub in 0..4 {
            for g in 0..400u16 {
                vector::glyph(g, cap, sub, &mut |_, _, _, _, _| {});
                asked += 1;
                if vector::stats().4 > r0 + 1 {
                    break 'fill;
                }
            }
        }
    }
    rect(10, 80, 300, 30, SURFACE);
    vtext(14, 88, probe_line, GOLD, 9);
    let h2 = screen_hash(10, 80, 300, 30, &|_, _| false);
    RUN_ON.store(runs_were, core::sync::atomic::Ordering::Relaxed);
    crate::kprintln!(
        "[desk] font: cache: {} glyphs asked for, emptied {} time(s), the same text after: {}",
        asked,
        vector::stats().4 - r0,
        if h1 == h2 { "same pixels" } else { "DIFFERENT pixels" }
    );
    // 7. the largest size, and a size past it clamped
    let drop0 = vector::stats().5;
    let mut big = (0, 0, 0usize);
    for ch in [0x57u32, 0x1EB8, 0x14A] {
        if let Some(g) = vector::lookup(ch) {
            vector::glyph(g, 1000, 0, &mut |_, _, w, h, cov| {
                big = (big.0.max(w), big.1.max(h), big.2 + cov.iter().filter(|&&v| v == 255).count());
            });
        }
    }
    crate::kprintln!(
        "[desk] font: largest: cap {} when 1000 is asked, boxes up to {}x{}, {} full pixels, lines dropped {}",
        vector::clamp_cap(1000),
        big.0,
        big.1,
        big.2,
        vector::stats().5 - drop0
    );
    // 8. clipping: text across a window's right edge and its cut corner draws nothing outside
    let (bx, by, bw, bh, k) = (100, 200, 60, 40, 8);
    let zone = by + bh - k;
    let outside = move |x: i32, y: i32| x < bx || x >= bx + bw || y < by || y >= by + bh || (y >= zone && x >= bx + bw - (y - zone));
    rect(90, 190, 200, 60, SURFACE);
    let before = screen_hash(90, 190, 200, 60, &|x, y| !outside(x, y));
    // SAFETY: mainline only; cleared below.
    unsafe { CLIP = Some((bx, by, bw, bh, k)) };
    vtext(130, 206, b"Clipped text", TEXT, 9);
    vtext(140, 224, b"Wg", TEXT, 18);
    // SAFETY: as above.
    unsafe { CLIP = None };
    let after = screen_hash(90, 190, 200, 60, &|x, y| !outside(x, y));
    let mut lit = 0;
    let p = crate::canvas::pixels();
    for y in by..by + bh {
        for x in bx..bx + bw {
            // SAFETY: on the canvas (every desk canvas is at least 640x480).
            lit += (unsafe { *p.add(y as usize * W() as usize + x as usize) } != SURFACE) as usize;
        }
    }
    crate::kprintln!(
        "[desk] font: clip: outside the window and its cut corner {}, {} pixels drawn inside",
        if before == after { "unchanged" } else { "CHANGED" },
        lit
    );
    // 9. frames: a full frame with each face, the first after an empty cache, and a title-bar sized partial frame
    let part: Rect = (W() / 2 - 150, H() / 2 - 12, 300, 24);
    let frame = |v: bool, cold: bool| -> (u64, u64) {
        set(v);
        let (mut f, mut pt) = (u64::MAX, u64::MAX);
        for k in 0..3 {
            if cold {
                vector::reset();
                let t0 = crate::timer::cycles();
                draw();
                return (us(t0), 0);
            }
            let _ = k;
            let t0 = crate::timer::cycles();
            draw();
            f = f.min(us(t0));
            let t0 = crate::timer::cycles();
            draw_damage(&[part]);
            pt = pt.min(us(t0));
        }
        (f, pt)
    };
    let (bf, bp) = frame(false, false);
    let (first, _) = frame(true, true);
    let (vf, vp) = frame(true, false);
    crate::kprintln!(
        "[desk] font: frames at {}x{}: full bitmap {} us, outline {} us ({}%), the first after an empty cache {} us; partial 300x24 bitmap {} us, outline {} us",
        W(),
        H(),
        bf,
        vf,
        vf * 100 / bf.max(1),
        first,
        bp,
        vp
    );
    let (used, slots, made, hits, resets, _) = vector::stats();
    crate::kprintln!("[desk] font: cache {} bytes in {} glyphs; made {}, hits {}, emptied {}", used, slots, made, hits, resets);
    set(was);
    draw();
    // SAFETY: mainline only.
    unsafe {
        DRAW_US = keep_draw;
        PART_US = keep_part;
    }
    s.frames = keep_frames;
}


/// `desk look probe` (afriui.md section 30): the corner table, the see-through mix, the shadow profile, frames with
/// each switch, the mixed pixels of a frame, the run cache and its collision test. Leaves the switches as they were.
pub(super) fn look_probe() {
    let s = st();
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    // SAFETY: mainline only; put back at the end.
    let (keep_draw, keep_part, keep_frames) = unsafe { (DRAW_US, PART_US, s.frames) };
    let was = (look::corners(), look::soft(), look::glass_switch(), RUN_ON.load(core::sync::atomic::Ordering::Relaxed));
    let set = |c: look::Corners, soft: bool, glass: bool| {
        look::set_corners(c);
        look::set_soft(soft);
        look::set_glass(glass);
    };
    // 1. the corner table, made at build time
    let cut_mid = look::cov(look::Corners::Cut, 8, 3, 4);
    crate::kprintln!(
        "[desk] look: round corner table r8 {:016x}, r16 {:016x}; a cut corner's diagonal pixel {}",
        look::round_hash(8),
        look::round_hash(16),
        cut_mid
    );
    // 2. see-through: white at 220 over black
    rect(20, 40, 10, 10, 0);
    look::fill(20, 40, 10, 10, 0xFF_FF_FF, look::GLASS_ALPHA);
    let p = crate::canvas::pixels();
    // SAFETY: (24, 44) is on every desk canvas.
    let v = unsafe { *p.add(44 * W() as usize + 24) } & 0xFF_FF_FF;
    crate::kprintln!("[desk] look: see-through white at {} over black gives {:06x} (expected dcdcdc)", look::GLASS_ALPHA, v);
    // 3. the shadow's profile
    crate::kprintln!(
        "[desk] look: shadow alpha {} under the edge, {} at {} px out, 0 past it; damage reaches {} px out and {} down",
        look::PROFILE[0],
        look::PROFILE[look::REACH as usize],
        look::REACH,
        look::REACH,
        look::DROP
    );
    // 4. frames: flat (before step 2), each part alone, everything (smooth cut and round); the partial frame is a
    // title-bar sized box; mixed pixels counted over one full frame
    let part: Rect = (W() / 2 - 150, H() / 2 - 12, 300, 24);
    let frame = |c: look::Corners, soft: bool, glass: bool| -> (u64, u64, u64) {
        set(c, soft, glass);
        let (mut f, mut pt, mut mixed) = (u64::MAX, u64::MAX, 0);
        for k in 0..3 {
            // SAFETY: a scalar read and write, mainline only.
            let m0 = unsafe { crate::framebuffer::MIXED };
            let t0 = crate::timer::cycles();
            draw();
            f = f.min(us(t0));
            if k == 0 {
                // SAFETY: as above.
                mixed = unsafe { crate::framebuffer::MIXED } - m0;
            }
            let t0 = crate::timer::cycles();
            draw_damage(&[part]);
            pt = pt.min(us(t0));
        }
        (f, pt, mixed)
    };
    let flat = frame(look::Corners::Hard, false, false);
    let corners = frame(look::Corners::Cut, false, false);
    let shadows = frame(look::Corners::Hard, true, false);
    let glass = frame(look::Corners::Hard, false, true);
    let all = frame(look::Corners::Cut, true, true);
    let round = frame(look::Corners::Round, true, true);
    let pct = |t: u64| t * 100 / flat.0.max(1);
    let ppct = |t: u64| t * 100 / flat.1.max(1);
    crate::kprintln!(
        "[desk] look: full frames at {}x{}: flat {} us; smooth corners {}%, soft shadows {}%, see-through {}%; all on {} us ({}%), round {} us ({}%)",
        W(), H(), flat.0, pct(corners.0), pct(shadows.0), pct(glass.0), all.0, pct(all.0), round.0, pct(round.0)
    );
    crate::kprintln!(
        "[desk] look: partial 300x24: flat {} us, all on {} us ({}%), round {} us ({}%); mixed pixels in a full frame: flat {}, all on {}, round {}",
        flat.1, all.1, ppct(all.1), round.1, ppct(round.1), flat.2, all.2, round.2
    );
    // the perf engineer's fixed lines (perf, pc-perf, perf-release grade ARM touch mode and track pointer mode, the
    // architect's step 2 verdict): one per mode and look, timed as `desk perf` times frames. Fields and order fixed.
    let was_touch = s.touch;
    for (touch, mode) in [(true, "touch"), (false, "pointer")] {
        s.touch = touch;
        for (premium, name) in [(false, "flat"), (true, "premium")] {
            let r = if premium { frame(look::Corners::Cut, true, true) } else { frame(look::Corners::Hard, false, false) };
            crate::kprintln!("[desk] probe: mode {} look {} {}x{}: full {} us, partial {} us", mode, name, W(), H(), r.0, r.1);
        }
    }
    s.touch = was_touch;
    // the first frame after both caches are emptied, everything on
    set(look::Corners::Cut, true, true);
    vector::reset();
    run_reset();
    let t0 = crate::timer::cycles();
    draw();
    crate::kprintln!("[desk] look: the first frame after empty caches, all on: {} us", us(t0));
    // 5. the run cache: the 40-character title, best and worst of 20, cache on and off
    let title: &[u8] = b"Settings: AfriSovera desk, a window title";
    let mut t = [[0u64; 2]; 2];
    for (k, on) in [true, false].into_iter().enumerate() {
        RUN_ON.store(on, core::sync::atomic::Ordering::Relaxed);
        rect(10, 30, W() - 20, 40, SURFACE);
        text(14, 40, &title[..40], TEXT, 1);
        let (mut best, mut worst) = (u64::MAX, 0);
        for _ in 0..20 {
            let t0 = crate::timer::cycles();
            text(14, 40, &title[..40], TEXT, 1);
            let v = us(t0);
            best = best.min(v);
            worst = worst.max(v);
        }
        t[k] = [best, worst];
    }
    RUN_ON.store(true, core::sync::atomic::Ordering::Relaxed);
    crate::kprintln!(
        "[desk] look: 40-character title at cap 9, warm: run cache on {} us (worst of 20 {} us), off {} us (worst of 20 {} us)",
        t[0][0], t[0][1], t[1][0], t[1][1]
    );
    crate::kprintln!("[desk] probe: title worst of 20: {} us", t[0][1]); // the perf engineer's fixed line
    // 6. collisions: with every hash colliding, two strings still draw as themselves (compared with the cache off)
    let words: [&[u8]; 2] = [b"Alpha one", b"Bravo two"];
    let mut with = [0u64; 2];
    let mut without = [0u64; 2];
    for (k, wd) in words.iter().enumerate() {
        RUN_ON.store(false, core::sync::atomic::Ordering::Relaxed);
        rect(10, 80, 200, 30, SURFACE);
        text(14, 88, wd, TEXT, 1);
        without[k] = screen_hash(10, 80, 200, 30, &|_, _| false);
    }
    RUN_ON.store(true, core::sync::atomic::Ordering::Relaxed);
    RUN_WEAK.store(true, core::sync::atomic::Ordering::Relaxed);
    run_reset();
    // SAFETY: a scalar read, mainline only.
    let hits0 = unsafe { RUN_STATS.0 };
    for _ in 0..2 {
        for (k, wd) in words.iter().enumerate() {
            rect(10, 80, 200, 30, SURFACE);
            text(14, 88, wd, TEXT, 1);
            with[k] = screen_hash(10, 80, 200, 30, &|_, _| false);
        }
    }
    // SAFETY: as above.
    let hits = unsafe { RUN_STATS.0 } - hits0;
    RUN_WEAK.store(false, core::sync::atomic::Ordering::Relaxed);
    run_reset();
    crate::kprintln!(
        "[desk] look: run cache with every hash colliding: {} hit(s), each string drew as itself: {}",
        hits,
        if with == without && without[0] != without[1] { "yes" } else { "NO" }
    );
    // SAFETY: scalar reads, mainline only.
    let (h, m, e) = unsafe { RUN_STATS };
    crate::kprintln!("[desk] look: run cache {} bytes in {} runs; hits {}, made {}, emptied {}", unsafe { RUN_USED }, unsafe { RUN_N }, h, m, e);
    set(was.0, was.1, was.2);
    RUN_ON.store(was.3, core::sync::atomic::Ordering::Relaxed);
    draw();
    // SAFETY: mainline only.
    unsafe {
        DRAW_US = keep_draw;
        PART_US = keep_part;
    }
    s.frames = keep_frames;
}
