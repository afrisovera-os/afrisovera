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
use core::sync::atomic::{AtomicBool, Ordering};

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
    ON.store(true, Ordering::Relaxed);
    let s = scale(w, h);
    crate::kprintln!("[pc] screen: the {}x{} console shown on the {}x{} GOP screen at {}x", W, H, w, h, s);
}

fn scale(w: usize, h: usize) -> usize {
    (w / W).min(h / H).clamp(1, 4)
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

/// Copy the whole shared framebuffer to the screen now.
pub fn present() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: GOP was set once at attach; the shared framebuffer is a 'static W*H buffer;
    // writes stay inside the mode the firmware reported (centred, scale chosen to fit).
    unsafe {
        let g = GOP;
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
            cursor(g, s, ox, oy, px, py);
        }
        PRESENTS += 1;
    }
}

/// The pointer: an arrow 8 pixels wide, 12 tall, its tip at (px, py) of the console,
/// white with a black outline ('#' outline, 'o' fill), clipped to the console.
const ARROW: [&[u8; 8]; 12] = [
    b"#.......", b"##......", b"#o#.....", b"#oo#....", b"#ooo#...", b"#oooo#..",
    b"#ooooo#.", b"#oooooo#", b"#ooo####", b"#oo#....", b"#o#.....", b"##......",
];
pub const POINTER_FILL: u32 = 0x00FF_FFFF;

/// SAFETY: `g` is the attached mode and (ox, oy, s) the placement `present` used.
unsafe fn cursor(g: Gop, s: usize, ox: usize, oy: usize, px: usize, py: usize) {
    let dst = g.base as *mut u32;
    for (r, row) in ARROW.iter().enumerate() {
        for (c, &b) in row.iter().enumerate() {
            let (x, y) = (px + c, py + r);
            if b == b'.' || x >= W || y >= H {
                continue;
            }
            let v = if b == b'#' { 0 } else { POINTER_FILL };
            for sy in 0..s {
                for sx in 0..s {
                    write_volatile(dst.add((oy + y * s + sy) * g.stride + ox + x * s + sx), v);
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
    let (g, n) = unsafe { (GOP, PRESENTS) };
    crate::kprintln!(
        "screen: GOP {}x{} (stride {}, {}) at {:#x}; console {}x{} at {}x; {} redraws",
        g.w, g.h, g.stride, if g.bgr { "BGR" } else { "RGB" }, g.base, W, H, scale(g.w, g.h), n
    );
}

/// True when the console is shown on a GOP screen.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}
