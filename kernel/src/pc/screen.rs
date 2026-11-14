//! The PC's screen after boot (5 Nov 2026), for PC step 4: what the shared framebuffer
//! holds (the shell's text console and the Tonga windows, 640x480, drawn by the same
//! `framebuffer.rs` the ARM kernel shows through QEMU's ramfb) is copied onto the UEFI
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
/// One scaled screen row: up to 4x the 640-pixel console, or the desktop canvas's row at its
/// UI scale (a screen up to 4096 pixels wide).
const LINE_MAX: usize = 4096;
static mut LINE: [u32; LINE_MAX] = [0; LINE_MAX];
/// What the last redraw showed: the desktop canvas (true) or the 640x480 console. A change
/// repaints the frame around it (the canvas fills the screen; the console sits in bands).
static LAST_DESK: AtomicBool = AtomicBool::new(false);

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
        crate::kprintln!("[pc] screen: GOP mode {}x{} is smaller than the {}x{} console, so serial only", w, h, W, H);
        return;
    }
    // a real machine may put the framebuffer above the 4 GiB the kernel maps 1:1; its pages
    // are mapped write-combining where the CPU has PAT (mmu::map_screen), else uncached
    if crate::mmu::map_screen(base, (stride * h * 4) as u64, true).is_none() {
        crate::kprintln!("[pc] screen: could not map the GOP framebuffer at {:#x}, so serial only", base);
        return;
    }
    // SAFETY: boot, single core, before the timer reads it.
    unsafe { GOP = Gop { base, w, h, stride, bgr } };
    place(MODE.load(Ordering::Relaxed));
    crate::canvas::setup(w.min(LINE_MAX), h, 0);
    ON.store(true, Ordering::Relaxed);
    let s = scale(w, h);
    crate::kprintln!("[pc] screen: the {}x{} console shown on the {}x{} GOP screen at {}x", W, H, w, h, s);
    crate::kprintln!(
        "[pc] screen: the desktop at {}x{} logical pixels, UI scale {}x (the whole screen)",
        crate::canvas::width(), crate::canvas::height(), crate::canvas::scale()
    );
}

/// The pointer's range: the desktop canvas while the desktop is shown, else the console.
pub fn pointer_area() -> (i32, i32) {
    if crate::desk::shown() {
        (crate::canvas::width() as i32, crate::canvas::height() as i32)
    } else {
        (W as i32, H as i32)
    }
}

/// `screen ui`, `screen ui 1..4|auto`: the desktop's UI scale (section 13 of afriui.md).
fn ui_command(arg: &str) {
    if !ON.load(Ordering::Relaxed) {
        crate::kprintln!("screen: no GOP framebuffer, so the desktop has no screen to fill (serial console only)");
        return;
    }
    let s = match arg {
        "" => {
            crate::kprintln!(
                "screen: the desktop is {}x{} logical pixels at UI scale {}x; type: screen ui 1, 2, 3, 4 or auto",
                crate::canvas::width(), crate::canvas::height(), crate::canvas::scale()
            );
            return;
        }
        "auto" => 0,
        "1" | "2" | "3" | "4" => (arg.as_bytes()[0] - b'0') as usize,
        other => {
            crate::kprintln!("screen: unknown UI scale {}; use: screen ui 1, 2, 3, 4 or auto", other);
            return;
        }
    };
    crate::canvas::set_scale(s);
    crate::desk::relayout();
    clear(FRAME);
    FULL.store(true, Ordering::Relaxed); // the whole desktop, at its new size
    mark();
    crate::kprintln!(
        "screen: the desktop is now {}x{} logical pixels at UI scale {}x",
        crate::canvas::width(), crate::canvas::height(), crate::canvas::scale()
    );
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
        a if a == "ui" || a.starts_with("ui ") => {
            ui_command(a[2..].trim());
            return;
        }
        "wc on" | "wc off" => {
            // write-combining for the screen's pages, on or off (a real machine that misbehaves)
            if !ON.load(Ordering::Relaxed) {
                crate::kprintln!("screen: no GOP framebuffer");
                return;
            }
            // SAFETY: GOP is set at attach; scalar reads.
            let g = unsafe { GOP };
            let on = arg.trim() == "wc on";
            // architect's D1 condition 3: never change a type another core cannot be told about
            if !crate::smp::can_flush_others() {
                crate::kprintln!("screen: core 1 is still starting and cannot reload its page tables; try again");
                return;
            }
            let before = crate::smp::flushes_done();
            match crate::mmu::map_screen(g.base, (g.stride * g.h * 4) as u64, on) {
                Some(_) => wc_line(),
                None => crate::kprintln!("screen: could not change the screen's pages (out of table frames)"),
            }
            core1_line(before);
            return;
        }
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
    // the desktop's canvas is copied in mainline (`flush`, from desk::idle), never in this
    // interrupt: at a laptop's own resolution a whole copy to an uncached framebuffer can take
    // tens of milliseconds (architect's review, 10 Nov 2026)
    if crate::desk::shown() {
        return;
    }
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
    crate::mmu::wc_fence();
}

/// Copy the whole shared framebuffer to the screen now. Ends with `sfence`, so a
/// write-combined frame is out of the CPU's buffers when it returns (`flush` comes here too).
pub fn present() {
    present_now();
    cam_overlay();
    crate::mmu::wc_fence();
}

/// The camera's picture box and its CAMERA ON mark (uvc.rs), drawn last, over the console, the desktop and the
/// pointer, so nothing drawn before can cover the mark.
fn cam_overlay() {
    if !ON.load(Ordering::Relaxed) || !crate::uvc::mark_on() {
        return;
    }
    // SAFETY: GOP was set once at attach; uvc::draw writes inside the mode the firmware reported.
    unsafe {
        let g = GOP;
        crate::uvc::draw(g.base, g.w, g.h, g.stride, g.bgr);
    }
}

/// A new camera picture (uvc::idle, mainline): the box and the mark now, without waiting for a redraw.
pub fn cam_draw() {
    cam_overlay();
    crate::mmu::wc_fence();
}

/// The camera stopped: the frame painted afresh and everything copied again, so no picture or mark is left.
pub fn repaint_all() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    clear(FRAME);
    FULL.store(true, Ordering::Relaxed);
    DIRTY.store(true, Ordering::Relaxed);
}

fn present_now() {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: GOP was set once at attach; the shared framebuffer is a 'static W*H buffer;
    // writes stay inside the mode the firmware reported (centred, scale chosen to fit).
    unsafe {
        let g = GOP;
        let desk = crate::desk::shown();
        if LAST_DESK.swap(desk, Ordering::Relaxed) != desk {
            clear(FRAME); // desktop <-> console: the frame around changes
            FULL.store(true, Ordering::Relaxed);
        }
        if desk {
            present_canvas(g);
            return;
        }
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
            // of its `s` screen lines as a block: a real 1920x1080 screen at 2x would
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

/// The desktop's copy to the screen, from mainline (desk::idle): when something drew or the
/// pointer moved, copy what changed.
pub fn flush() {
    if crate::desk::shown() && DIRTY.swap(false, Ordering::Relaxed) {
        present();
    }
}

/// The next desktop copy is whole (after a switch from the console or a new UI scale).
static FULL: AtomicBool = AtomicBool::new(true);
/// Where the pointer was drawn by the last desktop copy (its rows are put back next time).
static mut LAST_PTR: Option<(usize, usize)> = None;
/// The desktop copy's time in microseconds: the last one and the worst one (`desk perf`).
static mut COPY_US: (u64, u64) = (0, 0);

/// (last, worst) desktop copy time in microseconds, and the redraw count, for `desk perf`.
pub fn copy_perf() -> (u64, u64, u64, u64) {
    // SAFETY: scalar reads.
    unsafe { (COPY_US.0, COPY_US.1, PRESENTS, COPY_ROWS) }
}

/// Canvas rows the last desktop copy moved to the screen (`desk perf`).
static mut COPY_ROWS: u64 = 0;

/// `present` while the desktop is shown: its canvas (kernel/src/pc/canvas.rs), the screen's
/// own size over a whole-number UI scale, centred (it fills the screen but for the few pixels
/// a non-dividing scale leaves), then the pointer at that scale. Only the rows the desktop
/// drew since the last copy, and the rows under the pointer's old and new place, are copied.
/// SAFETY: as `present`; the canvas is width() x height() pixels, a row width() long.
unsafe fn present_canvas(g: Gop) {
    let t0 = crate::timer::cycles();
    let (cw, ch, s) = (crate::canvas::width(), crate::canvas::height(), crate::canvas::scale());
    let (dw, dh) = ((cw * s).min(g.w).min(LINE_MAX), (ch * s).min(g.h));
    let (vw, vh) = (dw / s, dh / s); // the canvas pixels that fit
    let (ox, oy) = ((g.w - vw * s) / 2, (g.h - vh * s) / 2);
    let ptr = crate::xhci::pointer();
    // the rows drawn (up to canvas::NSPAN spans, architect's N4) and the pointer's old and new rows
    let mut drawn = [(0, 0); crate::canvas::NSPAN];
    let n = crate::canvas::take_damage(&mut drawn);
    let mut spans: [(usize, usize); crate::canvas::NSPAN + 2] = [(0, 0); crate::canvas::NSPAN + 2];
    if FULL.swap(false, Ordering::Relaxed) {
        spans[0] = (0, vh);
    } else {
        spans[..n].copy_from_slice(&drawn[..n]);
        if let Some((_, y)) = LAST_PTR {
            spans[n] = (y, y + ARROW.len());
        }
        if let Some((_, y)) = ptr {
            spans[n + 1] = (y, y + ARROW.len());
        }
    }
    let mut rows = 0;
    for &(a, b) in spans.iter() {
        let b = b.min(vh);
        if b > a {
            copy_rows(g, (ox, oy, vw, s, cw), a, b);
            rows += (b - a) as u64;
        }
    }
    COPY_ROWS = rows;
    LAST_PTR = ptr;
    if let Some((px, py)) = ptr {
        let dst = g.base as *mut u32;
        for (r, row) in ARROW.iter().enumerate() {
            for (c, &b) in row.iter().enumerate() {
                let (x, y) = (px + c, py + r);
                if b == b'.' || x >= vw || y >= vh {
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
    PRESENTS += 1;
    let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    COPY_US = (us, COPY_US.1.max(us));
}

/// Copy canvas rows [y0, y1) to the screen at the UI scale.
/// SAFETY: as `present_canvas`; (ox, oy, vw, s, cw) as it worked them out.
unsafe fn copy_rows(g: Gop, (ox, oy, vw, s, cw): (usize, usize, usize, usize, usize), y0: usize, y1: usize) {
    let src = crate::canvas::pixels();
    let dst = g.base as *mut u32;
    for y in y0..y1 {
        let row = src.add(y * cw);
        let line: *const u32 = if s == 1 && g.bgr {
            row
        } else {
            let l = addr_of_mut!(LINE) as *mut u32;
            for x in 0..vw {
                let p = *row.add(x);
                let v = if g.bgr { p } else { (p & 0xFF) << 16 | (p & 0xFF00) | (p >> 16) & 0xFF };
                for sx in 0..s {
                    *l.add(x * s + sx) = v;
                }
            }
            l
        };
        for sy in 0..s {
            core::ptr::copy_nonoverlapping(line, dst.add((oy + y * s + sy) * g.stride + ox), vw * s);
        }
    }
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
        crate::kprintln!("screen: no GOP framebuffer, so serial console only");
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
    wc_line();
}

/// `screen` and `screen wc on|off`: how the screen's pages are cached (PAT write-combining).
/// After a change of the screen's type: what core 1 did (architect's D1a). Silent when only
/// one core runs.
fn core1_line(before: u32) {
    let n = crate::smp::flushes_done() - before;
    if !crate::mmu::others_flushed() {
        crate::kprintln!("screen: core 1 did not answer; reboot to be sure");
    } else if n > 0 {
        crate::kprintln!("screen: core 1 drained its buffers and reloaded its page tables ({} IPI)", n);
    }
}

/// Once core 1 is online (from main, after smp::bring_up): the screen was mapped WC while only
/// the boot core ran; if core 1's PAT entry 4 did not read back as WC, map the screen uncached
/// so no core sees its pages as WB (architect's D1c).
pub fn pat_check() {
    if !ON.load(Ordering::Relaxed) || crate::mmu::pat_all_cores() {
        return;
    }
    let (_, _, base, len) = crate::mmu::wc_state();
    if crate::mmu::cache_type(base) != "WC" {
        return;
    }
    // core 1 must take the shootdown IPI: wait (up to 0.2 s) for its scheduler
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while !crate::smp::can_flush_others() && crate::timer::cycles() - t0 < hz / 5 {
        core::hint::spin_loop();
    }
    let before = crate::smp::flushes_done();
    match crate::mmu::map_screen(base, len, false) {
        Some(_) => wc_line(),
        None => crate::kprintln!("screen: could not change the screen's pages (out of table frames)"),
    }
    core1_line(before);
}

fn wc_line() {
    let (pat, cores, base, len) = crate::mmu::wc_state();
    let (first, last) = (crate::mmu::cache_type(base), crate::mmu::cache_type(base + len.max(4096) - 4096));
    if !crate::mmu::pat_present() {
        crate::kprintln!("screen: write-combining off, this CPU has no PAT; the screen's pages are {}", first);
    } else {
        crate::kprintln!(
            "screen: write-combining {}, PAT entry 4 = WC on {} core(s); the screen's {} KiB at {:#x}: first page {}, last page {}",
            if pat && first == "WC" { "on" } else { "off" }, cores, len / 1024, base, first, last
        );
        if pat && !crate::mmu::pat_all_cores() {
            crate::kprintln!(
                "screen: PAT entry 4 is WC on only {} of {} cores, so the screen stays uncached",
                cores, crate::smp::cores_running()
            );
        }
    }
}

/// True when the console is shown on a GOP screen.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}
