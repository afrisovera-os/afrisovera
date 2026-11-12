//! Framebuffer driver — AfriSovera's first pixels.
//!
//! Configures QEMU's `ramfb` (a simple linear framebuffer) via [`crate::fwcfg`]
//! and draws a splash. `ramfb` scans out directly from a guest-memory buffer we
//! own, so "drawing" is just writing XRGB8888 pixels into that buffer.
//!
//! Phase 1.0 keeps this in the kernel for bring-up. Per the architecture
//! ([`docs/02`]) the display belongs in a **userspace Asili driver** behind the
//! Tonga compositor — that migration is a later Phase 1 milestone. This is the
//! kernel-framebuffer bootstrap that every OS starts with.

use crate::font;
use crate::fwcfg;
use core::ptr::addr_of_mut;

/// The framebuffer's pixels, WIDTH to a row (the PC's screen copy reads them; the phone's
/// desktop canvas is this buffer, kernel/src/afriui/arm_canvas.rs).
pub fn pixels() -> *const u32 {
    core::ptr::addr_of!(FB) as *const u32
}

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;

/// DRM fourcc `XR24` (XRGB8888): memory bytes B,G,R,X — a u32 `0x00RRGGBB`
/// stored little-endian lands correctly.
const FOURCC_XRGB8888: u32 = 0x3432_5258;

#[repr(C, align(4096))]
struct Framebuffer([u32; WIDTH * HEIGHT]);
static mut FB: Framebuffer = Framebuffer([0; WIDTH * HEIGHT]);

/// Fill a rectangle (clipped to the screen). Public so the Tonga compositor can
/// draw surfaces onto the framebuffer.
/// Fast at opt-level 0 (afriui.md section 18): the first row is filled by doubling copies, then copied down; the
/// copies are compiler_builtins' memcpy, which is optimised even in a debug build.
pub fn fill_rect(x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
    let (x1, y1) = (x0.saturating_add(w).min(WIDTH), y0.saturating_add(h).min(HEIGHT));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let n = x1 - x0;
    // SAFETY: rows y0..y1 and columns x0..x1 are inside the framebuffer (clipped above); single writer.
    unsafe {
        let fb = addr_of_mut!(FB) as *mut u32;
        let row = fb.add(y0 * WIDTH + x0);
        row.write(rgb);
        let mut done = 1;
        while done < n {
            let k = done.min(n - done);
            core::ptr::copy_nonoverlapping(row, row.add(done), k);
            done += k;
        }
        for y in y0 + 1..y1 {
            core::ptr::copy_nonoverlapping(row, fb.add(y * WIDTH + x0), n);
        }
    }
}

/// Copy a `w`x`h` pixel block from `src` (row stride `stride`) to (dx, dy).
/// Used by the compositor to blit an app's surface buffer onto the screen.
///
/// Fast at opt-level 0 (afriui.md section 19): the block is clipped to the screen once (saturating, so a huge `w` or
/// `h` cannot wrap), then each row is one memmove (compiler_builtins, optimised even in a debug build); the same
/// pixels as the old per-pixel loop.
///
/// # Safety
/// `src` must be valid for `(h-1)*stride + w` u32 pixels (stride 0 is allowed: every row reads the same `w`
/// pixels), and must not overlap the destination rows on the framebuffer: rows are copied top to bottom, and the
/// memmove only protects within one row.
pub unsafe fn blit(dx: usize, dy: usize, w: usize, h: usize, src: *const u32, stride: usize) {
    let (x1, y1) = (dx.saturating_add(w).min(WIDTH), dy.saturating_add(h).min(HEIGHT));
    if dx >= x1 || dy >= y1 {
        return;
    }
    let (n, rows) = (x1 - dx, y1 - dy);
    let fb = addr_of_mut!(FB) as *mut u32;
    for j in 0..rows {
        // SAFETY: the caller's contract covers src's row j; the destination row is on screen (clipped above).
        core::ptr::copy(src.add(j * stride), fb.add((dy + j) * WIDTH + dx), n);
    }
}

/// Enable/disable console mirroring — used when switching to a graphical
/// (Tonga) surface so shell echo doesn't scribble over the desktop.
/// The text console is on screen (`fbcon wm` refuses otherwise, afriui.md section 20).
pub fn console_shown() -> bool {
    // SAFETY: single scalar read, single-threaded shell context.
    unsafe { CON_ON }
}

pub fn set_console(on: bool) {
    // SAFETY: single scalar write, single-threaded shell context.
    unsafe {
        CON_ON = on;
    }
}

/// Configure `ramfb` and draw the splash. Returns false (gracefully) if no
/// `ramfb` device is present (e.g. the serial-only smoke-test invocation).
pub fn init() -> bool {
    let sel = match fwcfg::find_file("etc/ramfb") {
        Some(s) => s,
        None => {
            crate::kprintln!("[fb] no ramfb device present — skipping graphics");
            return false;
        }
    };

    let fb_pa = addr_of_mut!(FB) as u64;

    // RAMFBCfg: all fields big-endian — addr(8) fourcc(4) flags(4) w(4) h(4) stride(4).
    let mut cfg = [0u8; 28];
    cfg[0..8].copy_from_slice(&fb_pa.to_be_bytes());
    cfg[8..12].copy_from_slice(&FOURCC_XRGB8888.to_be_bytes());
    cfg[12..16].copy_from_slice(&0u32.to_be_bytes()); // flags
    cfg[16..20].copy_from_slice(&(WIDTH as u32).to_be_bytes());
    cfg[20..24].copy_from_slice(&(HEIGHT as u32).to_be_bytes());
    cfg[24..28].copy_from_slice(&((WIDTH * 4) as u32).to_be_bytes()); // stride

    if fwcfg::write_item(sel, &cfg).is_err() {
        crate::kprintln!("[fb] ramfb configuration failed");
        return false;
    }

    console_init();
    crate::kprintln!("[fb] ramfb {}x{} — framebuffer console live", WIDTH, HEIGHT);
    true
}

/// Linearly interpolate two 0xRRGGBB colors (`t` in 0..=100).
fn lerp(a: u32, b: u32, t: usize) -> u32 {
    let ch = |sh: u32| -> u32 {
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        (av + (bv - av) * (t as i32) / 100) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// A premium full-screen brand splash: the AfriSovera gold monument (twin sails
/// + a rising needle, etched) on black, the wordmark, and the purpose line — the
/// OS carrying the brand and what it stands for. Drawn with framebuffer primitives.
pub fn brand_splash() {
    set_console(false);
    let cx = (WIDTH / 2) as i32;

    // Pure black field with the faintest warm lift at the horizon line.
    for y in 0..HEIGHT {
        fill_rect(0, y, WIDTH, 1, lerp(0x05_04_02, 0x00_00_00, y * 100 / HEIGHT));
    }
    // A single, slim, curved gold monument: a thin needle flowing into two sails
    // separated by a fine central seam that closes at the base. The outer edge is
    // concave (narrow waist, flaring foot), which reads far more elegant than a
    // straight triangle.
    let (y_top, y_base) = (56i32, 344i32);
    let span = y_base - y_top;
    for yy in y_top..y_base {
        let t = yy - y_top; // 0..span
        let f = t * 100 / span; // 0..100 progress to the base
        // concave flare: width ~ progress^1.6, scaled to a slim silhouette
        let outer = 6 + (128 * f * f) / 10000 + (30 * f) / 100;
        let seam = (10 * (100 - f)) / 100; // hairline central gap, 0 at the base
        // gold gradient: pale highlight up top -> rich deep gold at the foot
        let gold = lerp(0xFF_EE_BC, 0x9A_67_18, f as usize);
        let y = yy as usize;
        let (lx, lr) = ((cx - outer).max(2), cx - seam);
        if lr > lx {
            fill_rect(lx as usize, y, (lr - lx) as usize, 1, gold);
        }
        let (rx, rr) = (cx + seam, (cx + outer).min(WIDTH as i32 - 2));
        if rr > rx {
            fill_rect(rx as usize, y, (rr - rx) as usize, 1, gold);
        }
    }
    // Fine tip highlight at the very apex.
    fill_rect((cx - 1) as usize, 46, 2, 12, 0xFF_F7_DA);

    // Bright bevelled plinth.
    fill_rect((cx - 150) as usize, y_base as usize, 300, 6, 0xF0_CE_6C);
    fill_rect((cx - 150) as usize, (y_base + 6) as usize, 300, 2, 0x4E_36_10);

    // Wordmark, spaced sub-line, and the purpose line (what the brand stands for).
    let gold_word = 0xF4_CB_57;
    draw_string(196, 360, "AFRISOVERA", gold_word, 4);
    draw_string(214, 400, "O P E R A T I N G   S Y S T E M", 0xC9_9E_44, 2);
    draw_string(190, 438, "OWN YOUR DEVICE   -   OWN YOUR FUTURE", 0x8F_7A_52, 1);

    // Thin gold frame.
    fill_rect(0, 0, WIDTH, 2, gold_word);
    fill_rect(0, HEIGHT - 2, WIDTH, 2, gold_word);
    fill_rect(0, 0, 2, HEIGHT, gold_word);
    fill_rect(WIDTH - 2, 0, 2, HEIGHT, gold_word);
}

// --- Framebuffer text console ----------------------------------------------
// A live, scrolling text region that mirrors the kernel log to the display, so
// the real boot + IPC-demo output shows on screen (see uart::_print's tee).
const CON_X: usize = 12;
const CON_Y: usize = 86;
const CON_SCALE: usize = 1;
const CH: usize = 8 * CON_SCALE;
const CON_COLS: usize = (WIDTH - 2 * CON_X) / CH;
const CON_ROWS: usize = (HEIGHT - CON_Y - 10) / CH;
const CON_BG: u32 = 0x08_0B_12;
const TEXT: u32 = 0xD8_E0_E8;

static mut CCOL: usize = 0;
static mut CROW: usize = 0;
static mut CON_ON: bool = false;

fn console_init() {
    const GOLD: u32 = 0xE0_A0_00;
    const GRAY: u32 = 0x8A_9B_B0;
    fill_rect(0, 0, WIDTH, HEIGHT, 0x0B_0F_1A);
    let b = 6;
    fill_rect(0, 0, WIDTH, b, GOLD);
    fill_rect(0, HEIGHT - b, WIDTH, b, GOLD);
    fill_rect(0, 0, b, HEIGHT, GOLD);
    fill_rect(WIDTH - b, 0, b, HEIGHT, GOLD);

    draw_string(216, 16, "AFRISOVERA OS", GOLD, 2);
    // pan-African tricolor rule under the wordmark
    fill_rect(216, 50, 69, 4, 0xC1_12_1F);
    fill_rect(285, 50, 69, 4, 0x10_10_10);
    fill_rect(354, 50, 70, 4, 0x1B_7A_3D);
    draw_string(CON_X, 64, "MANSA SYSTEM CONSOLE", GRAY, 1);

    // console panel background
    fill_rect(CON_X - 4, CON_Y - 4, CON_COLS * CH + 8, CON_ROWS * CH + 8, CON_BG);
    // SAFETY: single-threaded bring-up.
    unsafe {
        CCOL = 0;
        CROW = 0;
        CON_ON = true;
    }
}

/// Write a string to the framebuffer console (no-op until the console is up).
/// Called from `uart::_print`, so every `kprintln!` also lands on screen.
pub fn console_write(s: &str) {
    crate::desk::term_feed(s); // the desktop's Terminal shows the shell (desk.rs)
    // SAFETY: callers hold IRQs masked (uart::_print), single core → no aliasing.
    unsafe {
        if !CON_ON {
            return;
        }
        for &c in s.as_bytes() {
            con_putc(c);
        }
    }
}

unsafe fn con_putc(c: u8) {
    match c {
        b'\n' => con_newline(),
        b'\r' => {}
        _ => {
            if CCOL >= CON_COLS {
                con_newline();
            }
            con_glyph(CON_X + CCOL * CH, CON_Y + CROW * CH, c);
            CCOL += 1;
        }
    }
}

const _: () = assert!(CON_SCALE == 1, "con_glyph draws 8x8 cells");

/// One console character with its background: each 8-pixel row is built in a small array and copied in once.
unsafe fn con_glyph(x: usize, y: usize, c: u8) {
    let fb = addr_of_mut!(FB) as *mut u32;
    let mut px = [CON_BG; 8];
    for (row, bits) in font::glyph(c).iter().enumerate() {
        for (col, p) in px.iter_mut().enumerate() {
            *p = if (bits >> (7 - col)) & 1 == 1 { TEXT } else { CON_BG };
        }
        // SAFETY: the cell is inside the console region (CCOL < CON_COLS, CROW < CON_ROWS).
        core::ptr::copy_nonoverlapping(px.as_ptr(), fb.add((y + row) * WIDTH + x), 8);
    }
}

unsafe fn con_newline() {
    CCOL = 0;
    CROW += 1;
    if CROW >= CON_ROWS {
        con_scroll();
        CROW = CON_ROWS - 1;
    }
}

/// Scroll the console region up one text row: each pixel row moves with one memmove (afriui.md section 18).
unsafe fn con_scroll() {
    let fb = addr_of_mut!(FB) as *mut u32;
    let w = CON_COLS * CH;
    let h = CON_ROWS * CH;
    for y in 0..(h - CH) {
        let src = (CON_Y + CH + y) * WIDTH + CON_X;
        let dst = (CON_Y + y) * WIDTH + CON_X;
        core::ptr::copy(fb.add(src), fb.add(dst), w);
    }
    fill_rect(CON_X, CON_Y + h - CH, w, CH, CON_BG);
}

/// A hash of console pixel rows `r0..r1` (FNV-1a over the pixels), for `fbcon time`'s scroll check.
unsafe fn con_hash(r0: usize, r1: usize) -> u64 {
    let fb = addr_of_mut!(FB) as *const u32;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in r0..r1 {
        for x in 0..CON_COLS * CH {
            h = (h ^ *fb.add((CON_Y + y) * WIDTH + CON_X + x) as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// `fbcon time` (afriui.md section 18): the time of one console line with a scroll, through the real
/// `kprintln!` path, on the shown console; then the scroll on its own, with two self-checks.
pub fn con_time() {
    // SAFETY: shell mainline; the cursor statics are only also written by prints, and none run in between.
    if !unsafe { CON_ON } {
        crate::kprintln!("[fbcon] the console is not shown (type 'clear' first)");
        return;
    }
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    const LINES: u64 = 8;
    let (mut fast, mut slow) = (u64::MAX, 0u64);
    for k in 0..LINES {
        // SAFETY: as above; the line starts at the bottom row, so its newline scrolls.
        unsafe {
            CROW = CON_ROWS - 1;
            CCOL = 0;
        }
        let t0 = crate::timer::cycles();
        crate::kprintln!("[FBCON] LINE {} OF {}: 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 ABCDEF", k + 1, LINES);
        let t = us(t0);
        fast = fast.min(t);
        slow = slow.max(t);
    }
    // the glyph check: the last line now sits one row above the bottom
    let line = b"[FBCON] LINE 8 OF 8: 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 ABCDEF";
    let n = line.len().min(CON_COLS);
    let mut bad = 0usize;
    let fb = addr_of_mut!(FB) as *const u32;
    for (i, &c) in line[..n].iter().enumerate() {
        let g = font::glyph(c);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8 {
                let want = if (bits >> (7 - col)) & 1 == 1 { TEXT } else { CON_BG };
                let (x, y) = (CON_X + i * CH + col, CON_Y + (CON_ROWS - 2) * CH + row);
                // SAFETY: inside the console region.
                if unsafe { *fb.add(y * WIDTH + x) } != want {
                    bad += 1;
                }
            }
        }
    }
    // the scroll alone, with its check
    let (mut ok, mut scroll) = (0u64, 0u64);
    for _ in 0..LINES {
        // SAFETY: as above.
        unsafe {
            let before = con_hash(1 * CH, CON_ROWS * CH);
            let t0 = crate::timer::cycles();
            con_scroll();
            scroll = scroll.max(us(t0));
            let fbp = addr_of_mut!(FB) as *const u32;
            let mut blank = true;
            for y in (CON_ROWS - 1) * CH..CON_ROWS * CH {
                for x in 0..CON_COLS * CH {
                    blank &= *fbp.add((CON_Y + y) * WIDTH + CON_X + x) == CON_BG;
                }
            }
            if blank && con_hash(0, (CON_ROWS - 1) * CH) == before {
                ok += 1;
            }
        }
    }
    crate::kprintln!("[fbcon] one line with a scroll: fastest {} us, slowest {} us over {} lines; the scroll alone: {} us", fast, slow, LINES, scroll);
    crate::kprintln!("[fbcon] scroll check: {} of {} scrolls moved every row up one text row", ok, LINES);
    crate::kprintln!("[fbcon] glyph check: {} chars, {} pixels differ", n, bad);
}

/// `fbcon blit` (afriui.md section 19): the time of a console-sized and a whole-screen `blit`, from one stack row
/// with stride 0 (no statics), then a copy check and a clip check; the console is redrawn after.
pub fn blit_time() {
    // SAFETY: shell mainline; nothing else draws while the verb runs.
    if !unsafe { CON_ON } {
        crate::kprintln!("[fbblit] the console is not shown (type 'clear' first)");
        return;
    }
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    let mut row = [0u32; WIDTH];
    for (i, p) in row.iter_mut().enumerate() {
        *p = 0x10_20_30 + i as u32;
    }
    let (cw, chh) = (CON_COLS * CH, CON_ROWS * CH);
    let (mut con, mut full) = (u64::MAX, u64::MAX);
    for _ in 0..8 {
        let t0 = crate::timer::cycles();
        // SAFETY: stride 0 reads the same `cw` pixels of `row` for every row.
        unsafe { blit(CON_X, CON_Y, cw, chh, row.as_ptr(), 0) };
        con = con.min(us(t0));
        let t0 = crate::timer::cycles();
        // SAFETY: as above, WIDTH pixels.
        unsafe { blit(0, 0, WIDTH, HEIGHT, row.as_ptr(), 0) };
        full = full.min(us(t0));
    }
    let fb = addr_of_mut!(FB) as *const u32;
    let px = |x: usize, y: usize| unsafe { *fb.add(y * WIDTH + x) }; // SAFETY: callers pass on-screen points.
    // copy check: 24 x 8 from a 32-stride source, every pixel different
    let mut src = [0u32; 32 * 8];
    for (k, p) in src.iter_mut().enumerate() {
        *p = 0x00_A0_00_00 | k as u32;
    }
    fill_rect(0, 0, WIDTH, HEIGHT, 0);
    // SAFETY: src holds 32 * 8 pixels.
    unsafe { blit(100, 50, 24, 8, src.as_ptr(), 32) };
    let mut right = 0;
    for j in 0..8 {
        for i in 0..24 {
            right += (px(100 + i, 50 + j) == src[j * 32 + i]) as usize;
        }
    }
    // clip check: 64 x 64 put 20 from the right edge and 10 from the bottom; only 20 x 10 fits
    let mut big = [0u32; 64];
    for (i, p) in big.iter_mut().enumerate() {
        *p = 0x00_00_B0_00 | i as u32;
    }
    fill_rect(0, 0, WIDTH, HEIGHT, 0);
    // SAFETY: stride 0 reads `big`'s 64 pixels for every row.
    unsafe { blit(WIDTH - 20, HEIGHT - 10, 64, 64, big.as_ptr(), 0) };
    let mut inside = 0;
    for j in 0..10 {
        for i in 0..20 {
            inside += (px(WIDTH - 20 + i, HEIGHT - 10 + j) == big[i]) as usize;
        }
    }
    let edge = px(WIDTH - 21, HEIGHT - 11) == 0 && px(WIDTH - 21, HEIGHT - 1) == 0 && px(WIDTH - 1, HEIGHT - 11) == 0;
    console_init();
    crate::kprintln!("[fbblit] console block {}x{}: fastest {} us; whole screen {}x{}: fastest {} us", cw, chh, con, WIDTH, HEIGHT, full);
    crate::kprintln!("[fbblit] copy check: {} of 192 pixels right; clip check: {} of 200 inside, edge {}", right, inside, if edge { "unchanged" } else { "CHANGED" });
}

/// Draw a single glyph at scale, transparent background (only lit pixels drawn).
pub fn draw_char(x0: usize, y0: usize, c: u8, fg: u32, scale: usize) {
    let g = font::glyph(c);
    for (row, bits) in g.iter().enumerate() {
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 == 1 {
                fill_rect(x0 + col * scale, y0 + row * scale, scale, scale, fg);
            }
        }
    }
}

/// Draw a **proportionally-spaced** string (variable glyph widths), returning the
/// x just past the last glyph. Each 8x8 glyph is measured for its actual ink width
/// so 'I' is narrow and 'W' wide — a premium look from the fixed bitmap font. Used
/// for UI (store/desktop); the console stays monospace via `draw_char`.
pub fn draw_string(x0: usize, y0: usize, s: &str, fg: u32, scale: usize) -> usize {
    let mut x = x0;
    for &b in s.as_bytes() {
        if b == b' ' {
            x += 4 * scale;
            continue;
        }
        let g = font::glyph(b);
        // Measure ink bounds (leftmost/rightmost lit column).
        let mut l = 8usize;
        let mut r = 0usize;
        for &bits in g.iter() {
            for col in 0..8usize {
                if (bits >> (7 - col)) & 1 == 1 {
                    if col < l {
                        l = col;
                    }
                    if col > r {
                        r = col;
                    }
                }
            }
        }
        if r < l {
            x += 4 * scale; // blank glyph
            continue;
        }
        for (row, bits) in g.iter().enumerate() {
            for col in l..=r {
                if (bits >> (7 - col)) & 1 == 1 {
                    fill_rect(x + (col - l) * scale, y0 + row * scale, scale, scale, fg);
                }
            }
        }
        x += (r - l + 2) * scale; // ink width + 1px gap
    }
    x
}

/// Redraw the console header + clear the log region (`fbcon wm` uses it, afriui.md section 20).
pub fn console_reset() {
    console_init();
}
