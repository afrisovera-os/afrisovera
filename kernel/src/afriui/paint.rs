//! The Sovereign UI's paint (docs/design/afriui.md section 14): the theme (palette,
//! cloths, cut-corner primitives, text, icons) and every drawing function of the PC desktop.
//! It reads the model in desk.rs (`use super::*`: the window state, geometry and app data) and
//! never changes it, except for its own frame count and timing; the model never draws. When the
//! phone moves over, it keeps the model and gets its paint here (afriui.md section 9).

use super::*;

#[path = "face.rs"]
mod face; // the Sovereign face (afriui.md section 16)

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
const CLOTH: [[u32; 3]; 4] = [[OCHRE, NIGHT, GOLD], [GREEN, NIGHT, GOLD], [INDIGO, TEXT, GOLD], [GOLD, RED, GREEN]];

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
    let up = |c: i32| if j < c { c - j } else { 0 };
    let down = |c: i32| if h - 1 - j < c { c - (h - 1 - j) } else { 0 };
    (up(tl).max(down(bl)), up(tr).max(down(br)))
}

/// A filled shape with cut corners.
fn cut(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    let top = tl.max(tr).min(h);
    let bot = (h - br.max(bl)).max(top);
    for j in (0..top).chain(bot..h) {
        let (l, r) = cut_inset(j, h, tl, tr, br, bl);
        rect(x + l, y + j, w - l - r, 1, c);
    }
    rect(x, y + top, w, bot - top, c); // the straight middle in one fill
}

/// The one-pixel outline of a cut shape.
fn cut_frame(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
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

/// Proportional text in the Sovereign face (afriui.md section 16): each glyph advances
/// by its own width plus one pixel. (x0, y0) is the top of the capitals; descenders hang
/// below the baseline (face::LINE_H rows in all). Returns the end x.
fn text(x0: i32, y0: i32, s: &[u8], c: u32, scale: i32) -> i32 {
    // a line wholly above or below the box being redrawn: only its width is needed
    // SAFETY: mainline only.
    let (_, dy, _, dh) = unsafe { DCLIP };
    if !s.is_empty() && (y0 >= dy + dh || y0 + face::LINE_H * scale <= dy) {
        return x0 + text_w(s, scale) + scale;
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
    let w: i32 = s.iter().map(|&b| face::glyph(b).0 + 1).sum();
    ((w - 1) * scale).max(0)
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
    for i in 1..=NAPPS {
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
        under.s(b"UTC");
    } else {
        let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
        l.u(h12).s(b":").two(mm);
        under.s(if hh < 12 { b"am" } else { b"pm" });
    }
    s.last_minute = secs / 60;
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
    for i in 1..=NAPPS {
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
        under.s(b"UTC");
    } else {
        let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
        l.u(h12).s(b":").two(mm);
        under.s(if hh < 12 { b"am" } else { b"pm" });
    }
    s.last_minute = secs / 60;
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
    text(px + 10, py + 2, b"AC", DIM, 1);
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
    if !s.touch {
        // a hard relief shadow, down and right, as a carving's
        cut(x + 4, y + 4, w, h, k, 0, k, 0, SHADOW);
    }
    cut(x, y, w, h, k, 0, k, 0, if focus { SURFACE } else { SURFACE_OFF });
    // the lintel (title bar)
    for j in 0..th {
        let (l, r) = ins(j);
        rect(x + l, y + j, w - l - r, 1, if focus { TITLE_ON } else { TITLE_OFF });
    }
    rect(x, y + th, w, 1, LINE);
    let band = 6; // the cloth: three woven rows (4 px was too thin to read focus at a glance, review S1)
    let isz = if s.touch { 16 } else { 12 };
    icon(Some(APPS[a]), x + 12, y + band + (th - band - isz) / 2, isz);
    let ts = if s.touch { 2 } else { 1 };
    text(x + 12 + isz + 8, y + band + (th - band - face::CAP * ts) / 2, APPS[a].name().as_bytes(), if focus { TEXT } else { DIM }, ts);
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
    }
    // SAFETY: as above.
    unsafe { CLIP = None };
    // the outline, then the cloth along the top of the focused window's lintel
    cut_frame(x, y, w, h, k, 0, k, 0, if focus { GOLD_DEEP } else { LINE }); // review S1: gold-dark was too muddy
    if focus {
        weave(x, y, w, band / 2, &CLOTH[a], &ins);
    }
}

fn files_app(a: usize, x: i32, y: i32, w: i32, h: i32) {
    let s = st();
    let split = files_split(a);
    rect(x, y, split, 20, SURFACE2);
    text(x + 10, y + 6, b"Name", DIM, 1);
    text(x + split - 44, y + 6, b"Size", DIM, 1);
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
        text(x + 10, y + 30, b"No files yet", DIM, 1);
    }
    // the preview pane
    let (px, pw) = (x + split + 1, w - split - 1);
    match s.files_sel {
        Some(i) if crate::vfs::is_used(i) => {
            let name = crate::vfs::name_of(i);
            text(px + 10, y + 8, name.as_bytes(), GOLD, 1);
            let mut l = Line::new();
            l.u(crate::vfs::size_of(i) as u64).s(b" bytes");
            text(px + 10, y + 20, l.get(), DIM, 1);
            rect(px + 10, y + 32, pw - 20, 1, LINE);
            let data = crate::vfs::read_raw(name).unwrap_or(&[]);
            let printable = data.iter().take(512).all(|&c| c == b'\n' || c == b'\r' || c == b'\t' || (0x20..0x7f).contains(&c));
            if name.ends_with(".wasm") {
                text(px + 10, y + 42, b"WebAssembly program", TEXT, 1);
                text(px + 10, y + 56, b"Run it from Terminal:", DIM, 1);
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
                text(px + 10, y + 42, b"Binary data", TEXT, 1);
            }
        }
        _ => {
            let m = b"Select a file";
            text(px + (pw - text_w(m, 1)) / 2, y + h / 2 - 4, m, DIM, 1);
        }
    }
}

fn terminal_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, TERM_BG);
    let cols = ((w - 16) / 6).max(1) as usize;
    let rows = ((h - 12) / 13).max(1) as usize;
    // the newest lines that are not desktop log lines, bottom up
    let mut shown = [0usize; TL];
    let mut n = 0;
    let mut k = 0;
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
        if j == 0 {
            // the cursor after the line being typed
            let cx = x + 8 + (line.len().min(cols) as i32) * 6;
            rect(cx + 1, yy, 5, face::CAP, GOLD);
        }
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
    head(y + 10, b"Display");
    let rows = settings_rows();
    let mut l = Line::new();
    l.u(W() as u64).s(b" x ").u(H() as u64).s(b" logical pixels");
    text(x + 14, y + 24, l.get(), DIM, 1);
    // input profile: a two-part switch
    label(y + rows[0] + 5, b"Input");
    let sx = x + settings_seg_x(a);
    chip(sx, y + rows[0], 128, 20, 4, SURFACE2);
    let on = if s.touch { 64 } else { 0 };
    chip(sx + on + 2, y + rows[0] + 2, 60, 16, 3, GOLD_DEEP);
    text(sx + 10, y + rows[0] + 6, b"Pointer", if s.touch { DIM } else { NIGHT }, 1);
    text(sx + 64 + 14, y + rows[0] + 6, b"Touch", if s.touch { NIGHT } else { DIM }, 1);

    head(y + 70, b"Desktop");
    label(y + rows[1] + 3, b"Monument wallpaper");
    switch(x + w - 50, y + rows[1], s.wallpaper);
    label(y + rows[2] + 3, b"24-hour clock");
    switch(x + w - 50, y + rows[2], s.clock24);
    label(y + rows[3] + 3, b"Motion");
    switch(x + w - 50, y + rows[3], s.motion);

    head(y + 164, b"Network");
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(b"No network device");
    } else if ip == [0u8; 4] {
        l.s(b"Link up, no address yet");
    } else {
        rect(x + 14, y + 183, 5, 5, GREEN);
        l.s(b"   Connected  ");
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                l.s(b".");
            }
            l.u(*o as u64);
        }
    }
    label(y + 180, l.get());

    head(y + 204, b"System");
    let mut l = Line::new();
    l.u(crate::cores_online() as u64).s(b" core(s), ").u(crate::ram_mib()).s(b" MiB memory");
    label(y + 220, l.get());
    let mut l = Line::new();
    l.s(b"Booted ").u(crate::config::boots() as u64).s(b" time(s), up ").u(crate::sched::ticks() / 100).s(b" s");
    text(x + 14, y + 234, l.get(), DIM, 1);
    note_uptime(a, y + 234);
    let _ = (w, a);
}

/// The line where Settings or About shows the uptime (it changes every second), as a box on
/// the desktop; the model damages only that line once a second (afriui.md section 15).
/// Each app records where it last drew its uptime line, as rows below its window's top
/// (architect's note N6: the place comes from the drawing, not a copy of its layout); -1 until
/// the app has been drawn, and then the whole window is the line's box.
static mut UPTIME_DY: [i32; NAPPS] = [-1; NAPPS];

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
    let sub = b"Own your device - own your future";
    text(x + (w - text_w(sub, 1)) / 2, yy, sub, GOLD_DEEP, 1);
    yy += 18;
    rect(x + 24, yy, w - 48, 1, LINE);
    yy += 10;
    let arch: &[u8] = if cfg!(target_arch = "aarch64") { b"ARM64 (phone)" } else { b"x86-64 (PC)" };
    let mut rows: [(&[u8], Line); 4] = [(b"Kernel", Line::new()), (b"Desktop", Line::new()), (b"Device", Line::new()), (b"Memory", Line::new())];
    rows[0].1.s(b"Mansa microkernel, Rust");
    rows[1].1.s(b"Tonga, Sovereign UI");
    rows[2].1.s(arch).s(b", ").u(crate::cores_online() as u64).s(b" core(s)");
    rows[3].1.u((crate::frame::total_frames() * 4 / 1024) as u64).s(b" MiB, up ").u(crate::sched::ticks() / 100).s(b" s");
    for (k, v) in rows.iter() {
        if yy + face::LINE_H > y + h {
            break;
        }
        text(x + 30, yy, k, DIM, 1);
        text(x + 100, yy, v.get(), TEXT, 1);
        if *k == b"Memory" {
            note_uptime(App::About as usize, yy); // the uptime is on this row
        }
        yy += 14;
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
    cut(lx + 4, ly + 4, lw, lh, k, k, k, k, SHADOW);
    cut(lx, ly, lw, lh, k, k, k, k, 0x16_11_0E);
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
    for i in 0..NAPPS {
        let (tx, ty, ts) = launcher_tile(i);
        if i == s.sel {
            // the keyboard walk's choice: a gold cut frame round the tile (section 14)
            cut_frame(tx - 4, ty - 4, ts + 8, ts + 8, 6, 6, 6, 6, GOLD);
            cut_frame(tx - 3, ty - 3, ts + 6, ts + 6, 5, 5, 5, 5, GOLD);
        }
        icon(Some(APPS[i]), tx, ty, ts);
        let n = APPS[i].name().as_bytes();
        text(tx + (ts - text_w(n, 1)) / 2, ty + ts + 6, n, if s.wins[i].open { GOLD } else { TEXT }, 1);
    }
    // honest use of the space: what this device is doing now (review D2)
    let (ix, iy, iw, ih) = circle_info_box();
    chip(ix, iy, iw, ih, 6, 0x16_11_0E);
    cut_frame(ix, iy, iw, ih, 6, 6, 6, 6, LINE);
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(b"No network");
    } else if ip == [0u8; 4] {
        l.s(b"Link up, no address");
    } else {
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                l.s(b".");
            }
            l.u(*o as u64);
        }
    }
    l.s(b",  ").u(crate::ram_mib()).s(b" MiB,  ");
    l.u((0..crate::vfs::MAX_FILES).filter(|&i| crate::vfs::is_used(i)).count() as u64).s(b" files");
    let x0 = text(ix + 14, iy + 10, b"This device", GOLD_DEEP, 1);
    text(x0 + 10, iy + 10, l.get(), TEXT, 1);
    let mut l = Line::new();
    l.s(b"Up ").u(crate::sched::ticks() / 100).s(b" s.  Super or Esc closes the Circle.");
    text(ix + 14, iy + 26, l.get(), DIM, 1);
}

fn notes_panel() {
    let s = st();
    let (nx, ny, nw, nh) = notes_box();
    cut(nx + 4, ny + 4, nw, nh, CUT, 0, CUT, 0, SHADOW);
    cut(nx, ny, nw, nh, CUT, 0, CUT, 0, 0x16_11_0E);
    cut_frame(nx, ny, nw, nh, CUT, 0, CUT, 0, GOLD_DARK);
    text(nx + 14, ny + 10, b"Notifications", GOLD, 1);
    let total = s.nnotes.min(NNOTE);
    if total == 0 {
        text(nx + 14, ny + 32, b"Nothing new", DIM, 1);
    }
    for k in 0..total {
        let i = (s.nnotes - 1 - k) % NNOTE;
        let yy = ny + 28 + k as i32 * 18;
        chip(nx + 14, yy + 2, 4, 4, 1, GOLD_DEEP);
        text(nx + 24, yy, &s.notes[i][..s.note_len[i]], TEXT, 1);
    }
}

/// The toast's box while it shows (shadow included), else None. The model uses it for damage.
pub(super) fn toast_box() -> Option<Rect> {
    if crate::sched::ticks() >= st().toast_until {
        return None;
    }
    toast_place()
}

/// The toast's box while it is set, whether or not its time has run out yet: the model's
/// snapshot uses this, so a toast that expires between two snapshots still damages its box
/// (a partial frame for something else could otherwise leave a strip of it on the screen).
pub(super) fn toast_place() -> Option<Rect> {
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
    Some((x, y, w + 3, 28 + 3))
}

fn toast() {
    let s = st();
    let Some((x, y, w, _)) = toast_box() else { return };
    let w = w - 3;
    let i = (s.nnotes - 1) % NNOTE;
    let msg = &s.notes[i][..s.note_len[i]];
    cut(x + 3, y + 3, w, 28, 6, 0, 6, 0, SHADOW);
    cut(x, y, w, 28, 6, 0, 6, 0, SURFACE2);
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
        if s.notes_open && touches(grow(notes_box(), 4), r) {
            notes_panel();
        }
        if s.launcher && (touches(grow(launcher_box(), 4), r) || touches(grow(circle_info_box(), 4), r)) {
            launcher();
        }
    }
    // SAFETY: as above.
    unsafe { DCLIP = (0, 0, i32::MAX / 2, i32::MAX / 2) };
    s.frames += 1;
    let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    // SAFETY: mainline only.
    unsafe { PART_US = (us, PART_US.1.max(us), PART_US.2 + 1) };
    crate::canvas::mark();
}
