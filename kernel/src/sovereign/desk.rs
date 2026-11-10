//! The AfriSovera desktop in the Sovereign UI (docs/design/sovereign-ui.md, 10 Nov 2026).
//!
//! The PC builds this file; the phone still builds kernel/src/desk.rs (the 8 Nov desktop), so
//! the ARM image stays byte-identical until the phone moves over (sovereign-ui.md section 9).
//! Behaviour (input ring, windows, focus, `desk` commands, the apps' data) is the same as
//! desk.rs; the look is new: the Spine down the left edge (monument button, app tiles with
//! beads, status and clock), cut corners instead of round ones, each app's woven cloth as its
//! focus mark, the Circle launcher, and the mud-cloth wallpaper.
//!
//! Input never draws inside an interrupt: mouse and keyboard handlers `post` small events to
//! a lock-free ring, and `idle()` (called by the shell before it sleeps on `wfi`) drains the
//! ring, hit-tests, updates the state and redraws in mainline context. Every action is also a
//! shell command (`desk ...`), so the same code is driven by a mouse, a touch, a keyboard or a
//! test script on either kernel.

use crate::framebuffer as fb;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

#[path = "paint.rs"]
mod paint; // the theme and every drawing function (sovereign-ui.md section 14)

/// The desktop's size in logical pixels: the PC canvas (kernel/src/pc/canvas.rs), the screen's
/// own size over the UI scale (sovereign-ui.md section 13), read at every use so `screen ui`
/// re-lays the desktop out.
#[allow(non_snake_case)]
fn W() -> i32 {
    crate::canvas::width() as i32
}
#[allow(non_snake_case)]
fn H() -> i32 {
    crate::canvas::height() as i32
}

// ---------------------------------------------------------------------------------------
// Palette: Earth and Night (sovereign-ui.md section 4). Every colour on screen comes from
// this table.
// ---------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------
// Layout (section 6): derived from W/H and the Spine, never a literal screen size.
// ---------------------------------------------------------------------------------------
const SPINE_P: i32 = 44; // pointer Spine: 40 px of night + a 4 px woven edge
const SPINE_T: i32 = 56; // touch Spine: room for 44 px tiles
const WEAVE: i32 = 4; // the woven edge
const TOP: i32 = 0; // no top bar: windows keep the whole height
const TITLE_P: i32 = 26; // pointer lintel: a 6 px cloth + 20 px for the title (review S1)
const TITLE_T: i32 = 34; // touch lintel
const TILE_P: i32 = 32; // pointer Spine tile
const TILE_T: i32 = 44; // touch Spine tile (44 px minimum touch target)
const CUT: i32 = 8; // a window's cut corners (top left, bottom right)

#[derive(Clone, Copy, PartialEq, Eq)]
enum App {
    Files = 0,
    Terminal = 1,
    Settings = 2,
    About = 3,
}
const NAPPS: usize = 4;
const APPS: [App; NAPPS] = [App::Files, App::Terminal, App::Settings, App::About];

impl App {
    fn name(self) -> &'static str {
        ["Files", "Terminal", "Settings", "About"][self as usize]
    }
    fn id(self) -> &'static str {
        ["files", "terminal", "settings", "about"][self as usize]
    }
    fn from(s: &str) -> Option<App> {
        APPS.iter().copied().find(|a| a.id() == s || (s.len() >= 3 && a.id().starts_with(s)))
    }
}

#[derive(Clone, Copy)]
struct Win {
    open: bool,
    min: bool,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}
const WIN0: Win = Win { open: false, min: false, x: 0, y: 0, w: 0, h: 0 };

/// Default pointer-profile windows (sovereign-ui.md section 14): each app's centre as an offset
/// from the middle of the free space right of the Spine, and its size: a cascade round the
/// middle at any screen size, every lintel on the screen.
const HOME: [(i32, i32, i32, i32); NAPPS] = [(-150, -70, 372, 250), (40, 50, 410, 260), (-70, -30, 380, 300), (110, -20, 330, 300)];

/// Where app a's window opens: (x, y, w, h), clamped to the screen.
fn home(a: usize) -> (i32, i32, i32, i32) {
    let (dx, dy, w, h) = HOME[a];
    let sw = SPINE_P;
    let w = w.min(W() - sw - 8);
    let h = h.min(H() - TOP - 8);
    let (cx, cy) = (sw + (W() - sw) / 2, TOP + (H() - TOP) / 2);
    ((cx + dx - w / 2).clamp(sw + 4, W() - w), (cy + dy - h / 2).clamp(TOP, H() - h), w, h)
}

const NNOTE: usize = 8;
const NOTE_LEN: usize = 44;

struct State {
    shown: bool,
    touch: bool,
    wins: [Win; NAPPS],
    z: [usize; NAPPS], // stacking order of the open, not-minimised windows: z[0] bottom
    nz: usize,
    launcher: bool,
    notes_open: bool,
    notes: [[u8; NOTE_LEN]; NNOTE],
    note_len: [usize; NNOTE],
    nnotes: usize,
    unread: usize,
    toast_until: u64,
    drag: Option<(usize, i32, i32)>,
    drag_moved: bool,
    files_sel: Option<usize>,
    wallpaper: bool,
    clock24: bool,
    hover_close: Option<usize>,
    px: i32,
    py: i32,
    last_minute: u64,
    frames: u64,
    dock_clicks: u64,
    welcomed: bool,
    sel: usize, // the app chosen in the Circle by the keyboard walk
}

static mut S: State = State {
    shown: false,
    touch: cfg!(target_arch = "aarch64"),
    wins: [WIN0; NAPPS],
    z: [0; NAPPS],
    nz: 0,
    launcher: false,
    notes_open: false,
    notes: [[0; NOTE_LEN]; NNOTE],
    note_len: [0; NNOTE],
    nnotes: 0,
    unread: 0,
    toast_until: 0,
    drag: None,
    drag_moved: false,
    files_sel: None,
    wallpaper: true,
    clock24: true,
    hover_close: None,
    px: 0,
    py: 0,
    last_minute: u64::MAX,
    frames: 0,
    dock_clicks: 0,
    welcomed: false,
    sel: 0,
};

/// SAFETY: the desktop state is touched only in mainline context (shell commands and
/// `idle`); interrupt handlers reach it only through the atomic event ring below.
fn st() -> &'static mut State {
    unsafe { &mut *core::ptr::addr_of_mut!(S) }
}

static SHOWN: AtomicBool = AtomicBool::new(false);
static DIRTY: AtomicBool = AtomicBool::new(false);
static TERM_DIRTY: AtomicBool = AtomicBool::new(false);
/// While set, console text is not teed to the Terminal: the desktop's own log lines from
/// pointer and keyboard events would otherwise land in the middle of the shell's prompt line.
static TERM_MUTE: AtomicBool = AtomicBool::new(false);
static LAST_CHECK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

// ---------------------------------------------------------------------------------------
// The event ring: interrupt handlers post, `idle` drains. Single producer at a time (the
// timer interrupt, which never re-enters itself), single consumer (the mainline).
// Encoding: kind << 28 | x << 14 | y  (x, y < 16384).
// ---------------------------------------------------------------------------------------
const EV_DOWN: u32 = 1;
const EV_UP: u32 = 2;
const EV_MOVE: u32 = 3;
const EV_KEY: u32 = 4;

pub const KEY_SUPER: u32 = 1;
pub const KEY_ALT_TAB: u32 = 2;
pub const KEY_ALT_F4: u32 = 3;
pub const KEY_ESC: u32 = 4;
pub const KEY_SUPER_LEFT: u32 = 5;
pub const KEY_SUPER_RIGHT: u32 = 6;
pub const KEY_SUPER_UP: u32 = 7;
pub const KEY_SUPER_DOWN: u32 = 8;
/// The keyboard walk (sovereign-ui.md section 14): arrows and Enter while the Circle is open,
/// Super+1..4 for the Spine's apps (KEY_SUPER_1 + n).
pub const KEY_UP: u32 = 9;
pub const KEY_DOWN: u32 = 10;
pub const KEY_LEFT: u32 = 11;
pub const KEY_RIGHT: u32 = 12;
pub const KEY_ENTER: u32 = 13;
pub const KEY_SUPER_1: u32 = 16;

/// The Circle is open, so the keyboard drivers send arrows and Enter here, not to the shell.
static WALK: AtomicBool = AtomicBool::new(false);
/// True while arrows and Enter belong to the desktop (read by the keyboard interrupt handlers).
pub fn walking() -> bool {
    WALK.load(Ordering::Relaxed) && SHOWN.load(Ordering::Relaxed)
}

const RING: usize = 128;
static EVQ: [AtomicU32; RING] = [const { AtomicU32::new(0) }; RING];
static EV_HEAD: AtomicUsize = AtomicUsize::new(0);
static EV_TAIL: AtomicUsize = AtomicUsize::new(0);
static EV_DROPS: AtomicU32 = AtomicU32::new(0);

fn post(kind: u32, x: i32, y: i32) {
    if !SHOWN.load(Ordering::Relaxed) {
        return;
    }
    let t = EV_TAIL.load(Ordering::Relaxed);
    let next = (t + 1) % RING;
    if next == EV_HEAD.load(Ordering::Acquire) {
        EV_DROPS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let v = kind << 28 | (x.clamp(0, 16383) as u32) << 14 | y.clamp(0, 16383) as u32;
    EVQ[t].store(v, Ordering::Relaxed);
    EV_TAIL.store(next, Ordering::Release);
}

/// The desktop is on screen (it takes pointer and shortcut events).
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}
#[allow(dead_code)] // the ARM kernel has no pointer driver yet; `desk tap` drives it there
/// A pointer button went down at (x, y) of the framebuffer (a click or a tap).
pub fn post_down(x: i32, y: i32) {
    post(EV_DOWN, x, y);
}
#[allow(dead_code)]
/// The pointer button went up.
pub fn post_up(x: i32, y: i32) {
    post(EV_UP, x, y);
}
#[allow(dead_code)]
/// The pointer moved to (x, y).
pub fn post_move(x: i32, y: i32) {
    post(EV_MOVE, x, y);
}
#[allow(dead_code)]
/// A desktop shortcut (`KEY_*`).
pub fn post_key(k: u32) {
    post(EV_KEY, k as i32, 0);
}

#[allow(dead_code)]
/// Clicks on dock tiles since boot (`lsusb` reports them on the PC).
pub fn dock_clicks() -> u64 {
    st().dock_clicks
}

// ---------------------------------------------------------------------------------------
// The Terminal's text: every console write is teed here (framebuffer::console_write).
// ---------------------------------------------------------------------------------------
const TL: usize = 48;
const TC: usize = 96;
static mut TERM: [[u8; TC]; TL] = [[0; TC]; TL];
static mut TLEN: [u8; TL] = [0; TL];
static mut TROW: usize = 0; // index of the line being written (ring)
static mut TLINES: usize = 1; // lines written so far (capped at TL)

/// Tee of the kernel console. Called with interrupts masked (uart print), so it never
/// interleaves with itself; it only appends bytes and raises a flag.
pub fn term_feed(s: &str) {
    if TERM_MUTE.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: callers hold interrupts masked; single writer.
    unsafe {
        let term = &mut *core::ptr::addr_of_mut!(TERM);
        let tlen = &mut *core::ptr::addr_of_mut!(TLEN);
        for &c in s.as_bytes() {
            match c {
                b'\n' => {
                    TROW = (TROW + 1) % TL;
                    tlen[TROW] = 0;
                    if TLINES < TL {
                        TLINES += 1;
                    }
                }
                0x08 => {
                    if tlen[TROW] > 0 {
                        tlen[TROW] -= 1;
                    }
                }
                0x20..=0x7E => {
                    let n = tlen[TROW] as usize;
                    if n < TC {
                        term[TROW][n] = c;
                        tlen[TROW] += 1;
                    }
                }
                _ => {}
            }
        }
    }
    if SHOWN.load(Ordering::Relaxed) {
        TERM_DIRTY.store(true, Ordering::Relaxed);
    }
}

/// The k-th most recent terminal line (0 = the line being typed).
fn term_line(k: usize) -> &'static [u8] {
    // SAFETY: read in mainline; a concurrent append can only lengthen the current line.
    unsafe {
        if k >= TLINES {
            return &[];
        }
        let i = (TROW + TL - k) % TL;
        let term = &*core::ptr::addr_of!(TERM);
        &term[i][..(TLEN[i] as usize).min(TC)]
    }
}

// ---------------------------------------------------------------------------------------
// Drawing primitives (clipped; signed coordinates so windows can be partly off screen).
// ---------------------------------------------------------------------------------------

/// Small decimal formatter into a caller's buffer.
fn num(mut v: u64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    if v == 0 {
        i -= 1;
        buf[i] = b'0';
    }
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[i..]
}

/// A tiny line builder for labels ("12 KiB", "10.0.2.15").
struct Line {
    b: [u8; 64],
    n: usize,
}
impl Line {
    fn new() -> Line {
        Line { b: [0; 64], n: 0 }
    }
    fn s(&mut self, s: &[u8]) -> &mut Line {
        for &c in s {
            if self.n < 64 {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        self
    }
    fn u(&mut self, v: u64) -> &mut Line {
        let mut t = [0u8; 20];
        let d = num(v, &mut t);
        self.s(d)
    }
    fn two(&mut self, v: u64) -> &mut Line {
        if v < 10 {
            self.s(b"0");
        }
        self.u(v)
    }
    fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

// ---------------------------------------------------------------------------------------
// The brand monument (the splash's twin gold sails and needle), at any height.
// ---------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------
// Icons (section 5), drawn at any size s >= 12.
// ---------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------
// Geometry shared by drawing and hit-testing.
// ---------------------------------------------------------------------------------------
fn title_h() -> i32 {
    if st().touch {
        TITLE_T
    } else {
        TITLE_P
    }
}
fn tile() -> i32 {
    if st().touch {
        TILE_T
    } else {
        TILE_P
    }
}
/// The Spine's width (its woven edge included): windows live right of it.
fn spine_w() -> i32 {
    if st().touch {
        SPINE_T
    } else {
        SPINE_P
    }
}

/// The Spine (x, y, w, h): the whole left edge.
fn dock_box() -> (i32, i32, i32, i32) {
    (0, 0, spine_w(), H())
}

/// Tile i of the Spine (0 = the Circle button, 1.. = the apps): (x, y, size).
fn dock_tile(i: usize) -> (i32, i32, i32) {
    let s = tile();
    let body = spine_w() - WEAVE;
    let x = (body - s) / 2;
    if i == 0 {
        return (x, 6, s);
    }
    // a gap and a separator line under the Circle button, then the apps
    (x, 6 + s + 14 + (i as i32 - 1) * (s + 8), s)
}

/// The window rectangle for an app (touch: maximised right of the Spine).
fn win_rect(a: usize) -> (i32, i32, i32, i32) {
    let s = st();
    if s.touch {
        let sw = spine_w();
        return (sw, TOP, W() - sw, H() - TOP);
    }
    let w = s.wins[a];
    (w.x, w.y, w.w, w.h)
}

/// A window's cut corners (top left, bottom right): none when maximised (touch).
fn win_cut() -> i32 {
    if st().touch {
        0
    } else {
        CUT
    }
}

/// The two title-bar buttons of window a: (close box, minimise box) as (x, y, w, h).
fn title_buttons(a: usize) -> ((i32, i32, i32, i32), (i32, i32, i32, i32)) {
    let (x, y, w, _) = win_rect(a);
    let th = title_h();
    let bw = if st().touch { 44 } else { 22 }; // touch: a 44 px wide target
    ((x + w - bw - 4, y + 5, bw, th - 7), (x + w - 2 * bw - 8, y + 5, bw, th - 7))
}

fn inside(px: i32, py: i32, r: (i32, i32, i32, i32)) -> bool {
    px >= r.0 && px < r.0 + r.2 && py >= r.1 && py < r.1 + r.3
}

/// The Circle: an octagon (x, y, size, size) centred in the space right of the Spine.
fn launcher_box() -> (i32, i32, i32, i32) {
    let sw = spine_w();
    let size = if st().touch { 340 } else { 300 }.min(H() - 90).min(W() - sw - 16);
    (sw + (W() - sw - size) / 2, (H() - size - 56) / 2, size, size)
}

/// The strip under the Circle: this device, and how to close.
fn circle_info_box() -> (i32, i32, i32, i32) {
    let (lx, ly, lw, lh) = launcher_box();
    (lx, ly + lh + 8, lw, 44)
}

/// cos and sin of 16 directions round the circle, starting at the top, clockwise, x1000.
const DIRS: [(i32, i32); 16] = [
    (0, -1000), (383, -924), (707, -707), (924, -383), (1000, 0), (924, 383), (707, 707), (383, 924),
    (0, 1000), (-383, 924), (-707, 707), (-924, 383), (-1000, 0), (-924, -383), (-707, -707), (-383, -924),
];

/// App i's tile in the Circle: standing in a ring round the monument (x, y, size).
fn launcher_tile(i: usize) -> (i32, i32, i32) {
    let (lx, ly, lw, lh) = launcher_box();
    let s = if st().touch { 52 } else { 44 };
    let (cx, cy) = (lx + lw / 2, ly + lh / 2 - 6);
    let r = lw * 33 / 100;
    let (c, si) = DIRS[(i * 16 / NAPPS) % 16];
    (cx + r * c / 1000 - s / 2, cy + r * si / 1000 - s / 2, s)
}

/// The notification list opens beside the Spine, at its foot (by the bell).
fn notes_box() -> (i32, i32, i32, i32) {
    let w = 280;
    let h = 30 + 18 * NNOTE as i32;
    (spine_w() + 6, H() - h - 8, w, h)
}

/// The bell at the Spine's foot, above the clock.
fn bell_box() -> (i32, i32, i32, i32) {
    (0, H() - 52, spine_w() - WEAVE, 22)
}
/// The Circle button at the top of the Spine.
fn brand_box() -> (i32, i32, i32, i32) {
    let (x, y, s) = dock_tile(0);
    (x - 2, y - 2, s + 4, s + 4)
}

// ---------------------------------------------------------------------------------------
// Window state.
// ---------------------------------------------------------------------------------------
fn focused() -> Option<usize> {
    let s = st();
    if s.nz == 0 {
        None
    } else {
        Some(s.z[s.nz - 1])
    }
}

fn z_remove(a: usize) {
    let s = st();
    if let Some(k) = s.z[..s.nz].iter().position(|&v| v == a) {
        for j in k..s.nz - 1 {
            s.z[j] = s.z[j + 1];
        }
        s.nz -= 1;
    }
}

fn raise(a: usize) {
    z_remove(a);
    let s = st();
    s.z[s.nz] = a;
    s.nz += 1;
}

fn open(a: usize) {
    let s = st();
    if !s.wins[a].open {
        let (x, y, w, h) = home(a);
        s.wins[a] = Win { open: true, min: false, x, y, w, h };
        if a == App::Files as usize {
            s.files_sel = None;
        }
    }
    s.wins[a].min = false;
    raise(a);
}

fn close(a: usize) {
    st().wins[a].open = false;
    st().wins[a].min = false;
    z_remove(a);
}

fn minimise(a: usize) {
    if st().wins[a].open {
        st().wins[a].min = true;
        z_remove(a);
    }
}

/// A dock tile or launcher tile: open it, focus it, restore it; a focused one minimises.
fn activate(a: usize) {
    let s = st();
    if s.wins[a].open && !s.wins[a].min && focused() == Some(a) && !s.touch {
        minimise(a);
        log(b"minimise ", a);
    } else {
        let was = s.wins[a].open;
        open(a);
        log(if was { b"focus " } else { b"open " }, a);
    }
}

fn move_by(a: usize, dx: i32, dy: i32) {
    let s = st();
    if s.touch {
        return;
    }
    let w = &mut s.wins[a];
    w.x = (w.x + dx).clamp(SPINE_P - (w.w - 48), W() - 48); // a grip of the lintel stays clear of the Spine
    w.y = (w.y + dy).clamp(TOP, H() - TITLE_P);
}

fn log(what: &[u8], a: usize) {
    let mut l = Line::new();
    l.s(what).s(APPS[a].id().as_bytes());
    crate::kprintln!("[desk] {}", core::str::from_utf8(l.get()).unwrap_or("?"));
}

/// Post a notification (shown as a toast, kept in the bell's list).
pub fn notify(msg: &str) {
    let s = st();
    let i = s.nnotes % NNOTE;
    let b = msg.as_bytes();
    let n = b.len().min(NOTE_LEN);
    s.notes[i][..n].copy_from_slice(&b[..n]);
    s.note_len[i] = n;
    s.nnotes += 1;
    s.unread = (s.unread + 1).min(NNOTE);
    s.toast_until = crate::sched::ticks() + 400;
    DIRTY.store(true, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------
// Input handling (mainline).
// ---------------------------------------------------------------------------------------
fn down(px: i32, py: i32) {
    let s = st();
    s.px = px;
    s.py = py;
    s.drag = None;
    s.drag_moved = false;
    DIRTY.store(true, Ordering::Relaxed);

    // 1. the Circle is modal-ish: a tile opens, outside closes it
    if s.launcher {
        if inside(px, py, launcher_box()) || inside(px, py, circle_info_box()) {
            for i in 0..NAPPS {
                let (tx, ty, ts) = launcher_tile(i);
                if inside(px, py, (tx, ty, ts, ts + 14)) {
                    s.launcher = false;
                    open(i);
                    log(b"open ", i);
                }
            }
            return;
        }
        s.launcher = false;
        let (tx, ty, ts) = dock_tile(0);
        if inside(px, py, (tx, ty, ts, ts)) || inside(px, py, brand_box()) {
            s.dock_clicks += inside(px, py, (tx, ty, ts, ts)) as u64;
            crate::kprintln!("[desk] launcher closed");
            return;
        }
    }
    // 2. the notification list
    if s.notes_open {
        if inside(px, py, notes_box()) {
            return;
        }
        s.notes_open = false;
        if inside(px, py, bell_box()) {
            return;
        }
    }
    // 3. the Spine: the bell, the Circle button, the app tiles
    if inside(px, py, dock_box()) {
        if inside(px, py, bell_box()) {
            s.notes_open = true;
            s.unread = 0;
            crate::kprintln!("[desk] notifications open");
            return;
        }
        for i in 0..=NAPPS {
            let (_, ty, ts) = dock_tile(i);
            if inside(px, py, (0, ty - 4, spine_w() - WEAVE, ts + 8)) {
                s.dock_clicks += 1;
                if i == 0 {
                    s.launcher = true;
                    crate::kprintln!("[desk] launcher open");
                } else {
                    activate(i - 1);
                }
            }
        }
        return;
    }
    // 5. windows, front to back
    for k in (0..s.nz).rev() {
        let a = s.z[k];
        let r = win_rect(a);
        if !inside(px, py, r) {
            continue;
        }
        let was_focused = focused() == Some(a);
        raise(a);
        let (cl, mn) = title_buttons(a);
        if inside(px, py, cl) {
            close(a);
            log(b"close ", a);
        } else if inside(px, py, mn) && !s.touch {
            minimise(a);
            log(b"minimise ", a);
        } else if py < r.1 + title_h() {
            if !s.touch {
                s.drag = Some((a, px - r.0, py - r.1));
            }
            if !was_focused {
                log(b"focus ", a);
            }
        } else {
            if !was_focused {
                log(b"focus ", a);
            }
            body_click(a, px - r.0, py - r.1 - title_h());
        }
        return;
    }
}

fn pointer_move(px: i32, py: i32) {
    let s = st();
    s.px = px;
    s.py = py;
    if let Some((a, ox, oy)) = s.drag {
        let (x, y, _, _) = win_rect(a);
        move_by(a, px - ox - x, py - oy - y);
        s.drag_moved = true;
        DIRTY.store(true, Ordering::Relaxed);
    }
    // the close button under the pointer turns red
    let mut hover = None;
    if let Some(a) = focused() {
        if inside(px, py, title_buttons(a).0) {
            hover = Some(a);
        }
    }
    if hover != s.hover_close {
        s.hover_close = hover;
        DIRTY.store(true, Ordering::Relaxed);
    }
}

fn up() {
    let s = st();
    if let Some((a, _, _)) = s.drag.take() {
        if !core::mem::take(&mut s.drag_moved) {
            return;
        }
        let w = s.wins[a];
        crate::kprintln!("[desk] moved {} to {},{}", APPS[a].id(), w.x, w.y);
    }
}

fn key(k: u32) {
    let s = st();
    DIRTY.store(true, Ordering::Relaxed);
    match k {
        KEY_SUPER => {
            s.launcher = !s.launcher;
            s.notes_open = false;
            crate::kprintln!("[desk] launcher {}", if s.launcher { "open" } else { "closed" });
        }
        KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT if s.launcher => {
            // walk round the ring: Right and Down clockwise, Left and Up back
            let step = if k == KEY_RIGHT || k == KEY_DOWN { 1 } else { NAPPS - 1 };
            s.sel = (s.sel + step) % NAPPS;
            crate::kprintln!("[desk] circle: {}", APPS[s.sel].id());
        }
        KEY_ENTER if s.launcher => {
            s.launcher = false;
            let a = s.sel;
            let was = s.wins[a].open;
            open(a);
            log(if was { b"focus " } else { b"open " }, a);
        }
        _ if (KEY_SUPER_1..KEY_SUPER_1 + NAPPS as u32).contains(&k) => {
            s.launcher = false;
            s.notes_open = false;
            activate((k - KEY_SUPER_1) as usize); // as a click on the Spine's tile
        }
        KEY_ESC => {
            if s.launcher || s.notes_open {
                s.launcher = false;
                s.notes_open = false;
                crate::kprintln!("[desk] launcher closed");
            }
        }
        KEY_ALT_TAB => {
            if s.nz > 1 {
                let a = s.z[0]; // the window at the bottom comes to the front: a full cycle
                raise(a);
                log(b"focus ", a);
            } else if let Some(a) = (0..NAPPS).find(|&a| s.wins[a].open && s.wins[a].min) {
                open(a); // the only window is minimised: bring it back
                log(b"focus ", a);
            }
        }
        KEY_ALT_F4 => {
            if let Some(a) = focused() {
                close(a);
                log(b"close ", a);
            }
        }
        KEY_SUPER_DOWN => {
            if let Some(a) = focused() {
                minimise(a);
                log(b"minimise ", a);
            }
        }
        KEY_SUPER_LEFT | KEY_SUPER_RIGHT | KEY_SUPER_UP => {
            if let Some(a) = focused() {
                let (dx, dy) = match k {
                    KEY_SUPER_LEFT => (-16, 0),
                    KEY_SUPER_RIGHT => (16, 0),
                    _ => (0, -16),
                };
                move_by(a, dx, dy);
                let w = s.wins[a];
                crate::kprintln!("[desk] moved {} to {},{}", APPS[a].id(), w.x, w.y);
            }
        }
        _ => {}
    }
}

/// A click inside an app's body at (bx, by) relative to the body's top-left.
fn body_click(a: usize, bx: i32, by: i32) {
    let s = st();
    match APPS[a] {
        App::Files => {
            if bx < files_split(a) && by >= 22 {
                let row = ((by - 22) / file_row_h()) as usize;
                if let Some(i) = file_at(row) {
                    s.files_sel = Some(i);
                    crate::kprintln!("[desk] files: selected {}", crate::vfs::name_of(i));
                }
            }
        }
        App::Settings => {
            for (k, &r) in settings_rows().iter().enumerate() {
                if by >= r && by < r + 20 {
                    match k {
                        0 => set_touch(bx >= settings_seg_x(a) + 64),
                        1 => s.wallpaper = !s.wallpaper,
                        2 => s.clock24 = !s.clock24,
                        _ => {}
                    }
                    if k == 1 || k == 2 {
                        crate::kprintln!(
                            "[desk] settings: {} {}",
                            if k == 1 { "wallpaper" } else { "24-hour clock" },
                            if (k == 1 && s.wallpaper) || (k == 2 && s.clock24) { "on" } else { "off" }
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn set_touch(on: bool) {
    let s = st();
    if s.touch != on {
        s.touch = on;
        s.drag = None;
        crate::kprintln!("[desk] input profile: {}", if on { "touch" } else { "pointer" });
    }
    DIRTY.store(true, Ordering::Relaxed);
}

/// The i-th used VFS file, by row.
fn file_at(row: usize) -> Option<usize> {
    (0..crate::vfs::MAX_FILES).filter(|&i| crate::vfs::is_used(i)).nth(row)
}

/// A Files row: 16 px for a pointer, 40 px for a finger (design review D2).
fn file_row_h() -> i32 {
    if st().touch {
        40
    } else {
        16
    }
}

fn files_split(a: usize) -> i32 {
    let (_, _, w, _) = win_rect(a);
    (w * 46 / 100).max(150)
}

/// Settings: the y of each interactive row in the body (profile, wallpaper, clock).
fn settings_rows() -> [i32; 3] {
    [40, 92, 114]
}
fn settings_seg_x(a: usize) -> i32 {
    let (_, _, w, _) = win_rect(a);
    w - 150
}

// ---------------------------------------------------------------------------------------
// The frame.
// ---------------------------------------------------------------------------------------
fn clock_secs() -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: the PL031 data register, as `date` reads it.
        unsafe { core::ptr::read_volatile(crate::board::RTC_BASE as *const u32) as u64 }
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::cmos::epoch()
    }
}

// ---------------------------------------------------------------------------------------
// Entry points.
// ---------------------------------------------------------------------------------------

/// `gui`: put the desktop on screen.
pub fn show() {
    fb::set_console(false);
    let s = st();
    s.shown = true;
    SHOWN.store(true, Ordering::Relaxed);
    // drop events queued while hidden
    EV_HEAD.store(EV_TAIL.load(Ordering::Acquire), Ordering::Release);
    if !s.welcomed {
        s.welcomed = true;
        notify("Welcome to AfriSovera");
    }
    paint::draw();
}

/// `clear`: the text console comes back.
pub fn hide() {
    st().shown = false;
    st().drag = None;
    SHOWN.store(false, Ordering::Relaxed);
}

/// Called by the shell before it sleeps: drain input events, redraw when something changed.
pub fn idle() {
    if !SHOWN.load(Ordering::Relaxed) {
        return;
    }
    let mut moved: Option<(i32, i32)> = None;
    TERM_MUTE.store(true, Ordering::Relaxed);
    loop {
        let h = EV_HEAD.load(Ordering::Relaxed);
        if h == EV_TAIL.load(Ordering::Acquire) {
            break;
        }
        let v = EVQ[h].load(Ordering::Relaxed);
        EV_HEAD.store((h + 1) % RING, Ordering::Release);
        let (kind, x, y) = (v >> 28, ((v >> 14) & 0x3FFF) as i32, (v & 0x3FFF) as i32);
        if kind == EV_MOVE {
            moved = Some((x, y)); // consecutive moves collapse into the last
            continue;
        }
        if let Some((mx, my)) = moved.take() {
            pointer_move(mx, my);
        }
        match kind {
            EV_DOWN => down(x, y),
            EV_UP => {
                pointer_move(x, y);
                up();
            }
            EV_KEY => key(x as u32),
            _ => {}
        }
    }
    if let Some((mx, my)) = moved {
        pointer_move(mx, my);
    }
    TERM_MUTE.store(false, Ordering::Relaxed);
    let s = st();
    if TERM_DIRTY.swap(false, Ordering::Relaxed) && s.wins[App::Terminal as usize].open && !s.wins[App::Terminal as usize].min {
        DIRTY.store(true, Ordering::Relaxed);
    }
    let t = crate::sched::ticks();
    if s.toast_until != 0 && t >= s.toast_until {
        s.toast_until = 0;
        DIRTY.store(true, Ordering::Relaxed);
    }
    if t / 100 != LAST_CHECK.swap(t / 100, Ordering::Relaxed) && clock_secs() / 60 != s.last_minute {
        DIRTY.store(true, Ordering::Relaxed);
    }
    if DIRTY.swap(false, Ordering::Relaxed) {
        paint::draw();
    }
    WALK.store(s.launcher, Ordering::Relaxed);
    // the copy to the screen happens here, in mainline, never in the timer interrupt
    crate::screen::flush();
}

/// The canvas changed size (`screen ui N`): keep every window's lintel on the screen and
/// redraw (sovereign-ui.md section 13).
pub fn relayout() {
    let s = st();
    for a in 0..NAPPS {
        let w = &mut s.wins[a];
        if w.open {
            w.w = w.w.min(W() - SPINE_P - 8);
            w.h = w.h.min(H() - TOP - 8);
            w.x = w.x.clamp(SPINE_P - (w.w - 48), W() - 48);
            w.y = w.y.clamp(TOP, H() - TITLE_P);
        }
    }
    s.drag = None;
    redraw_now();
}

fn redraw_now() {
    DIRTY.store(false, Ordering::Relaxed);
    WALK.store(st().launcher, Ordering::Relaxed);
    if st().shown {
        paint::draw();
    }
}

/// `desk ...`: every desktop action as a shell command (section 6).
pub fn command(rest: &str) {
    let mut it = rest.split_whitespace();
    let verb = it.next().unwrap_or("list");
    let arg = it.next().unwrap_or("");
    let num_arg = |s: Option<&str>| s.and_then(|v| v.parse::<i32>().ok());
    let s = st();
    match verb {
        "open" | "close" | "min" | "focus" => match App::from(arg) {
            Some(a) => {
                let a = a as usize;
                match verb {
                    "open" | "focus" => {
                        let was = s.wins[a].open;
                        open(a);
                        log(if was { b"focus " } else { b"open " }, a);
                    }
                    "close" => {
                        if s.wins[a].open {
                            close(a);
                            log(b"close ", a);
                        } else {
                            crate::kprintln!("[desk] {} is not open", APPS[a].id());
                        }
                    }
                    _ => {
                        if s.wins[a].open && !s.wins[a].min {
                            minimise(a);
                            log(b"minimise ", a);
                        } else {
                            crate::kprintln!("[desk] {} is not on screen", APPS[a].id());
                        }
                    }
                }
            }
            None => crate::kprintln!("desk: no app '{}'. Apps: files terminal settings about", arg),
        },
        "notes" => {
            // the bell's list, as a click on the bell (scripts and screenshots: its place
            // depends on the screen's height)
            s.notes_open = !s.notes_open;
            s.launcher = false;
            if s.notes_open {
                s.unread = 0;
                crate::kprintln!("[desk] notifications open");
            } else {
                crate::kprintln!("[desk] notifications closed");
            }
        }
        "perf" => {
            // how long the desktop takes on this machine (the 2012 laptop target, sovereign-ui.md 8)
            // SAFETY: scalar reads.
            let (dl, dw) = unsafe { paint::DRAW_US };
            let (cl, cw, n) = crate::screen::copy_perf();
            crate::kprintln!(
                "[desk] perf: {}x{} at UI scale {}x; draw {} us (worst {} us) over {} frame(s); copy to the screen {} us (worst {} us) over {} copies",
                W(), H(), crate::canvas::scale(), dl, dw, s.frames, cl, cw, n
            );
            return;
        }
        "select" => {
            // select a file in Files by name (scripts and screenshots, whatever the layout)
            match (0..crate::vfs::MAX_FILES).find(|&i| crate::vfs::is_used(i) && crate::vfs::name_of(i) == arg) {
                Some(i) => {
                    s.files_sel = Some(i);
                    crate::kprintln!("[desk] files: selected {}", arg);
                }
                None => crate::kprintln!("desk: no file '{}'", arg),
            }
        }
        "move" => match (App::from(arg), num_arg(it.next()), num_arg(it.next())) {
            (Some(a), Some(dx), Some(dy)) if s.wins[a as usize].open => {
                let a = a as usize;
                move_by(a, dx, dy);
                let w = s.wins[a];
                crate::kprintln!("[desk] moved {} to {},{}", APPS[a].id(), w.x, w.y);
            }
            _ => crate::kprintln!("desk: move <open app> <dx> <dy>"),
        },
        "next" => key(KEY_ALT_TAB),
        "launcher" => key(KEY_SUPER),
        "key" => match arg {
            "super" => key(KEY_SUPER),
            "alt-tab" => key(KEY_ALT_TAB),
            "alt-f4" => key(KEY_ALT_F4),
            "esc" => key(KEY_ESC),
            "super-left" => key(KEY_SUPER_LEFT),
            "super-right" => key(KEY_SUPER_RIGHT),
            "super-up" => key(KEY_SUPER_UP),
            "super-down" => key(KEY_SUPER_DOWN),
            "up" => key(KEY_UP),
            "down" => key(KEY_DOWN),
            "left" => key(KEY_LEFT),
            "right" => key(KEY_RIGHT),
            "enter" => key(KEY_ENTER),
            "super-1" | "super-2" | "super-3" | "super-4" => key(KEY_SUPER_1 + (arg.as_bytes()[6] - b'1') as u32),
            _ => crate::kprintln!("desk: keys are super alt-tab alt-f4 esc super-left super-right super-up super-down up down left right enter super-1..4"),
        },
        "tap" => match (num_arg(Some(arg)), num_arg(it.next())) {
            (Some(x), Some(y)) => {
                down(x.clamp(0, W() - 1), y.clamp(0, H() - 1));
                up();
            }
            _ => crate::kprintln!("desk: tap <x> <y>"),
        },
        "drag" => match (num_arg(Some(arg)), num_arg(it.next()), num_arg(it.next()), num_arg(it.next())) {
            (Some(x), Some(y), Some(dx), Some(dy)) => {
                down(x, y);
                pointer_move((x + dx).clamp(0, W() - 1), (y + dy).clamp(0, H() - 1));
                up();
            }
            _ => crate::kprintln!("desk: drag <x> <y> <dx> <dy>"),
        },
        "mode" => match arg {
            "touch" => set_touch(true),
            "pointer" => set_touch(false),
            _ => crate::kprintln!("[desk] input profile: {}", if s.touch { "touch" } else { "pointer" }),
        },
        "notify" => {
            let msg = rest.trim_start()["notify".len()..].trim();
            if msg.is_empty() {
                crate::kprintln!("desk: notify <text>");
            } else {
                notify(msg);
                crate::kprintln!("[desk] notified: {}", msg);
            }
        }
        "list" => {
            crate::kprintln!(
                "[desk] {} profile, {}; {} frame(s) drawn; {} dock click(s); {} notice(s); {} event(s) dropped",
                if s.touch { "touch" } else { "pointer" },
                if s.shown { "on screen" } else { "hidden" },
                s.frames,
                s.dock_clicks,
                s.nnotes,
                EV_DROPS.load(Ordering::Relaxed)
            );
            let mut any = false;
            for a in 0..NAPPS {
                let w = s.wins[a];
                if !w.open {
                    continue;
                }
                any = true;
                let (x, y, ww, hh) = win_rect(a);
                crate::kprintln!(
                    "[desk] window {} at {},{} size {}x{}{}",
                    APPS[a].id(),
                    x,
                    y,
                    ww,
                    hh,
                    if w.min { " minimised" } else if focused() == Some(a) { " focused" } else { "" }
                );
            }
            if !any {
                crate::kprintln!("[desk] no windows open");
            }
            let mut order = Line::new();
            for k in (0..s.nz).rev() {
                if order.n > 0 {
                    order.s(b" ");
                }
                order.s(APPS[s.z[k]].id().as_bytes());
            }
            crate::kprintln!(
                "[desk] front to back: {}{}",
                if order.n == 0 { "(none)" } else { core::str::from_utf8(order.get()).unwrap_or("?") },
                if s.launcher { "; launcher open" } else { "" }
            );
        }
        _ => {
            crate::kprintln!("desk: open|close|min|focus <app>, next, launcher, key <name>, tap <x> <y>,");
            crate::kprintln!("      drag <x> <y> <dx> <dy>, mode pointer|touch, notify <text>, notes, perf, select <file>,");
            crate::kprintln!("      move <app> <dx> <dy>, list");
            return;
        }
    }
    if verb != "list" {
        redraw_now();
    }
}
