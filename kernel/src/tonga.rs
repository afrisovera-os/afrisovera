//! Tonga — a minimal compositor (v0).
//!
//! Draws a windowed desktop onto the framebuffer: a wallpaper, a status bar,
//! several windows (surfaces) with title-bar chrome, and a dock. In the real
//! architecture ([`docs/02`]) surfaces are owned by apps and delivered over IPC,
//! composited zero-copy from shared memory. This v0 composes windows the kernel
//! draws itself, populated with **real system state** (VFS files, Asili devices,
//! heap usage) — enough to show the GUI direction and the compositor primitives.

use crate::framebuffer as fb;
use crate::{asili, config, font, frame, heap, sched, vfs, virtio};

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
const TEXT: u32 = 0xDD_E4_F0;
const DIM: u32 = 0x8F_A6_C8;
const GREEN: u32 = 0x27_C9_3F;

fn lerp(a: u32, b: u32, t: usize) -> u32 {
    let ch = |sh: u32| -> u32 {
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        (av + (bv - av) * (t as i32) / 100) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Draw premium window chrome and return the (x, y) origin of the content area.
fn window(x: usize, y: usize, w: usize, h: usize, title: &str, accent: u32) -> (usize, usize) {
    fb::fill_rect(x + 4, y + 5, w, h, 0x02_03_07); // drop shadow
    // gradient body
    for j in 0..h {
        fb::fill_rect(x, y + j, w, 1, lerp(0x1A_22_33, 0x11_16_24, j * 100 / h));
    }
    fb::fill_rect(x, y, w, 3, accent); // top accent stripe
    fb::fill_rect(x, y + 3, w, 21, 0x0E_13_20); // title bar
    // traffic-light buttons
    fb::fill_rect(x + 10, y + 11, 7, 7, 0xFF_5F_56);
    fb::fill_rect(x + 22, y + 11, 7, 7, 0xFF_BD_2E);
    fb::fill_rect(x + 34, y + 11, 7, 7, 0x27_C9_3F);
    fb::draw_string(x + 50, y + 9, title, WHITE, 1);
    (x + 12, y + 32)
}

/// Composite the desktop onto the framebuffer.
pub fn desktop() {
    // Wallpaper gradient.
    for y in 0..H {
        let t = y * 100 / H;
        fb::fill_rect(0, y, W, 1, lerp(WALL_TOP, WALL_BOT, t));
    }

    // Status bar with gold underline.
    fb::fill_rect(0, 0, W, 28, BAR);
    fb::fill_rect(0, 28, W, 2, GOLD);
    fb::fill_rect(12, 8, 10, 12, GOLD); // mark
    fb::draw_string(28, 6, "AFRISOVERA OS", GOLD, 2);
    let t = sched::ticks();
    let up = (t / 100) as u32;
    fb::draw_string(W - 168, 10, "TONGA WM", DIM, 1);
    // uptime at far right
    let mut buf = [0u8; 12];
    let s = fmt_u32(up, &mut buf);
    fb::draw_string(W - 84, 10, "UP", DIM, 1);
    fb::draw_string(W - 60, 10, s, GREEN, 1);

    // System-status window: a live dashboard of real kernel state.
    let (bx, by) = window(24, 40, 288, 150, "SYSTEM STATUS", 0x1B_3A_6B);
    let mut r = by;
    let mut nb = [0u8; 12];

    // CPU cores online.
    fb::draw_string(bx, r, "CORES", DIM, 1);
    fb::draw_string(bx + 150, r, fmt_u32(crate::cores_online(), &mut nb), GREEN, 1);
    r += 15;

    // Heap usage + a proportional bar.
    let used = heap::used();
    // The GROWN arena (2.740), not the reservation: a bar drawn against a fixed 1 MiB
    // would peg at full the moment the heap grows past it and stay there.
    let size = heap::arena().max(1);
    let mut hb = [0u8; 12];
    fb::draw_string(bx, r, "HEAP", DIM, 1);
    fb::draw_string(bx + 70, r, fmt_u32((used / 1024) as u32, &mut hb), TEXT, 1);
    fb::draw_string(bx + 118, r, "KIB", DIM, 1);
    r += 12;
    let bw = 256usize;
    fb::fill_rect(bx, r, bw, 6, 0x0A_10_1E);
    fb::fill_rect(bx, r, (bw * used / size).max(2).min(bw), 6, GOLD);
    r += 15;

    // Physical RAM.
    let mut fbuf = [0u8; 12];
    fb::draw_string(bx, r, "RAM", DIM, 1);
    fb::draw_string(bx + 70, r, fmt_u32((frame::total_frames() * 4 / 1024) as u32, &mut fbuf), TEXT, 1);
    fb::draw_string(bx + 130, r, "MIB", DIM, 1);
    r += 15;

    // Network / DHCP lease.
    fb::draw_string(bx, r, "NET", DIM, 1);
    if virtio::net_present() {
        let ip = virtio::dhcp_ip();
        if ip != [0u8; 4] {
            let mut ipb = [0u8; 16];
            fb::draw_string(bx + 60, r, fmt_ip(ip, &mut ipb), GREEN, 1);
        } else {
            fb::draw_string(bx + 60, r, "UP (NO LEASE)", TEXT, 1);
        }
    } else {
        fb::draw_string(bx + 60, r, "NONE", DIM, 1);
    }
    r += 15;

    // Devices, boot count, uptime.
    let mut db = [0u8; 12];
    fb::draw_string(bx, r, "DEVICES", DIM, 1);
    fb::draw_string(bx + 150, r, fmt_u32(asili::count() as u32, &mut db), TEXT, 1);
    r += 15;
    let mut bb = [0u8; 12];
    fb::draw_string(bx, r, "BOOTS", DIM, 1);
    fb::draw_string(bx + 150, r, fmt_u32(config::boots(), &mut bb), TEXT, 1);
    r += 15;
    let mut ub = [0u8; 12];
    fb::draw_string(bx, r, "UPTIME", DIM, 1);
    fb::draw_string(bx + 150, r, fmt_u32((sched::ticks() / 100) as u32, &mut ub), TEXT, 1);
    fb::draw_string(bx + 186, r, "S", DIM, 1);

    // Files window (from the RAM VFS).
    let (fx, fy) = window(328, 40, 288, 150, "FILES  (RAMFS)", 0x2B_1B_6B);
    let mut yy = fy;
    let mut any = false;
    for i in 0..vfs::MAX_FILES {
        if vfs::is_used(i) {
            let mut b = [0u8; 12];
            let sz = fmt_u32(vfs::size_of(i) as u32, &mut b);
            fb::draw_string(fx, yy, vfs::name_of(i), TEXT, 1);
            fb::draw_string(fx + 180, yy, sz, DIM, 1);
            yy += 14;
            any = true;
        }
    }
    if !any {
        fb::draw_string(fx, fy, "(no files)", DIM, 1);
    }

    // System window (Asili device manager + heap).
    let (sx, sy) = window(24, 200, 592, 198, "SYSTEM  --  ASILI DEVICE MANAGER", 0x1B_5B_3A);
    fb::draw_string(sx, sy, "NAME", DIM, 1);
    fb::draw_string(sx + 160, sy, "CLASS", DIM, 1);
    fb::draw_string(sx + 320, sy, "STATUS", DIM, 1);
    let mut ry = sy + 16;
    for i in 0..asili::count() {
        if let Some(d) = asili::get(i) {
            fb::draw_string(sx, ry, d.name, TEXT, 1);
            fb::draw_string(sx + 160, ry, d.class.as_str(), DIM, 1);
            fb::draw_string(sx + 320, ry, if d.ok { "OK" } else { "FAIL" }, if d.ok { GREEN } else { 0xE0_50_50 }, 1);
            ry += 14;
        }
    }
    let mut hb = [0u8; 12];
    let used = fmt_u32(heap::used() as u32, &mut hb);
    fb::draw_string(sx, ry + 8, "HEAP USED:", DIM, 1);
    fb::draw_string(sx + 96, ry + 8, used, TEXT, 1);
    fb::draw_string(sx + 160, ry + 8, "BYTES", DIM, 1);

    // Dock.
    let dock_y = 408;
    fb::fill_rect(24, dock_y, 592, 58, 0x0C_10_20);
    fb::fill_rect(24, dock_y, 592, 1, BORDER);
    let icons = [
        ("SH", 0x1B_3A_6B, "SHELL"),
        ("FS", 0x2B_1B_6B, "FILES"),
        ("ST", 0x6B_3A_1B, "STORE"),
        ("AI", 0x1B_5B_3A, "ASILI-AI"),
        ("HW", 0x5B_1B_3A, "DEVICES"),
    ];
    let n = icons.len();
    let slot = 592 / n;
    for (i, (glyph, color, label)) in icons.iter().enumerate() {
        let cx = 24 + slot * i + slot / 2;
        fb::fill_rect(cx - 18, dock_y + 8, 36, 26, *color);
        fb::draw_string(cx - 8, dock_y + 13, glyph, WHITE, 1);
        fb::draw_string(cx - (label.len() * 4), dock_y + 40, label, DIM, 1);
    }
}

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
        (*wi).w = w.min(CW);
        (*wi).h = h.min(CH);
        // Simple grid placement.
        (*wi).x = 30 + (id % 2) * 300;
        (*wi).y = 44 + (id / 2) * 200;
        for p in 0..CW * CH {
            (*wi).buf[p] = 0;
        }
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
        let wi = win(id);
        let ww = (*wi).w;
        let hh = (*wi).h;
        for j in ry..(ry + rh).min(hh) {
            for i in rx..(rx + rw).min(ww) {
                (*wi).buf[j * CW + i] = color;
            }
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
                    let px = x + col;
                    let py = y + row;
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
    // Wallpaper + status bar.
    for y in 0..H {
        let t = y * 100 / H;
        fb::fill_rect(0, y, W, 1, lerp(WALL_TOP, WALL_BOT, t));
    }
    fb::fill_rect(0, 0, W, 28, BAR);
    fb::draw_string(10, 6, "AFRISOVERA OS", GOLD, 2);
    fb::draw_string(W - 232, 10, "TONGA WM  --  APP SURFACES", DIM, 1);

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

/// Format a dotted-quad IPv4 address into `buf`, returning the &str slice.
fn fmt_ip(ip: [u8; 4], buf: &mut [u8; 16]) -> &str {
    let mut i = 0;
    for (k, oct) in ip.iter().enumerate() {
        if k > 0 {
            buf[i] = b'.';
            i += 1;
        }
        let mut v = *oct;
        if v == 0 {
            buf[i] = b'0';
            i += 1;
        } else {
            let mut t = [0u8; 3];
            let mut n = 0;
            while v > 0 {
                t[n] = b'0' + v % 10;
                v /= 10;
                n += 1;
            }
            for j in 0..n {
                buf[i] = t[n - 1 - j];
                i += 1;
            }
        }
    }
    core::str::from_utf8(&buf[..i]).unwrap_or("?")
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
