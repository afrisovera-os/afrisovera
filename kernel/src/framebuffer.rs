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
use core::ptr::addr_of_mut; #[cfg(not(target_arch = "aarch64"))] pub fn pixels() -> *const u32 { core::ptr::addr_of!(FB) as *const u32 }

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;

/// DRM fourcc `XR24` (XRGB8888): memory bytes B,G,R,X — a u32 `0x00RRGGBB`
/// stored little-endian lands correctly.
const FOURCC_XRGB8888: u32 = 0x3432_5258;

#[repr(C, align(4096))]
struct Framebuffer([u32; WIDTH * HEIGHT]);
static mut FB: Framebuffer = Framebuffer([0; WIDTH * HEIGHT]);

#[inline]
fn put(x: usize, y: usize, rgb: u32) {
    if x < WIDTH && y < HEIGHT {
        // SAFETY: bounds-checked index into the framebuffer; single writer.
        unsafe {
            (addr_of_mut!(FB) as *mut u32).add(y * WIDTH + x).write(rgb);
        }
    }
}

/// Fill a rectangle (clipped to the screen). Public so the Tonga compositor can
/// draw surfaces onto the framebuffer.
pub fn fill_rect(x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
    for y in y0..(y0 + h).min(HEIGHT) {
        for x in x0..(x0 + w).min(WIDTH) {
            put(x, y, rgb);
        }
    }
}

/// Copy a `w`x`h` pixel block from `src` (row stride `stride`) to (dx, dy).
/// Used by the compositor to blit an app's surface buffer onto the screen.
///
/// # Safety
/// `src` must point to at least `stride*h` valid u32 pixels.
pub unsafe fn blit(dx: usize, dy: usize, w: usize, h: usize, src: *const u32, stride: usize) {
    for j in 0..h {
        for i in 0..w {
            put(dx + i, dy + j, *src.add(j * stride + i));
        }
    }
}

/// Enable/disable console mirroring — used when switching to a graphical
/// (Tonga) surface so shell echo doesn't scribble over the desktop.
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
            let x = CON_X + CCOL * CH;
            let y = CON_Y + CROW * CH;
            fill_rect(x, y, CH, CH, CON_BG);
            draw_char(x, y, c, TEXT, CON_SCALE);
            CCOL += 1;
        }
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

/// Scroll the console region up one text row by moving pixels.
unsafe fn con_scroll() {
    let fb = addr_of_mut!(FB) as *mut u32;
    let w = CON_COLS * CH;
    let h = CON_ROWS * CH;
    for y in 0..(h - CH) {
        let src = (CON_Y + CH + y) * WIDTH + CON_X;
        let dst = (CON_Y + y) * WIDTH + CON_X;
        for x in 0..w {
            fb.add(dst + x).write(*fb.add(src + x));
        }
    }
    fill_rect(CON_X, CON_Y + h - CH, w, CH, CON_BG);
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

/// Redraw the console header + clear the log region (used by a future `clear`).
#[allow(dead_code)]
pub fn console_reset() {
    console_init();
}
