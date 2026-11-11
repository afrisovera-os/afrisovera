//! The desktop's canvas on the PC (10 Nov 2026, docs/design/afriui.md section 13, field
//! item F9): a buffer the size of the screen, so the Sovereign desktop fills a laptop's screen
//! at its own resolution instead of sitting as a 640x480 box in a dark frame.
//!
//! The text console keeps the shared 640x480 framebuffer (framebuffer.rs, the phone's too);
//! only the desktop (kernel/src/afriui/desk.rs, PC only) draws here, and screen.rs copies
//! this canvas to the screen while the desktop is shown. The canvas is the screen's size
//! divided by a whole-number UI scale, so the 8x8 font stays sharp: scale = min(width / 800,
//! height / 500), from 1 to 4 (1366x768 at 1x, 1920x1080 at 2x as 960x540).

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

/// The largest canvas held, in logical pixels (9.2 MB); a bigger one is cut to this, centred.
pub const MAX_W: usize = 1920;
pub const MAX_H: usize = 1200;

#[repr(C, align(4096))]
struct Buf([u32; MAX_W * MAX_H]);
static mut BUF: Buf = Buf([0; MAX_W * MAX_H]);

/// Before a screen is attached (or with none) the canvas is the console's size.
static WIDTH: AtomicUsize = AtomicUsize::new(crate::framebuffer::WIDTH);
static HEIGHT: AtomicUsize = AtomicUsize::new(crate::framebuffer::HEIGHT);
static SCALE: AtomicUsize = AtomicUsize::new(1);
/// The screen's mode, kept so `screen ui N` can lay the canvas out again.
static SCREEN_W: AtomicUsize = AtomicUsize::new(0);
static SCREEN_H: AtomicUsize = AtomicUsize::new(0);

/// The scale the screen's size calls for (section 13).
pub fn auto_scale(sw: usize, sh: usize) -> usize {
    (sw / 800).min(sh / 500).clamp(1, 4)
}

/// Lay the canvas out for a screen of sw x sh pixels at UI scale `s` (0 = automatic).
pub fn setup(sw: usize, sh: usize, s: usize) {
    let s = if s == 0 { auto_scale(sw, sh) } else { s.clamp(1, 4) };
    // never smaller than the 640x480 the desktop was designed at: a smaller screen gets a
    // smaller scale instead
    let mut s = s;
    while s > 1 && (sw / s < crate::framebuffer::WIDTH || sh / s < crate::framebuffer::HEIGHT) {
        s -= 1;
    }
    SCREEN_W.store(sw, Ordering::Relaxed);
    SCREEN_H.store(sh, Ordering::Relaxed);
    SCALE.store(s, Ordering::Relaxed);
    WIDTH.store((sw / s).clamp(crate::framebuffer::WIDTH, MAX_W), Ordering::Relaxed);
    HEIGHT.store((sh / s).clamp(crate::framebuffer::HEIGHT, MAX_H), Ordering::Relaxed);
}

/// `screen ui N`: a new UI scale for the attached screen (1..4, or 0 for automatic).
pub fn set_scale(s: usize) -> bool {
    let (sw, sh) = (SCREEN_W.load(Ordering::Relaxed), SCREEN_H.load(Ordering::Relaxed));
    if sw == 0 {
        return false;
    }
    setup(sw, sh, s);
    true
}

pub fn width() -> usize {
    WIDTH.load(Ordering::Relaxed)
}
pub fn height() -> usize {
    HEIGHT.load(Ordering::Relaxed)
}
pub fn scale() -> usize {
    SCALE.load(Ordering::Relaxed)
}

/// The canvas pixels, `width()` to a row.
pub fn pixels() -> *const u32 {
    // only the address is taken (no unsafe needed)
    addr_of_mut!(BUF) as *const u32
}

/// The rows drawn since the screen last copied the canvas (architect's review, 10 Nov 2026:
/// copy only what changed): [DMG_TOP, DMG_BOT), empty when DMG_TOP >= DMG_BOT. Mainline only.
static DMG_TOP: AtomicUsize = AtomicUsize::new(usize::MAX);
static DMG_BOT: AtomicUsize = AtomicUsize::new(0);

/// The rows drawn since the last call, if any (screen.rs copies just those).
pub fn take_damage() -> Option<(usize, usize)> {
    let (t, b) = (DMG_TOP.swap(usize::MAX, Ordering::Relaxed), DMG_BOT.swap(0, Ordering::Relaxed));
    if t < b {
        Some((t, b.min(height())))
    } else {
        None
    }
}

/// Fill a rectangle, clipped to the canvas. Mainline only (the desktop draws outside interrupts).
pub fn fill_rect(x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
    let (cw, ch) = (width(), height());
    let (x1, y1) = ((x0 + w).min(cw), (y0 + h).min(ch));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    DMG_TOP.fetch_min(y0, Ordering::Relaxed);
    DMG_BOT.fetch_max(y1, Ordering::Relaxed);
    // SAFETY: indices are inside width() x height() <= MAX_W x MAX_H; single writer (mainline).
    unsafe {
        let p = addr_of_mut!(BUF) as *mut u32;
        for y in y0..y1 {
            core::slice::from_raw_parts_mut(p.add(y * cw + x0), x1 - x0).fill(rgb);
        }
    }
}
