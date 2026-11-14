//! Tonga — a minimal compositor (v0).
//!
//! Draws a windowed desktop onto the framebuffer: a wallpaper, a status bar,
//! several windows (surfaces) with title-bar chrome, and a dock. In the real
//! architecture ([`docs/02`]) surfaces are owned by apps and delivered over IPC,
//! composited zero-copy from shared memory. This v0 composes windows the kernel
//! draws itself, populated with **real system state** (VFS files, Asili devices,
//! heap usage) — enough to show the GUI direction and the compositor primitives.

use crate::framebuffer as fb;
use crate::font;

const W: usize = fb::WIDTH;
const H: usize = fb::HEIGHT;

// Palette
const WALL_TOP: u32 = 0x0E_18_36;
const WALL_BOT: u32 = 0x05_07_0F;
const BAR: u32 = 0x0A_0E_1A;
const BODY: u32 = 0x14_1A_28;
const SHADOW: u32 = 0x03_05_0A;
const BORDER: u32 = 0x2A_35_50;
const GOLD: u32 = 0xF2_B7_05;
const WHITE: u32 = 0xFF_FF_FF;
const DIM: u32 = 0x8F_A6_C8;

fn lerp(a: u32, b: u32, t: usize) -> u32 {
    let ch = |sh: u32| -> u32 {
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        (av + (bv - av) * (t as i32) / 100) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

// The desktop itself (`gui`) is drawn by desk.rs (docs/design/desktop.md, 8 Nov 2026);
// this file keeps the window server for app-owned surfaces.

// ---------------------------------------------------------------------------
// Window server: app-owned surfaces.
//
// An EL0 app calls win_create() to get a surface, draws into it with win_fill()
// (server-side rects), and win_present() composites all surfaces onto the
// screen. In the full architecture the surface buffer is shared memory the app
// draws into directly and the compositor is a userspace service; this v0 keeps
// the buffers server-side and the draws come in as syscalls — same client/server
// shape, simpler plumbing.
// ---------------------------------------------------------------------------
const NWIN: usize = 4;
const CW: usize = 240; // max surface content width
const CH: usize = 130; // max surface content height
const TITLE_H: usize = 20;

#[derive(Clone, Copy)]
struct Win {
    used: bool,
    owner: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    buf: [u32; CW * CH],
}

const WIN0: Win = Win { used: false, owner: 0, x: 0, y: 0, w: 0, h: 0, buf: [0; CW * CH] };
static mut WINS: [Win; NWIN] = [WIN0; NWIN]; #[cfg(not(target_arch = "aarch64"))] static mut ZORDER: [usize; NWIN] = [0, 1, 2, 3]; #[cfg(not(target_arch = "aarch64"))] pub fn win_raise(id: usize) { unsafe { let z = &mut *core::ptr::addr_of_mut!(ZORDER); if let Some(k) = z.iter().position(|&w| w == id) { for j in k..NWIN - 1 { z[j] = z[j + 1]; } z[NWIN - 1] = id; } } } #[cfg(not(target_arch = "aarch64"))] pub fn win_order_reset() { unsafe { *core::ptr::addr_of_mut!(ZORDER) = [0, 1, 2, 3]; } } #[cfg(not(target_arch = "aarch64"))] pub fn win_order() -> [usize; NWIN] { unsafe { ZORDER } }
static mut NWINS: usize = 0; #[cfg(not(target_arch = "aarch64"))] pub fn win_title_at(px: usize, py: usize) -> Option<usize> { unsafe { ZORDER.iter().rev().copied().find(|&i| { let w = &*win(i); i < NWINS && w.used && px >= w.x && px < w.x + w.w && py >= w.y && py < w.y + TITLE_H }) } } #[cfg(not(target_arch = "aarch64"))] pub fn win_move(id: usize, dx: i32, dy: i32) { unsafe { if id < NWIN && (*win(id)).used { let w = win(id); (*w).x = ((*w).x as i32 + dx).clamp(0, (W - (*w).w) as i32) as usize; (*w).y = ((*w).y as i32 + dy).clamp(28, (H - TITLE_H - (*w).h) as i32) as usize; } } } #[cfg(not(target_arch = "aarch64"))] pub fn win_pos(id: usize) -> Option<(usize, usize, usize, usize)> { unsafe { if id < NWINS && (*win(id)).used { let w = &*win(id); Some((w.x, w.y, w.w, w.h)) } else { None } } }

#[inline]
unsafe fn win(i: usize) -> *mut Win {
    (core::ptr::addr_of_mut!(WINS) as *mut Win).add(i)
}

/// Reset the window server (called before a fresh app-surface demo).
pub fn wm_reset() {
    // SAFETY: single-threaded shell context.
    unsafe {
        for i in 0..NWIN {
            (*win(i)).used = false;
        }
        NWINS = 0;
    }
}

/// Create a surface for `owner`; returns a window id or -1 if full.
pub fn win_create(owner: usize, w: usize, h: usize) -> i64 {
    // SAFETY: window ops run at EL1 with IRQs masked (syscall context).
    unsafe {
        if NWINS >= NWIN {
            return -1;
        }
        let id = NWINS;
        let wi = win(id);
        (*wi).used = true;
        (*wi).owner = owner;
        // At least 2: the compositor draws the body at `w - 2` (afriui.md section 19; a 0 or 1 wrapped and faulted).
        (*wi).w = w.clamp(2, CW);
        (*wi).h = h.min(CH);
        // Simple grid placement.
        (*wi).x = 30 + (id % 2) * 300;
        (*wi).y = 44 + (id / 2) * 200;
        // One memset (compiler_builtins, fast at opt-level 0), not a 31,200-pixel loop (afriui.md section 20).
        core::ptr::write_bytes(core::ptr::addr_of_mut!((*wi).buf) as *mut u32, 0, CW * CH);
        NWINS += 1;
        id as i64
    }
}

/// Fill a rectangle within window `id`'s surface (clipped to the surface).
pub fn win_fill(id: usize, rx: usize, ry: usize, rw: usize, rh: usize, color: u32) {
    // SAFETY: syscall context; id bounds-checked below.
    unsafe {
        if id >= NWIN || !(*win(id)).used {
            return;
        }
        // Fast at opt-level 0 (afriui.md section 21): clip once (saturating, so 64-bit sizes from pc/proc.rs stop at
        // the edge), write the first row by doubling copies, then copy it into each row below (compiler_builtins'
        // memcpy). An empty clip returns before anything is written. Same pixels as the old per-pixel loop.
        let wi = win(id);
        let x1 = rx.saturating_add(rw).min((*wi).w);
        let y1 = ry.saturating_add(rh).min((*wi).h);
        if rx >= x1 || ry >= y1 {
            return;
        }
        let n = x1 - rx;
        let buf = core::ptr::addr_of_mut!((*wi).buf) as *mut u32;
        let row = buf.add(ry * CW + rx);
        row.write(color);
        let mut done = 1;
        while done < n {
            let k = done.min(n - done);
            core::ptr::copy_nonoverlapping(row, row.add(done), k);
            done += k;
        }
        for y in ry + 1..y1 {
            core::ptr::copy_nonoverlapping(row, buf.add(y * CW + rx), n);
        }
    }
}

/// Draw an 8x8 glyph into window `id`'s surface at (x, y).
pub fn win_text(id: usize, x: usize, y: usize, ch: u8, color: u32) {
    // SAFETY: syscall context; id bounds-checked.
    unsafe {
        if id >= NWIN || !(*win(id)).used {
            return;
        }
        let wi = win(id);
        let g = font::glyph(ch);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..8 {
                if (bits >> (7 - col)) & 1 == 1 {
                    // Saturating: pc/proc.rs passes 64-bit x and y (afriui.md section 21).
                    let px = x.saturating_add(col);
                    let py = y.saturating_add(row);
                    if px < (*wi).w && py < (*wi).h {
                        (*wi).buf[py * CW + px] = color;
                    }
                }
            }
        }
    }
}

/// Composite the desktop and all app surfaces onto the framebuffer.
pub fn win_present() {
    present_wall();
    present_text();
    present_windows();
}

/// The status bar's height: the wallpaper starts below it, and its pixels never change.
const BAR_H: usize = 28;
/// The status bar with its text, drawn once and kept (afriui.md section 20). 70 KiB.
static mut BAR_PIXELS: [u32; W * BAR_H] = [0; W * BAR_H];
static mut BAR_KEPT: bool = false;

/// win_present, part 1: the wallpaper gradient below the bar (split out for `fbcon wm`, afriui.md section 20).
/// Row y has the colour `lerp(.., y * 100 / H)`, so rows come in bands of one colour: each band is one `fill_rect`
/// (one row filled, then copied down), not one call and one `lerp` per row. The bar covers rows 0..BAR_H, so the
/// gradient starts below it. Same pixels as the old row-by-row loop.
fn present_wall() {
    let mut y = BAR_H;
    while y < H {
        let t = y * 100 / H;
        let end = ((t + 1) * H).div_ceil(100).min(H); // the first row of the next band
        fb::fill_rect(0, y, W, end - y, lerp(WALL_TOP, WALL_BOT, t));
        y = end;
    }
}

/// win_present, part 2: the status bar and its two lines of text. Drawn once, then copied back from BAR_PIXELS.
fn present_text() {
    // SAFETY: syscall or shell context, single writer; BAR_PIXELS is W * BAR_H pixels and the framebuffer's first
    // BAR_H rows are W * BAR_H pixels; the two never overlap.
    unsafe {
        let kept = core::ptr::addr_of_mut!(BAR_PIXELS) as *mut u32;
        if BAR_KEPT {
            fb::blit(0, 0, W, BAR_H, kept, W);
            return;
        }
        fb::fill_rect(0, 0, W, BAR_H, BAR);
        fb::draw_string(10, 6, "AFRISOVERA OS", GOLD, 2);
        fb::draw_string(W - 232, 10, "TONGA WM: APP SURFACES", DIM, 1); // afriui.md section 22
        core::ptr::copy_nonoverlapping(fb::pixels(), kept, W * BAR_H);
        BAR_KEPT = true;
    }
}

/// win_present, part 3: each window's chrome and its surface.
fn present_windows() {
    let accents = [0x1B_3A_6B, 0x2B_1B_6B, 0x1B_5B_3A, 0x6B_3A_1B];
    // SAFETY: read-only iteration over the window table in syscall context.
    unsafe {
        for id in 0..NWIN { #[cfg(not(target_arch = "aarch64"))] let id = ZORDER[id];
            let wi = win(id);
            if !(*wi).used {
                continue;
            }
            let (x, y, w, h) = ((*wi).x, (*wi).y, (*wi).w, (*wi).h);
            let accent = accents[(*wi).owner % accents.len()];
            // chrome
            fb::fill_rect(x + 4, y + 4, w, TITLE_H + h, SHADOW);
            fb::fill_rect(x, y, w, TITLE_H, accent);
            fb::fill_rect(x, y, w, TITLE_H + h, BORDER); // frame fill (overdrawn by blit)
            fb::fill_rect(x, y, w, TITLE_H, accent);
            fb::fill_rect(x + 1, y + TITLE_H, w - 2, h, BODY);
            // title "APP P<owner>"
            let mut nb = [0u8; 12];
            let ns = fmt_u32((*wi).owner as u32, &mut nb);
            fb::draw_string(x + 8, y + 6, "APP  P", WHITE, 1);
            fb::draw_string(x + 56, y + 6, ns, WHITE, 1);
            // blit the app's surface into the window body
            fb::blit(x + 1, y + TITLE_H, w.min(CW) - 2, h, (*wi).buf.as_ptr(), CW);
        }
    }
}

/// Minimal u32 → decimal string into `buf`, returning the &str slice.
fn fmt_u32(mut v: u32, buf: &mut [u8; 12]) -> &str {
    if v == 0 {
        buf[0] = b'0';
        return core::str::from_utf8(&buf[..1]).unwrap_or("0");
    }
    let mut tmp = [0u8; 12];
    let mut i = 0;
    while v > 0 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    core::str::from_utf8(&buf[..i]).unwrap_or("?")
}

/// `fbcon wm` (afriui.md section 20): time win_create and win_present (with its three parts), then two checks:
/// a re-created surface is all zero, and a hash of the whole screen after win_present with two patterned windows.
/// Leaves the window server empty and the console redrawn. Refuses, with one line, when a window is in use or the
/// desk or another graphics screen is shown (the architect's condition 1), so it never resets a live window table.
pub fn wm_time() {
    // SAFETY: shell mainline; reads the window table's `used` flags only.
    let busy = (0..NWIN).filter(|&i| unsafe { (*win(i)).used }).count();
    if busy > 0 || crate::desk::shown() || !fb::console_shown() {
        crate::kprintln!(
            "[fbwm] refused: {} window(s) in use, desk {}, console {} (fbcon wm needs no windows and the console shown)",
            busy,
            if crate::desk::shown() { "shown" } else { "hidden" },
            if fb::console_shown() { "shown" } else { "hidden" }
        );
        return;
    }
    let hz = crate::timer::hz().max(1);
    let us = |t0: u64| crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / hz;
    let (mut big, mut small, mut present, mut wall, mut text, mut wins) =
        (u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX);
    for _ in 0..8 {
        wm_reset();
        let t0 = crate::timer::cycles();
        let _ = win_create(0, CW, CH);
        big = big.min(us(t0));
        wm_reset();
        let t0 = crate::timer::cycles();
        let _ = win_create(0, 40, 20);
        small = small.min(us(t0));
        let t0 = crate::timer::cycles();
        win_present();
        present = present.min(us(t0));
        let t0 = crate::timer::cycles();
        present_wall();
        wall = wall.min(us(t0));
        let t0 = crate::timer::cycles();
        present_text();
        text = text.min(us(t0));
        let t0 = crate::timer::cycles();
        present_windows();
        wins = wins.min(us(t0));
    }
    // Clear check: a used surface, reset, created again, must be all zero.
    wm_reset();
    let id = win_create(0, CW, CH) as usize;
    win_fill(id, 0, 0, CW, CH, 0x00AB_CDEF);
    wm_reset();
    let id = win_create(0, CW, CH) as usize;
    // SAFETY: shell mainline; `id` is the slot just created.
    let zero = unsafe { (*win(id)).buf.iter().filter(|&&p| p == 0).count() };
    // Fill time and clip check (afriui.md section 21).
    wm_reset();
    let fa = win_create(0, CW, CH) as usize;
    let fs = win_create(0, 40, 20) as usize;
    let (mut fbig, mut fsmall) = (u64::MAX, u64::MAX);
    for k in 0..8u32 {
        let t0 = crate::timer::cycles();
        win_fill(fa, 0, 0, CW, CH, 0x0012_3400 + k);
        fbig = fbig.min(us(t0));
        let t0 = crate::timer::cycles();
        win_fill(fs, 0, 0, 40, 20, 0x0012_3400 + k);
        fsmall = fsmall.min(us(t0));
    }
    let (inside, rest1) = fill_check(200, 100, 100, 100);
    let corner = SATURATING_CLIP.then(|| fill_check(230, 120, usize::MAX, usize::MAX));
    let rest_ok = rest1 == 0 && corner.map_or(true, |(_, r)| r == 0);
    // Empty clips write nothing (the architect's condition 1): a fill starting at the right edge, and one 0 wide.
    let (e1, e2) = (fill_check(CW, 0, 10, 10), fill_check(10, 10, 0, 10));
    let empty = e1.0 + e1.1 + e2.0 + e2.1;
    // Text far outside the surface writes nothing and does not wrap (condition 2; run once the clip saturates).
    let far = SATURATING_CLIP.then(|| {
        wm_reset();
        let id = win_create(0, CW, CH) as usize;
        win_text(id, usize::MAX - 3, 5, b'A', WHITE);
        win_text(id, 5, usize::MAX - 3, b'A', WHITE);
        // SAFETY: shell mainline; `id` is the slot just created.
        unsafe { (*win(id)).buf.iter().filter(|&&p| p != 0).count() }
    });
    // Match check: two windows with known patterns, then a hash of the whole screen.
    wm_reset();
    let a = win_create(1, CW, CH) as usize;
    let b = win_create(2, 40, 20) as usize;
    for k in 0..6 {
        win_fill(a, k * 37, k * 19, 50, 30, 0x0011_2233u32.wrapping_mul(k as u32 + 1));
    }
    for (i, &c) in b"FAST WM".iter().enumerate() {
        win_text(a, 8 + i * 9, 100, c, WHITE);
    }
    win_fill(b, 0, 0, 40, 20, 0x0033_6699);
    win_fill(b, 5, 5, 10, 10, GOLD);
    win_present();
    let fbp = fb::pixels();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for i in 0..W * H {
        // SAFETY: `pixels()` is the W x H framebuffer.
        h = (h ^ unsafe { *fbp.add(i) } as u64).wrapping_mul(0x100_0000_01b3);
    }
    wm_reset();
    fb::console_reset();
    crate::kprintln!(
        "[fbwm] win_create {}x{}: fastest {} us; 40x20: fastest {} us; win_present: fastest {} us (wallpaper {}, text {}, windows {})",
        CW, CH, big, small, present, wall, text, wins
    );
    crate::kprintln!("[fbwm] clear check: {} of {} pixels zero; screen hash {:016x}", zero, CW * CH, h);
    let (mut cb, mut fb2) = ([0u8; 12], [0u8; 12]);
    let (cn, cs) = match corner {
        Some((c, _)) => (fmt_u32(c as u32, &mut cb), " of 100 corner"),
        None => ("corner not run", ""),
    };
    let (fnn, fs2) = match far {
        Some(n) => (fmt_u32(n as u32, &mut fb2), " from far text"),
        None => ("far text not run", ""),
    };
    crate::kprintln!(
        "[fbwm] win_fill {}x{}: fastest {} us; 40x20: fastest {} us; clip check: {} of 1200 inside, {}{}, {} from empty fills, {}{}, rest {}",
        CW, CH, fbig, fsmall, inside, cn, cs, empty, fnn, fs2,
        if rest_ok { "unchanged" } else { "CHANGED" }
    );
}

/// The corner and far-text cases feed `usize::MAX`-sized values; only the saturating clip survives them, so the
/// probe runs them once `win_fill` and `win_text` saturate (afriui.md section 21).
const SATURATING_CLIP: bool = true;

/// `fbcon wm`'s clip check: on a fresh 240x130 surface, fill (x, y, w, h) and count the pixels coloured inside
/// [x, CW) x [y, CH) and the pixels changed outside it, over the whole CW*CH buffer (the architect's condition 3).
fn fill_check(x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    const COL: u32 = 0x00FE_DCBA;
    wm_reset();
    let id = win_create(0, CW, CH) as usize;
    win_fill(id, x, y, w, h, COL);
    let (mut inside, mut changed) = (0, 0);
    // SAFETY: shell mainline; `id` is the slot just created.
    let buf = unsafe { &(*win(id)).buf };
    for (p, &px) in buf.iter().enumerate() {
        if p % CW >= x && p / CW >= y {
            inside += (px == COL) as usize;
        } else {
            changed += (px != 0) as usize;
        }
    }
    (inside, changed)
}
