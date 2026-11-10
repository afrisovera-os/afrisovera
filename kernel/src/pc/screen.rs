//! The PC's screen after boot (5 Nov 2026), for PC step 4: what the shared framebuffer
//! holds -- the shell's text console and the Tonga windows, 640x480, drawn by the same
//! `framebuffer.rs` the ARM kernel shows through QEMU's ramfb -- is copied onto the UEFI
//! GOP framebuffer, centred and scaled by a whole number where the mode is big enough.
//! A real laptop has no serial port; this is what makes the shell visible on one.
//!
//! The copy is driven by change, not by time: every console write and every window present
//! marks the screen dirty (`mark`), and the local-APIC timer copies a dirty screen at most
//! once every few ticks (`tick`), so an idle shell costs nothing.
//!
//! With a USB mouse (xhci.rs) its pointer is drawn on top, on the GOP screen only: the
//! shared framebuffer underneath is never written, so every redraw puts back what it covered.

use core::ptr::{addr_of_mut, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

const W: usize = crate::framebuffer::WIDTH;
const H: usize = crate::framebuffer::HEIGHT;

#[derive(Clone, Copy)]
struct Gop {
    base: u64,
    w: usize,
    h: usize,
    stride: usize,
    bgr: bool,
}
static mut GOP: Gop = Gop { base: 0, w: 0, h: 0, stride: 0, bgr: true };
static DIRTY: AtomicBool = AtomicBool::new(false);
/// The Tonga windows are what the screen shows (`wm`, `edit`); `clear` ends it.
static WM_VIEW: AtomicBool = AtomicBool::new(false);
/// A dragged window moved: recomposite the windows before the next redraw.
static WM_DIRTY: AtomicBool = AtomicBool::new(false);

/// The Tonga desktop (`gui`) is what the screen shows: its dock answers mouse clicks.
static DESKTOP: AtomicBool = AtomicBool::new(false);
pub fn set_desktop(on: bool) {
    DESKTOP.store(on, Ordering::Relaxed);
}
pub fn desktop() -> bool {
    DESKTOP.load(Ordering::Relaxed)
}

pub fn set_wm_view(on: bool) {
    WM_VIEW.store(on, Ordering::Relaxed);
}
pub fn wm_view() -> bool {
    WM_VIEW.load(Ordering::Relaxed)
}
/// A window moved (mouse drag): recompose at the next redraw, not in every mouse report.
pub fn wm_moved() {
    WM_DIRTY.store(true, Ordering::Relaxed);
    mark();
}
static ON: AtomicBool = AtomicBool::new(false);
static mut PRESENTS: u64 = 0;
/// One scaled screen row (up to 4x the 640-pixel console).
static mut LINE: [u32; W * 4] = [0; W * 4];

/// How the 640x480 console is laid on a bigger screen (field item F9b, HP ENVY, 9 Nov 2026).
/// The shell and the desktop are drawn into the shared 640x480 framebuffer (the ARM image's
/// too), so "edge to edge" on the PC means stretching that frame across the GOP mode:
/// - `whole` (the default): a whole-number scale, centred, every console pixel the same size;
///   a 1920x1080 laptop shows 1280x960 with dark bands all round;
/// - `fit`: the largest size that keeps the 4:3 shape (1440x1080 there): top to bottom,
///   bands only at the sides; some pixel columns and rows are drawn one wider than others;
/// - `fill`: the whole screen (1920x1080), stretched wider than tall.
/// `screen fit` / `screen fill` / `screen whole` choose; nothing is saved across a restart.
const WHOLE: u8 = 0;
const FIT: u8 = 1;
const FILL: u8 = 2;
static MODE: AtomicU8 = AtomicU8::new(WHOLE);
/// The console's place on the screen for the current mode: left, top, width, height.
static mut PLACE: (usize, usize, usize, usize) = (0, 0, W, H);
/// For `fit`/`fill`: the console column each screen column of the placed area shows.
static mut XMAP: [u16; W * 4] = [0; W * 4];
/// The colour of the bands around the console (painted once at boot by main.rs, too).
pub const FRAME: u32 = 0x0A_0E14;

/// Take over the firmware's GOP framebuffer (its address, mode and pixel order).
pub fn attach(base: u64, w: usize, h: usize, stride: usize, bgr: bool) {
    if base == 0 || w < W || h < H {
        crate::kprintln!("[pc] screen: GOP mode {}x{} is smaller than the {}x{} console -- serial only", w, h, W, H);
        return;
    }
    // a real machine may put the framebuffer above the 4 GiB the kernel maps 1:1
    if !crate::mmu::map_mmio(base, (stride * h * 4) as u64) {
        crate::kprintln!("[pc] screen: could not map the GOP framebuffer at {:#x} -- serial only", base);
        return;
    }
    // SAFETY: boot, single core, before the timer reads it.
    unsafe { GOP = Gop { base, w, h, stride, bgr } };
    place(MODE.load(Ordering::Relaxed));
    ON.store(true, Ordering::Relaxed);
    let s = scale(w, h);
    crate::kprintln!("[pc] screen: the {}x{} console shown on the {}x{} GOP screen at {}x", W, H, w, h, s);
}

fn scale(w: usize, h: usize) -> usize {
    (w / W).min(h / H).clamp(1, 4)
}

/// Work out where the console goes on the attached screen in mode `m` (and its column map).
fn place(m: u8) {
    // SAFETY: GOP is set at attach; PLACE/XMAP are written here only, at boot or from the
    // shell, and read by `present`; a redraw that overlaps a change shows one mixed frame at
    // worst, and the change marks the screen so the next redraw is whole.
    unsafe {
        let g = GOP;
        let (dw, dh) = match m {
            FIT if g.w * H >= g.h * W => (W * g.h / H, g.h), // wider than 4:3: full height
            FIT => (g.w, H * g.w / W),                        // taller: full width
            FILL => (g.w, g.h),
            _ => {
                let s = scale(g.w, g.h);
                (W * s, H * s)
            }
        };
        // LINE holds one screen row of at most 4x the console; a screen wider than 2560
        // (or taller than 1920) gets at most that size, centred
        let (dw, dh) = (dw.min(W * 4), dh.min(H * 4));
        let x = addr_of_mut!(XMAP) as *mut u16;
        for dx in 0..dw {
            *x.add(dx) = (dx * W / dw) as u16;
        }
        PLACE = ((g.w - dw) / 2, (g.h - dh) / 2, dw, dh);
    }
}

fn mode_name(m: u8) -> &'static str {
    match m {
        FIT => "fit",
        FILL => "fill",
        _ => "whole",
    }
}

/// `screen`, `screen fit|fill|whole`: show or choose how the console fills the screen.
pub fn command(arg: &str) {
    let m = match arg.trim() {
        "" => {
            info();
            crate::kprintln!("screen: modes: whole (sharp, whole-number scale), fit (full height, 4:3 kept), fill (whole screen, stretched); type: screen fit");
            return;
        }
        "whole" => WHOLE,
        "fit" => FIT,
        "fill" => FILL,
        other => {
            crate::kprintln!("screen: unknown mode {}; use: screen whole | screen fit | screen fill", other);
            return;
        }
    };
    if !ON.load(Ordering::Relaxed) {
        crate::kprintln!("screen: no GOP framebuffer, so nothing to fit (serial console only)");
        return;
    }
    MODE.store(m, Ordering::Relaxed);
    place(m);
    clear(FRAME); // the bands around the console change size: paint them afresh
    mark();
    // SAFETY: scalar read of the placement just made.
    let (ox, oy, dw, dh) = unsafe { PLACE };
    crate::kprintln!("screen: mode {}: the 640x480 console now covers {}x{} at {},{} of the screen", mode_name(m), dw, dh, ox, oy);
}

/// Something drew into the shared framebuffer.
pub fn mark() {
    if ON.load(Ordering::Relaxed) {
        DIRTY.store(true, Ordering::Relaxed);
    }
}

/// From the local-APIC timer: show a dirty framebuffer, at most every 4th tick.
pub fn tick(ticks: u64) {
    if ticks % 4 == 0 && DIRTY.swap(false, Ordering::Relaxed) {
        if WM_DIRTY.swap(false, Ordering::Relaxed) && wm_view() {
            crate::tonga::win_present();
        }
        present();
    }
}

/// Fill the WHOLE GOP screen with one colour (field item F9, 8 Nov 2026). The console and
/// the desktop are the 640x480 shared framebuffer drawn centred and whole-number-scaled, so
/// a bigger screen keeps a margin; on the owner's HP ENVY it still showed the boot title and
/// the green boot ground around the window. This paints a clean frame across the whole
/// screen once, before the console is first presented; `present` only ever rewrites the
/// centred window, so the frame stays put with no per-redraw cost.
pub fn clear(rgb: u32) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: GOP set once at attach; writes stay inside the mode the firmware reported.
    unsafe {
        let g = GOP;
        let v = if g.bgr { rgb } else { (rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16) & 0xFF };
        let dst = g.base as *mut u32;
        for y in 0..g.h {
            let out = dst.add(y * g.stride);
            for x in 0..g.w {
                write_volatile(out.add(x), v);
            }
        }
    }
}

/// Copy the whole shared framebuffer to the screen now.
pub fn present() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: GOP was set once at attach; the shared framebuffer is a 'static W*H buffer;
    // writes stay inside the mode the firmware reported (centred, scale chosen to fit).
    unsafe {
        let g = GOP;
        if MODE.load(Ordering::Relaxed) != WHOLE {
            present_stretched(g);
            return;
        }
        let s = scale(g.w, g.h);
        let (ox, oy) = ((g.w - W * s) / 2, (g.h - H * s) / 2);
        let src = crate::framebuffer::pixels();
        let dst = g.base as *mut u32;
        for y in 0..H {
            let row = src.add(y * W);
            // build the screen row once (scaled, pixel order converted) and copy it to each
            // of its `s` screen lines as a block -- a real 1920x1080 screen at 2x would
            // otherwise take a million single writes per redraw, inside the timer interrupt
            let line: *const u32 = if s == 1 && g.bgr {
                row // same pixel layout (0x00RRGGBB little-endian = B,G,R,X)
            } else {
                let l = addr_of_mut!(LINE) as *mut u32;
                for x in 0..W {
                    let p = *row.add(x);
                    let v = if g.bgr { p } else { (p & 0xFF) << 16 | (p & 0xFF00) | (p >> 16) & 0xFF };
                    for sx in 0..s {
                        *l.add(x * s + sx) = v;
                    }
                }
                l
            };
            for sy in 0..s {
                let out = dst.add((oy + y * s + sy) * g.stride + ox);
                core::ptr::copy_nonoverlapping(line, out, W * s);
            }
        }
        if let Some((px, py)) = crate::xhci::pointer() {
            cursor(g, (ox, oy, W * s, H * s), px, py);
        }
        PRESENTS += 1;
    }
}

/// `present` for `fit` and `fill`: console row y covers screen rows ceil(y*dh/H) up to
/// ceil((y+1)*dh/H); screen column dx shows console column XMAP[dx].
/// SAFETY: as `present`; `g` is the attached mode and PLACE was made for it.
unsafe fn present_stretched(g: Gop) {
    let (ox, oy, dw, dh) = PLACE;
    let src = crate::framebuffer::pixels();
    let dst = g.base as *mut u32;
    let l = addr_of_mut!(LINE) as *mut u32;
    let xm = addr_of_mut!(XMAP) as *const u16;
    for y in 0..H {
        let (y0, y1) = ((y * dh).div_ceil(H), ((y + 1) * dh).div_ceil(H));
        if y0 >= y1 {
            continue;
        }
        let row = src.add(y * W);
        for dx in 0..dw {
            let p = *row.add(*xm.add(dx) as usize);
            *l.add(dx) = if g.bgr { p } else { (p & 0xFF) << 16 | (p & 0xFF00) | (p >> 16) & 0xFF };
        }
        for dy in y0..y1 {
            core::ptr::copy_nonoverlapping(l as *const u32, dst.add((oy + dy) * g.stride + ox), dw);
        }
    }
    if let Some((px, py)) = crate::xhci::pointer() {
        cursor(g, (ox, oy, dw, dh), px, py);
    }
    PRESENTS += 1;
}

/// The pointer: an arrow 8 pixels wide, 12 tall, its tip at (px, py) of the console,
/// white with a black outline ('#' outline, 'o' fill), clipped to the console.
const ARROW: [&[u8; 8]; 12] = [
    b"#.......", b"##......", b"#o#.....", b"#oo#....", b"#ooo#...", b"#oooo#..",
    b"#ooooo#.", b"#oooooo#", b"#ooo####", b"#oo#....", b"#o#.....", b"##......",
];
pub const POINTER_FILL: u32 = 0x00FF_FFFF;

/// SAFETY: `g` is the attached mode and (ox, oy, dw, dh) the placement `present` used; console
/// pixel (x, y) covers screen columns ceil(x*dw/W) up to ceil((x+1)*dw/W), rows likewise.
unsafe fn cursor(g: Gop, (ox, oy, dw, dh): (usize, usize, usize, usize), px: usize, py: usize) {
    let dst = g.base as *mut u32;
    for (r, row) in ARROW.iter().enumerate() {
        for (c, &b) in row.iter().enumerate() {
            let (x, y) = (px + c, py + r);
            if b == b'.' || x >= W || y >= H {
                continue;
            }
            let v = if b == b'#' { 0 } else { POINTER_FILL };
            for sy in (y * dh).div_ceil(H)..((y + 1) * dh).div_ceil(H) {
                for sx in (x * dw).div_ceil(W)..((x + 1) * dw).div_ceil(W) {
                    write_volatile(dst.add((oy + sy) * g.stride + ox + sx), v);
                }
            }
        }
    }
}

/// `screeninfo`: the mode, the scale and how often the screen was redrawn.
pub fn info() {
    if !ON.load(Ordering::Relaxed) {
        crate::kprintln!("screen: no GOP framebuffer -- serial console only");
        return;
    }
    // SAFETY: scalar reads.
    let (g, n, (ox, oy, dw, dh)) = unsafe { (GOP, PRESENTS, PLACE) };
    let m = MODE.load(Ordering::Relaxed);
    if m == WHOLE {
        crate::kprintln!(
            "screen: GOP {}x{} (stride {}, {}) at {:#x}; console {}x{} at {}x; {} redraws",
            g.w, g.h, g.stride, if g.bgr { "BGR" } else { "RGB" }, g.base, W, H, scale(g.w, g.h), n
        );
    } else {
        crate::kprintln!(
            "screen: GOP {}x{} (stride {}, {}) at {:#x}; console {}x{} as {}x{} at {},{} ({}); {} redraws",
            g.w, g.h, g.stride, if g.bgr { "BGR" } else { "RGB" }, g.base, W, H, dw, dh, ox, oy, mode_name(m), n
        );
    }
}

/// True when the console is shown on a GOP screen.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}
