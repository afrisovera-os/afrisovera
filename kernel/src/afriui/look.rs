//! The premium look, step 2 (docs/design/afriui.md section 30): one shape for every corner (Hard as before, Cut made
//! smooth, or Round), soft shadows from a fixed profile, see-through panels, and the switches that choose them.
//! Everything is integer and the same on ARM and PC. Mainline only, as all of paint.
#![allow(dead_code)]

use super::*;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// The corner style. Cut (made smooth) is the default until the owner chooses between it and Round (the architect,
/// 13 Nov); Hard is today's drawing, kept for before shots and as the fallback.
#[derive(Clone, Copy, PartialEq)]
pub(in super::super) enum Corners {
    Hard,
    Cut,
    Round,
}
static CORNERS: AtomicU8 = AtomicU8::new(1);
static SOFT: AtomicBool = AtomicBool::new(true);
static GLASS: AtomicBool = AtomicBool::new(true);

pub(in super::super) fn corners() -> Corners {
    match CORNERS.load(Ordering::Relaxed) {
        0 => Corners::Hard,
        2 => Corners::Round,
        _ => Corners::Cut,
    }
}
pub(in super::super) fn set_corners(c: Corners) {
    CORNERS.store(c as u8, Ordering::Relaxed);
}
/// Soft shadows (pointer mode only; touch mode has no shadow at all, as before).
pub(in super::super) fn soft() -> bool {
    SOFT.load(Ordering::Relaxed)
}
pub(in super::super) fn set_soft(on: bool) {
    SOFT.store(on, Ordering::Relaxed);
}
/// See-through panels (pointer mode only).
pub(in super::super) fn glass() -> bool {
    GLASS.load(Ordering::Relaxed) && !st().touch
}
pub(in super::super) fn glass_switch() -> bool {
    GLASS.load(Ordering::Relaxed)
}
pub(in super::super) fn set_glass(on: bool) {
    GLASS.store(on, Ordering::Relaxed);
}

/// How much of a see-through panel is its own colour (of 255): about 86%.
pub(in super::super) const GLASS_ALPHA: u8 = 220;

/// Coverage passed straight through (shapes and shadows are not text: no gamma).
pub(in super::super) static IDENT: [u8; 256] = ident();
const fn ident() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = i as u8;
        i += 1;
    }
    t
}

// ---- Round corners: a coverage table made at build time (the architect's note: .rodata, no first-use state)

/// Radii up to this come from the table; a bigger one (the Circle's) is worked out per row.
pub(in super::super) const MAXR: usize = 24;
const fn off(r: usize) -> usize {
    // the sum of k * k for k < r
    if r == 0 {
        0
    } else {
        (r - 1) * r * (2 * r - 1) / 6
    }
}
const TOTAL: usize = off(MAXR + 1);
/// Radius r is r x r bytes at off(r): row j and column i counted inward from the corner's outer edges; each pixel
/// is 8 x 8 points tested against the circle of radius r about (r, r).
pub(in super::super) static ROUND: [u8; TOTAL] = build_round();
const fn build_round() -> [u8; TOTAL] {
    let mut t = [0u8; TOTAL];
    let mut r = 1;
    while r <= MAXR {
        let c = 16 * r as i64; // the centre and the radius, in 1/16 pixel
        let mut j = 0;
        while j < r {
            let mut i = 0;
            while i < r {
                let mut n = 0i64;
                let mut sy = 0;
                while sy < 8 {
                    let mut sx = 0;
                    while sx < 8 {
                        let dx = c - (16 * i as i64 + 2 * sx + 1);
                        let dy = c - (16 * j as i64 + 2 * sy + 1);
                        if dx * dx + dy * dy <= c * c {
                            n += 1;
                        }
                        sx += 1;
                    }
                    sy += 1;
                }
                t[off(r) + j * r + i] = ((n * 255 + 32) / 64) as u8;
                i += 1;
            }
            j += 1;
        }
        r += 1;
    }
    t
}

/// Rounded distances sqrt(u * u + v * v) for u, v below 32, made at build time: the shadows' corner squares ask for
/// one a pixel, and a square root at opt-level 0 cost more than the mixing.
static DIST: [u8; 32 * 32] = build_dist();
const fn build_dist() -> [u8; 32 * 32] {
    let mut t = [0u8; 32 * 32];
    let mut u = 0;
    while u < 32 {
        let mut v = 0;
        while v < 32 {
            let q = (u * u + v * v) as i64;
            // the integer square root of q, rounded: the largest r with r * r <= q, then up if q is past r + 1/2
            let mut r = 0i64;
            while (r + 1) * (r + 1) <= q {
                r += 1;
            }
            if 4 * q > (2 * r + 1) * (2 * r + 1) {
                r += 1;
            }
            t[u * 32 + v] = r as u8;
            v += 1;
        }
        u += 1;
    }
    t
}

/// sqrt(u * u + v * v), from the table when both are small.
fn dist(u: i32, v: i32) -> i32 {
    let (u, v) = (u.abs(), v.abs());
    if u < 32 && v < 32 {
        DIST[(u * 32 + v) as usize] as i32
    } else {
        isqrt((u as i64) * (u as i64) + (v as i64) * (v as i64)) as i32
    }
}

fn isqrt(v: i64) -> i64 {
    if v <= 0 {
        return 0;
    }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// The coverage (0 to 255) of pixel (i, j) of a corner of size k, counted inward from its outer edges, in a style.
pub(in super::super) fn cov(style: Corners, k: i32, i: i32, j: i32) -> u8 {
    if k <= 0 || i >= k || j >= k {
        return 255;
    }
    if i < 0 || j < 0 {
        return 0;
    }
    match style {
        Corners::Hard => {
            if i + j >= k {
                255
            } else {
                0
            }
        }
        Corners::Cut => {
            // the edge is the diagonal i + j = k: the pixel it crosses corner to corner is half covered
            if i + j >= k {
                255
            } else if i + j == k - 1 {
                128
            } else {
                0
            }
        }
        Corners::Round => {
            if (k as usize) <= MAXR {
                ROUND[off(k as usize) + (j * k + i) as usize]
            } else {
                let edge = big_edge(k, j);
                let (p0, p1) = (i as i64 * 256, i as i64 * 256 + 256);
                if edge <= p0 {
                    255
                } else if edge >= p1 {
                    0
                } else {
                    ((p1 - edge) * 255 / 256) as u8
                }
            }
        }
    }
}

/// Where a big circle (radius k, past the table) crosses the middle of row j, in 1/256 pixel from the outer edge.
/// Kept for the last row asked, as a corner's pixels are asked for row by row. Mainline only.
fn big_edge(k: i32, j: i32) -> i64 {
    static mut LAST: (i32, i32, i64) = (0, 0, 0);
    // SAFETY: mainline only.
    unsafe {
        if LAST.0 == k && LAST.1 == j && k > 0 {
            return LAST.2;
        }
        let (r, y) = (k as i64 * 256, (k - j) as i64 * 256 - 128);
        let e = r - isqrt(r * r - y * y);
        LAST = (k, j, e);
        e
    }
}

/// The first column of row j (counted inward) of a corner of size k that is fully covered.
pub(in super::super) fn inset(style: Corners, k: i32, j: i32) -> i32 {
    if k <= 0 || j >= k || j < 0 {
        return 0;
    }
    match style {
        Corners::Hard | Corners::Cut => k - j,
        Corners::Round if k as usize > MAXR => ((big_edge(k, j) + 255) / 256) as i32,
        Corners::Round => {
            let mut i = 0;
            while i < k && cov(style, k, i, j) < 255 {
                i += 1;
            }
            i
        }
    }
}

/// The corner (size, row counted inward) on each side of row j of a shape h rows high: (left, right). A side with no
/// corner on this row gives size 0. Where two corners on a side overlap (a shape shorter than both), the deeper wins.
fn corners_at(j: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32) -> ((i32, i32), (i32, i32)) {
    let pick = |top: i32, bot: i32| {
        let a = if j < top { (top, j) } else { (0, 0) };
        let b = if h - 1 - j < bot { (bot, h - 1 - j) } else { (0, 0) };
        let st = corners();
        if inset(st, a.0, a.1) >= inset(st, b.0, b.1) { a } else { b }
    };
    (pick(tl, bl), pick(tr, br))
}

/// The fully covered insets of row j, left and right (what `cut_inset` gives, for every style).
pub(in super::super) fn insets(j: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32) -> (i32, i32) {
    let ((kl, jl), (kr, jr)) = corners_at(j, h, tl, tr, br, bl);
    let st = corners();
    (inset(st, kl, jl), inset(st, kr, jr))
}

/// One pixel of colour c at coverage a (of 255), clipped as `rect` clips.
pub(in super::super) fn px(x: i32, y: i32, a: u8, c: u32) {
    if a == 255 {
        rect(x, y, 1, 1, c);
    } else if a > 0 {
        mix1(x, y, a, c);
    }
}

/// A rectangle of colour c at alpha a (of 255) over what is there, clipped as `rect` clips.
pub(in super::super) fn fill(x: i32, y: i32, w: i32, h: i32, c: u32, a: u8) {
    if w <= 0 || h <= 0 || a == 0 {
        return;
    }
    if a == 255 {
        rect(x, y, w, h, c);
        return;
    }
    if c == 0 {
        // black (the shadows' straight runs): the two-pixels-a-step darkening, clipped as `rect` clips (no app body
        // draws a shadow, so only the box redrawn and the screen)
        // SAFETY: mainline only.
        let (dx, dy, dw, dh) = unsafe { DCLIP };
        let (x0, y0) = (x.max(dx).max(0), y.max(dy).max(0));
        let (x1, y1) = ((x + w).min(dx + dw).min(W()), (y + h).min(dy + dh).min(H()));
        if x1 > x0 && y1 > y0 {
            crate::canvas::shade_const(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, a as u32);
        }
        return;
    }
    // one row of the alpha, read again for every row (stride 0)
    let row = [a; 64];
    let mut cx = x;
    while cx < x + w {
        let n = (x + w - cx).min(64);
        cover_stride(cx, y, n as usize, h as usize, &row[..n as usize], 0, c, &IDENT);
        cx += n;
    }
}

/// The edge pixels of row j: the partly covered pixels of its corners, mixed at their coverage times a.
pub(in super::super) fn edges(x: i32, y: i32, w: i32, h: i32, j: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32, a: u8) {
    let st = corners();
    if st == Corners::Hard {
        return;
    }
    let ((kl, jl), (kr, jr)) = corners_at(j, h, tl, tr, br, bl);
    let (l, r) = (inset(st, kl, jl), inset(st, kr, jr));
    // a Cut corner and a big Round corner have one partly covered pixel a row, just outside the inset
    let first = |k: i32, l: i32| if st == Corners::Cut || (st == Corners::Round && k as usize > MAXR) { (l - 1).max(0) } else { 0 };
    for i in first(kl, l)..l.min(w - r) {
        let v = cov(st, kl, i, jl);
        if v > 0 && v < 255 {
            px(x + i, y + j, (v as u32 * a as u32 / 255) as u8, c);
        }
    }
    for i in first(kr, r)..r.min(w - l) {
        let v = cov(st, kr, i, jr);
        if v > 0 && v < 255 {
            px(x + w - 1 - i, y + j, (v as u32 * a as u32 / 255) as u8, c);
        }
    }
}

/// A filled shape with corners of sizes tl, tr, br, bl in the current style, at alpha a.
pub(in super::super) fn shape(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32, a: u8) {
    let top = tl.max(tr).min(h);
    let bot = (h - br.max(bl)).max(top);
    for j in (0..top).chain(bot..h) {
        let (l, r) = insets(j, h, tl, tr, br, bl);
        fill(x + l, y + j, w - l - r, 1, c, a);
        edges(x, y, w, h, j, tl, tr, br, bl, c, a);
    }
    fill(x, y + top, w, bot - top, c, a); // the straight middle in one fill
}

/// The one-pixel outline of a shape in the current style: the ring between the shape and the shape one pixel in.
pub(in super::super) fn frame(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    let st = corners();
    // the inner shape's coverage at the outer shape's (i, j) of a corner of size k
    let inner = |k: i32, i: i32, j: i32| -> u8 {
        if i < 1 || j < 1 {
            0
        } else if k <= 1 {
            255
        } else {
            cov(st, k - 1, i - 1, j - 1)
        }
    };
    for j in 0..h {
        let ((kl, jl), (kr, jr)) = corners_at(j, h, tl, tr, br, bl);
        let edge_row = j == 0 || j == h - 1;
        // the left corner's pixels, then the straight part, then the right corner's
        if kl > 0 {
            for i in 0..kl.min(w) {
                let v = cov(st, kl, i, jl).saturating_sub(inner(kl, i, jl));
                px(x + i, y + j, v, c);
            }
        } else if !edge_row {
            px(x, y + j, 255, c);
        }
        if kr > 0 {
            for i in 0..kr.min(w - kl.max(0)) {
                let v = cov(st, kr, i, jr).saturating_sub(inner(kr, i, jr));
                px(x + w - 1 - i, y + j, v, c);
            }
        } else if !edge_row {
            px(x + w - 1, y + j, 255, c);
        }
        if edge_row {
            let (l, r) = (kl.max(0), kr.max(0));
            rect(x + l, y + j, w - l - r, 1, c);
        }
    }
}

// ---- Soft shadows: a fixed profile, darkest under the edge, gone 12 pixels out

/// The shadow's reach past the shape and how far down it is moved.
pub(in super::super) const REACH: i32 = 7;
pub(in super::super) const DROP: i32 = 4;
/// Alpha (of 255) at a distance d from the shadow's shape, d = 0 under it: 47% black falling to nothing over REACH
/// (7 pixels: 11 cost twice the mixed pixels for a fade the eye barely tells apart on the dark desk).
pub(in super::super) const PROFILE: [u8; 8] = [120, 98, 76, 56, 38, 23, 11, 4];
pub(in super::super) const SHADE: u32 = 0x00_00_00;

/// How far (whole pixels, 0 inside) the point (px, py) lies outside the shape (x, y, w, h) with corners tl, tr, br,
/// bl in the current style: straight edges by their distance, corners by the diagonal or the circle.
fn outside(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32) -> i32 {
    let (dx, dy) = ((x - px).max(px - (x + w - 1)).max(0), (y - py).max(py - (y + h - 1)).max(0));
    // the corner this point is nearest, and its position counted inward from that corner's outer edges
    let left = px < x + w / 2;
    let up = py < y + h / 2;
    let k = match (left, up) {
        (true, true) => tl,
        (false, true) => tr,
        (false, false) => br,
        (true, false) => bl,
    };
    let i = if left { px - x } else { x + w - 1 - px };
    let j = if up { py - y } else { y + h - 1 - py };
    if k > 0 && i < k && j < k {
        let d = match corners() {
            Corners::Round => dist(k - i, k - j) - k,
            _ => (k - i - j) * 181 / 256, // the distance to the diagonal i + j = k
        };
        return d.max(dx).max(dy).max(0);
    }
    if dx > 0 && dy > 0 {
        dist(dx, dy)
    } else {
        dx.max(dy)
    }
}

/// The alpha of the shadow at a distance d outside its shape.
fn shade(d: i32) -> u8 {
    if d < 0 {
        0
    } else {
        *PROFILE.get(d as usize).unwrap_or(&0)
    }
}

/// A soft shadow for the shape (x, y, w, h) with corners tl, tr, br, bl: the shape moved DROP down and faded out over
/// REACH pixels, drawn only where the shape itself does not cover (it is opaque and drawn over it right after).
///
/// Cheap where it can be (the first build mixed every pixel one call at a time and cost two to three times a frame):
/// - only the rows inside the box being redrawn are visited;
/// - the side strips of the rows between the corners are one block each, at a fade that depends only on the column;
/// - the rows above and below are one fill each, at a fade that depends only on the row;
/// - only the corner squares are worked out pixel by pixel, one blend call per row of each.
pub(in super::super) fn shadow(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, reach: i32) {
    let (sx, sy) = (x, y + DROP);
    // a shorter shadow (an unfocused window's) is the same profile squeezed into `reach` pixels
    let reach = reach.clamp(1, REACH);
    let shade = |d: i32| if d > reach { 0 } else { shade(d * REACH / reach) };
    // SAFETY: mainline only.
    let (cx, cy, cw, ch) = unsafe { DCLIP };
    let (r0, r1) = ((sy - reach).max(cy), (sy + h + reach).min(cy + ch));
    if r0 >= r1 || x + w + reach <= cx || x - reach >= cx + cw {
        return;
    }
    let (kl, kr) = (tl.max(bl).max(0), tr.max(br).max(0));
    let (kt, kb) = (tl.max(tr).max(0), bl.max(br).max(0));
    // the middle rows: below the moved shape's top corners and above the shape's own bottom corners
    let (m0, m1) = ((sy + kt).max(r0), (y + h - kb).min(r1));
    // the side strips of the middle rows: the fade depends only on how far out the column is
    if m1 > m0 {
        let mut left = [0u8; REACH as usize];
        let mut right = [0u8; REACH as usize];
        for i in 0..reach as usize {
            left[i] = shade(reach - i as i32);
            right[i] = shade(i as i32 + 1);
        }
        cover_stride(x - reach, m0, reach as usize, (m1 - m0) as usize, &left[..reach as usize], 0, SHADE, &IDENT);
        cover_stride(x + w, m0, reach as usize, (m1 - m0) as usize, &right[..reach as usize], 0, SHADE, &IDENT);
        // where a middle row lies below the shape, the shadow is under the whole width
        let b0 = m0.max(y + h);
        if m1 > b0 {
            fill(x, b0, w, m1 - b0, SHADE, shade(0));
        }
    }
    // the corner bands: the rows from the top of the shadow to its top corners' end, and from the shape's own bottom
    // corners to the foot of the shadow
    let in_shape = |px: i32, py: i32| px >= x && px < x + w && py >= y && py < y + h && outside(px, py, x, y, w, h, tl, tr, br, bl) == 0;
    let mut zone = |z0: i32, z1: i32, py: i32| {
        let (z0, z1) = (z0.max(cx), z1.min(cx + cw));
        let mut a = [0u8; 64];
        let mut px = z0;
        while px < z1 {
            let n = (z1 - px).min(64);
            for k in 0..n {
                let q = px + k;
                a[k as usize] = if in_shape(q, py) { 0 } else { shade(outside(q, py, sx, sy, w, h, tl, tr, br, bl)) };
            }
            cover(px, py, n as usize, 1, &a[..n as usize], SHADE, &IDENT);
            px += n;
        }
    };
    for py in (r0..m0.max(r0).min(r1)).chain(m1.max(r0).max(m0)..r1) {
        let row_d = (sy - py).max(py - (sy + h - 1)).max(0);
        // the corner zones, left and right: from the shadow's reach outside to one past the deeper corner inside
        let zl1 = (sx + kl + 1).min(sx + w / 2);
        let zr0 = (sx + w - 1 - kr).max(sx + w / 2);
        zone(sx - reach, zl1, py);
        zone(zr0, sx + w + reach, py);
        // between them: covered by the shape on its own rows, else one fade across
        if zr0 > zl1 && !(py >= y && py < y + h) {
            fill(zl1, py, zr0 - zl1, 1, SHADE, shade(row_d));
        }
    }
}

fn px_shade(x: i32, y: i32, a: u8) {
    if a > 0 {
        cover(x, y, 1, 1, &[a], SHADE, &IDENT);
    }
}

/// The box a shadow can touch for the shape (x, y, w, h): what damage must cover (the architect: grown boxes
/// clamped with saturating maths; `damage` clips them to the screen).
pub(in super::super) fn shadow_box(x: i32, y: i32, w: i32, h: i32) -> Rect {
    (x.saturating_sub(REACH), (y + DROP).saturating_sub(REACH), w.saturating_add(2 * REACH), h.saturating_add(2 * REACH))
}

/// A hash of the Round table for radius r (the suite: the same bytes on ARM and PC).
pub(in super::super) fn round_hash(r: usize) -> u64 {
    let r = r.clamp(1, MAXR);
    ROUND[off(r)..off(r) + r * r].iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}
