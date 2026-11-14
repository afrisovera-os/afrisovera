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

/// The rows drawn since the screen last copied the canvas, as up to NSPAN separate spans
/// [top, bottom) (architect's notes, 10 Nov 2026: copy only what changed; N4: boxes far apart
/// must not copy every row between them). Spans that overlap or nearly touch merge; a span
/// past NSPAN merges into the one nearest it. Mainline only (the desktop draws, then copies).
pub const NSPAN: usize = 8;
static mut SPANS: [(usize, usize); NSPAN] = [(0, 0); NSPAN];
static mut NSPANS: usize = 0;

/// Rows y0..y1 were drawn.
fn add_span(y0: usize, y1: usize) {
    // SAFETY: mainline only.
    unsafe {
        let s = &mut *core::ptr::addr_of_mut!(SPANS);
        let (mut a, mut b) = (y0, y1);
        // fold in every span this one overlaps or nearly touches (4 rows), as often as it grows
        let mut k = 0;
        while k < NSPANS {
            let (t, u) = s[k];
            if a <= u + 4 && t <= b + 4 {
                a = a.min(t);
                b = b.max(u);
                NSPANS -= 1;
                s[k] = s[NSPANS];
                k = 0;
                continue;
            }
            k += 1;
        }
        if NSPANS < NSPAN {
            s[NSPANS] = (a, b);
            NSPANS += 1;
            return;
        }
        // full: join the nearest span
        let gap = |t: usize, u: usize| if b < t { t - b } else { a.saturating_sub(u) };
        let mut best = 0;
        for k in 1..NSPANS {
            if gap(s[k].0, s[k].1) < gap(s[best].0, s[best].1) {
                best = k;
            }
        }
        s[best] = (s[best].0.min(a), s[best].1.max(b));
    }
}

/// The rows drawn since the last call: fills `out` with up to NSPAN spans, returns how many
/// (screen.rs copies just those).
pub fn take_damage(out: &mut [(usize, usize); NSPAN]) -> usize {
    let h = height();
    // SAFETY: mainline only.
    unsafe {
        let n = core::mem::replace(&mut *core::ptr::addr_of_mut!(NSPANS), 0);
        for (k, o) in out.iter_mut().enumerate().take(n) {
            let (t, u) = (*core::ptr::addr_of!(SPANS))[k];
            *o = (t.min(h), u.min(h));
        }
        n
    }
}

/// Fill a rectangle, clipped to the canvas. Mainline only (the desktop draws outside interrupts).
pub fn fill_rect(x0: usize, y0: usize, w: usize, h: usize, rgb: u32) {
    let (cw, ch) = (width(), height());
    let (x1, y1) = ((x0 + w).min(cw), (y0 + h).min(ch));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    add_span(y0, y1);
    // SAFETY: indices are inside width() x height() <= MAX_W x MAX_H; single writer (mainline).
    unsafe {
        let p = addr_of_mut!(BUF) as *mut u32;
        for y in y0..y1 {
            core::slice::from_raw_parts_mut(p.add(y * cw + x0), x1 - x0).fill(rgb);
        }
    }
}

/// Text coverage mixed into the canvas (afriui.md section 26), clipped to it. Mainline only.
pub fn blend(x0: usize, y0: usize, w: usize, h: usize, a: &[u8], astride: usize, rgb: u32, gamma: &[u8; 256]) {
    let (cw, ch) = (width(), height());
    let (w, h) = (w.min(cw.saturating_sub(x0)), h.min(ch.saturating_sub(y0)));
    if w == 0 || h == 0 {
        return;
    }
    add_span(y0, y0 + h);
    // SAFETY: the canvas is a static array; single writer (mainline).
    let buf = unsafe { &mut (*addr_of_mut!(BUF)).0 };
    crate::framebuffer::blend_into(buf, cw, x0, y0, w, h, a, astride, rgb, gamma);
}

/// Black at alphas over the canvas (soft shadows, afriui.md section 30), clipped to it. Mainline only.
pub fn shade(x0: usize, y0: usize, w: usize, h: usize, a: &[u8], astride: usize) {
    let (cw, ch) = (width(), height());
    let (w, h) = (w.min(cw.saturating_sub(x0)), h.min(ch.saturating_sub(y0)));
    if w == 0 || h == 0 {
        return;
    }
    add_span(y0, y0 + h);
    // SAFETY: the canvas is a static array; single writer (mainline).
    let buf = unsafe { &mut (*addr_of_mut!(BUF)).0 };
    crate::framebuffer::shade_into(buf, cw, x0, y0, w, h, a, astride);
}

/// Black at one alpha over a block of the canvas, clipped to it. Mainline only.
pub fn shade_const(x0: usize, y0: usize, w: usize, h: usize, al: u32) {
    let (cw, ch) = (width(), height());
    let (w, h) = (w.min(cw.saturating_sub(x0)), h.min(ch.saturating_sub(y0)));
    if w == 0 || h == 0 {
        return;
    }
    add_span(y0, y0 + h);
    // SAFETY: the canvas is a static array; single writer (mainline).
    let buf = unsafe { &mut (*addr_of_mut!(BUF)).0 };
    crate::framebuffer::shade_const_into(buf, cw, x0, y0, w, h, al);
}

/// One edge pixel mixed into the canvas. Mainline only.
pub fn mix(x: usize, y: usize, rgb: u32, al: u32) {
    let (cw, ch) = (width(), height());
    if x < cw && y < ch {
        add_span(y, y + 1);
        // SAFETY: as above.
        let buf = unsafe { &mut (*addr_of_mut!(BUF)).0 };
        crate::framebuffer::mix_into(buf, y * cw + x, rgb, al);
    }
}

// The desktop's whole boundary to the screen (docs/design/afriui-phone-plan.md, step P1): the
// Sovereign model and paint call only `crate::canvas::*`, so the phone can give them its own
// canvas (P2) without either naming a PC module.

/// Something was drawn: the screen copies it at the next flush.
pub fn mark() {
    crate::screen::mark();
}

/// Copy what changed to the screen (mainline, from desk::idle).
pub fn flush() {
    crate::screen::flush();
}

/// (last, worst) copy to the screen in microseconds, and the copy count (`desk perf`).
/// (last, worst) copy to the screen in microseconds, the copy count, and the rows the last
/// copy moved (`desk perf`).
pub fn copy_perf() -> (u64, u64, u64, u64) {
    crate::screen::copy_perf()
}
