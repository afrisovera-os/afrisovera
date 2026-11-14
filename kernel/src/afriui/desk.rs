//! The AfriSovera desktop in the Sovereign UI (docs/design/afriui.md, 10 Nov 2026).
//!
//! Both kernels build this file: the PC through pc/canvas.rs, the phone (since 11 Nov, step P2 of
//! afriui-phone-plan.md) through afriui/arm_canvas.rs, which draws straight into ramfb. The 8 Nov
//! desktop (kernel/src/desk.rs) is gone. Behaviour (input ring, windows, focus, `desk` commands,
//! the apps' data) is the 8 Nov desktop's; the look is new: the Spine down the left edge (monument button, app tiles with
//! beads, status and clock), cut corners instead of round ones, each app's woven cloth as its
//! focus mark, the Circle launcher, and the mud-cloth wallpaper.
//!
//! Input never draws inside an interrupt: mouse and keyboard handlers `post` small events to
//! a lock-free ring, and `idle()` (called by the shell before it sleeps on `wfi`) drains the
//! ring, hit-tests, updates the state and redraws in mainline context. Every action is also a
//! shell command (`desk ...`), so the same code is driven by a mouse, a touch, a keyboard or a
//! test script on either kernel.

use crate::framebuffer as fb;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering, AtomicI32};

#[path = "lang.rs"]
mod lang; // the desk's words by id, in each language (LANGUAGES_PLAN.md; afriui.md section 27)
#[path = "paint.rs"]
mod paint; // the theme and every drawing function (afriui.md section 14)
#[path = "cell_abi.rs"]
pub mod cell_abi; // the cell window ABI, one copy for nia.rs and the desk (afriui.md section 25)

// ---------------------------------------------------------------------------------------
// Damage (afriui.md section 15): a change repaints only what it touches.
// ---------------------------------------------------------------------------------------
/// A box on the desktop: (x, y, w, h).
type Rect = (i32, i32, i32, i32);
const NDMG: usize = 8;
/// The damaged boxes waiting for the next frame, and how many. Mainline only.
static mut DMG: [Rect; NDMG] = [(0, 0, 0, 0); NDMG];
static mut NDMG_USED: usize = 0;
/// Something changed that no box describes: the next frame is whole.
static FULL: AtomicBool = AtomicBool::new(true);

fn grow(r: Rect, by: i32) -> Rect {
    (r.0 - by, r.1 - by, r.2 + 2 * by, r.3 + 2 * by)
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.0.min(b.0), a.1.min(b.1));
    let (x1, y1) = ((a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
    (x0, y0, x1 - x0, y1 - y0)
}

/// Mark a box for the next frame (clipped to the screen); a ninth folds all into one box.
fn damage(r: Rect) {
    let (x0, y0) = (r.0.max(0), r.1.max(0));
    let (x1, y1) = ((r.0 + r.2).min(W()), (r.1 + r.3).min(H()));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let mut r = (x0, y0, x1 - x0, y1 - y0);
    // SAFETY: mainline only.
    unsafe {
        let d = &mut *core::ptr::addr_of_mut!(DMG);
        // a box that overlaps (or nearly touches) one already marked joins it: one redraw of the
        // windows there instead of two
        let mut k = 0;
        while k < NDMG_USED {
            let b = d[k];
            let near = grow(b, 8);
            if r.0 < near.0 + near.2 && near.0 < r.0 + r.2 && r.1 < near.1 + near.3 && near.1 < r.1 + r.3 {
                r = union(r, b);
                NDMG_USED -= 1;
                d[k] = d[NDMG_USED];
                k = 0; // the bigger box may now meet another
                continue;
            }
            k += 1;
        }
        if NDMG_USED == NDMG {
            let mut all = r;
            for b in d.iter() {
                all = union(all, *b);
            }
            d[0] = all;
            NDMG_USED = 1;
        } else {
            d[NDMG_USED] = r;
            NDMG_USED += 1;
        }
    }
}

/// A window's box with its relief shadow (what drawing it can touch).
fn win_box(a: usize) -> Rect {
    let (x, y, w, h) = win_rect(a);
    if paint::look::soft() && !st().touch {
        // the soft shadow reaches 11 pixels out and 4 down (afriui.md section 30); `damage` clips to the screen
        paint::look::shadow_box(x, y, w, h)
    } else {
        (x, y, w + 4, h + 4)
    }
}

/// What the desktop shows, as far as the frame is concerned (compared before and after a
/// batch of input, to find the boxes that changed).
#[derive(Clone, Copy, PartialEq)]
struct Snap {
    wins: [(bool, Rect); NWIN],
    z: [usize; NWIN],
    nz: usize,
    launcher: bool,
    sel: usize,
    notes: bool,
    unread: usize,
    nnotes: usize,
    toast: Option<Rect>,
    hover: Option<usize>,
    lang: (bool, usize, usize, bool),
    files_sel: Option<usize>,
    touch: bool,
    wall: bool,
    clock24: bool,
    size: (i32, i32),
}

fn snap() -> Snap {
    let s = st();
    let mut wins = [(false, (0, 0, 0, 0)); NWIN];
    for (a, w) in wins.iter_mut().enumerate() {
        *w = (s.wins[a].open && !s.wins[a].min, win_box(a));
    }
    Snap {
        wins,
        z: s.z,
        nz: s.nz,
        launcher: s.launcher,
        sel: s.sel,
        notes: s.notes_open,
        unread: s.unread,
        nnotes: s.nnotes,
        toast: paint::toast_place(), // set, not yet timed out: its expiry must damage it
        hover: s.hover_close,
        lang: (s.lang_list, s.lang_sel, s.lang_top, s.lang_hover),
        files_sel: s.files_sel,
        touch: s.touch,
        wall: s.wallpaper,
        clock24: s.clock24,
        size: (W(), H()),
    }
}

/// The boxes that changed between two snapshots; false when the change has no box (redraw all).
fn diff(a: &Snap, b: &Snap) -> bool {
    if a.touch != b.touch || a.wall != b.wall || a.size != b.size || a.lang != b.lang {
        return false;
    }
    let spine = dock_box();
    let top = |s: &Snap| if s.nz == 0 { None } else { Some(s.z[s.nz - 1]) };
    let zpos = |s: &Snap, w: usize| s.z[..s.nz].iter().position(|&v| v == w);
    for w in 0..NWIN {
        let (va, ra) = a.wins[w];
        let (vb, rb) = b.wins[w];
        let moved = (va, ra) != (vb, rb);
        let restacked = zpos(a, w) != zpos(b, w);
        let focus = (top(a) == Some(w)) != (top(b) == Some(w));
        if moved || restacked || focus {
            if va {
                damage(ra);
            }
            if vb {
                damage(rb);
            }
            damage(spine); // its beads
        }
    }
    if a.launcher != b.launcher || (b.launcher && a.sel != b.sel) {
        damage(paint::panel_box(launcher_box()));
        damage(grow(circle_info_box(), 4));
        damage(spine);
    }
    if a.notes != b.notes || a.unread != b.unread || a.nnotes != b.nnotes {
        if a.notes || b.notes {
            damage(paint::panel_box(notes_box()));
        }
        damage(spine); // the bell
    }
    if a.toast != b.toast {
        for t in [a.toast, b.toast].into_iter().flatten() {
            damage(t);
        }
    }
    if a.hover != b.hover {
        for w in [a.hover, b.hover].into_iter().flatten() {
            damage(grow(title_buttons(w).0, 1));
        }
    }
    if a.files_sel != b.files_sel && b.wins[App::Files as usize].0 {
        damage(b.wins[App::Files as usize].1);
    }
    if a.clock24 != b.clock24 {
        damage(spine);
    }
    true
}

/// The network state the desktop last showed (settle redraws its places when it changes).
static mut LAST_NET: Option<(bool, [u8; 4])> = None;

/// Draw what is waiting: the whole desktop, or only the damaged boxes.
fn frame(want: bool) {
    let set = App::Settings as usize;
    if st().lang_list && !(st().wins[set].open && !st().wins[set].min) {
        lang_list_close();
        FULL.store(true, Ordering::Relaxed);
    }
    // SAFETY: mainline only.
    let n = unsafe { core::mem::replace(&mut *core::ptr::addr_of_mut!(NDMG_USED), 0) };
    // SAFETY: mainline only.
    unsafe { FRAME_ROWS = 0 };
    if FULL.swap(false, Ordering::Relaxed) || (want && n == 0) {
        paint::draw();
        // SAFETY: as above.
        unsafe { FRAME_ROWS = H().max(0) as u64 };
    } else if n > 0 {
        // SAFETY: mainline only; a copy, so the static is not borrowed while paint runs.
        let boxes: [Rect; NDMG] = unsafe { DMG };
        paint::draw_damage(&boxes[..n]);
        // SAFETY: as above.
        unsafe { FRAME_ROWS = boxes[..n].iter().map(|b| b.3.max(0) as u64).sum() };
    }
}

/// The rows the last frame redrew: the sum of its damage boxes' heights, or the whole height for a full frame,
/// 0 when nothing was drawn (the `desk cell` verbs report it; the same on both kernels).
static mut FRAME_ROWS: u64 = 0;

/// `desk verify`: hash every row as the partial frames left it, redraw everything, count the
/// rows that differ (0 = the damage boxes missed nothing).
static mut ROWS: [u64; crate::canvas::MAX_H] = [0; crate::canvas::MAX_H];
static mut ROWS_NOW: [u64; crate::canvas::MAX_H] = [0; crate::canvas::MAX_H];
fn row_hashes(out: &mut [u64]) {
    let (cw, ch) = (crate::canvas::width(), crate::canvas::height());
    let px = crate::canvas::pixels();
    for (y, h) in out.iter_mut().enumerate().take(ch) {
        let mut v: u64 = 0xcbf2_9ce4_8422_2325;
        for x in 0..cw {
            // SAFETY: inside the canvas.
            v = (v ^ unsafe { *px.add(y * cw + x) } as u64).wrapping_mul(0x100_0000_01b3);
        }
        *h = v;
    }
}

/// The desktop's size in logical pixels: the PC canvas (kernel/src/pc/canvas.rs), the screen's
/// own size over the UI scale (afriui.md section 13), read at every use so `screen ui`
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
// Palette: Earth and Night (afriui.md section 4). Every colour on screen comes from
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
    Cell = 4, // an app's cell window (afriui.md section 24): no Spine or Circle tile yet
}
/// Window state (`wins`, `z`, `HOME`) has a place for every kind; the Spine, the Circle, the keyboard walk and
/// Super+1..4 show only the first NTILES (the architect's split, 13 Nov).
const NWIN: usize = 5;
const NTILES: usize = 4;

/// Bundled apps with a Circle tile after the system apps (afriui.md section 32): (the bundled name, the tile's label,
/// the line the shell takes). Compiled in; nothing on disk adds a tile. The line is this constant, never built from a
/// file name or a title (the architect, 14 Nov). A label is the app's own name, the same in every language.
const CIRCLE_APPS: [(&str, &[u8], &str); 1] = [("andika.wasm", b"Andika", "nia andika.wasm")];
/// The line a tile asked for: 0 none, else CIRCLE_APPS index + 1. Taken only by the shell's prompt (`launch_take`).
static LAUNCH: AtomicUsize = AtomicUsize::new(0);

/// The shell's prompt takes a tile's line (shell.rs `prompt_getc`).
pub fn launch_take() -> Option<&'static str> {
    match LAUNCH.swap(0, Ordering::Relaxed) {
        0 => None,
        k => Some(CIRCLE_APPS[k - 1].2),
    }
}

/// A key came first: the tile's line is dropped.
pub fn launch_drop() {
    LAUNCH.store(0, Ordering::Relaxed);
}

/// The Circle's tiles: the system apps, then the bundled apps of CIRCLE_APPS that passed the boot check.
fn ntile() -> usize {
    NTILES + CIRCLE_APPS.iter().filter(|a| crate::nia::bundled_ok(a.0)).count()
}

/// Tile i (NTILES or more) as an index into CIRCLE_APPS.
fn circle_app(i: usize) -> Option<usize> {
    (0..CIRCLE_APPS.len()).filter(|&k| crate::nia::bundled_ok(CIRCLE_APPS[k].0)).nth(i.checked_sub(NTILES)?)
}

/// An app tile cannot start now: the shell is running something (a command, the pager, an app).
fn circle_busy() -> bool {
    !crate::shell::at_prompt() || crate::nia::app_running()
}
static WAS_BUSY: AtomicBool = AtomicBool::new(false);

/// A tile's id as `desk circle` and the walk print it.
fn tile_id(i: usize) -> &'static str {
    match circle_app(i) {
        Some(k) => CIRCLE_APPS[k].0,
        None => APPS[i].id(),
    }
}

/// An app tile clicked (or Enter on the walk): the shell's prompt takes its line, or a toast says why not.
fn circle_start(k: usize) {
    st().launcher = false;
    if circle_busy() {
        notify_msg(lang::Msg::CircleBusy);
        crate::kprintln!("[desk] circle: {} not started, the shell is busy", CIRCLE_APPS[k].0);
        return;
    }
    LAUNCH.store(k + 1, Ordering::Relaxed);
    crate::kprintln!("[desk] circle: start {}", CIRCLE_APPS[k].0);
}

/// A tile clicked or chosen: a system app opens, an app tile starts.
fn circle_pick(i: usize) {
    match circle_app(i) {
        Some(k) => circle_start(k),
        None => {
            st().launcher = false;
            let was = st().wins[i].open;
            open(i);
            log(if was { b"focus " } else { b"open " }, i);
        }
    }
}
const APPS: [App; NWIN] = [App::Files, App::Terminal, App::Settings, App::About, App::Cell];

impl App {
    /// The window's name as a person reads it, in the desk's language.
    fn name(self) -> &'static [u8] {
        lang::t([lang::Msg::AppFiles, lang::Msg::AppTerminal, lang::Msg::AppSettings, lang::Msg::AppAbout, lang::Msg::AppCell][self as usize])
    }
    fn id(self) -> &'static str {
        ["files", "terminal", "settings", "about", "cell"][self as usize]
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

/// Default pointer-profile windows (afriui.md section 14): each app's centre as an offset
/// from the middle of the free space right of the Spine, and its size: a cascade round the
/// middle at any screen size, every lintel on the screen.
const HOME: [(i32, i32, i32, i32); NWIN] = [(-150, -70, 372, 250), (40, 50, 410, 260), (-70, -30, 380, 300), (110, -20, 330, 300), (0, 0, 410, 360)];

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
    wins: [Win; NWIN],
    z: [usize; NWIN], // stacking order of the open, not-minimised windows: z[0] bottom
    nz: usize,
    launcher: bool,
    notes_open: bool,
    /// The power panel (14 Nov, the owner could not shut down from the desk): 0 closed, 1 the menu, 2 "Shut down
    /// now?", 3 "Restart now?".
    power: u8,
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
    lang_list: bool,  // the language list panel is open (Settings > Display > Language)
    lang_sel: usize,  // the row the keyboard outline is on (an index into lang::LANGS)
    lang_top: usize,  // the first row shown (the list scrolls past LANG_ROWS languages)
    lang_hover: bool, // the pointer is over the Language chip
    px: i32,
    py: i32,
    last_minute: u64,
    frames: u64,
    dock_clicks: u64,
    welcomed: bool,
    sel: usize, // the app chosen in the Circle by the keyboard walk
    motion: bool, // motion on (afriui.md section 7 and 17); Settings and `desk motion` switch it
}

static mut S: State = State {
    shown: false,
    touch: cfg!(target_arch = "aarch64"),
    wins: [WIN0; NWIN],
    z: [0; NWIN],
    nz: 0,
    launcher: false,
    notes_open: false,
    power: 0,
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
    lang_list: false,
    lang_sel: 0,
    lang_top: 0,
    lang_hover: false,
    px: 0,
    py: 0,
    last_minute: u64::MAX,
    frames: 0,
    dock_clicks: 0,
    welcomed: false,
    sel: 0,
    motion: true,
};

/// SAFETY: the desktop state is touched only in mainline context (shell commands and
/// `idle`); interrupt handlers reach it only through the atomic event ring below.
fn st() -> &'static mut State {
    unsafe { &mut *core::ptr::addr_of_mut!(S) }
}

// ---------------------------------------------------------------------------------------
// Motion (afriui.md sections 7 and 17): a window unfolds from its lintel, the Circle grows.
// At most 3 frames, 2 ticks (20 ms) apart, driven by time from `idle()` (mainline); a late
// frame is skipped, never queued. The model is final at once (hit-testing, `desk list`);
// only paint reads the step, to clip a window's rows or size the Circle.
// ---------------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq)]
enum Motion {
    Unfold(usize),
    Grow,
}

/// What a motion did: its kind, the tick each frame was drawn at (0: skipped), and whether a
/// command, a close or a new motion finished it early. For `desk motion`.
#[derive(Clone, Copy)]
struct MotionLog {
    what: Motion,
    ticks: [u64; 3],
    early: bool,
}

/// The running motion: what, the tick it started, the frame now shown (1 or 2; frame 3 is the
/// window or Circle as it is, so the motion ends there). Mainline only.
static mut ANIM: Option<(Motion, u64, u8)> = None;
/// `desk motion hold`: the frame clock stops at this step until any next desk command, any
/// input event or `desk motion off` (0: no hold). Test only.
static mut HOLD: u8 = 0;
/// The last two motions (index 0 the newest) and how many have run since boot.
static mut MLOG: [Option<MotionLog>; 2] = [None, None];
static mut MOTIONS: u64 = 0;
/// The slowest frame the motion clock drew, in microseconds, and how many it drew (`desk perf`).
static mut MOTION_US: (u64, u64) = (0, 0);

fn anim() -> Option<(Motion, u64, u8)> {
    // SAFETY: mainline only.
    unsafe { ANIM }
}

/// The boxes a motion's frames touch (slice 3's damage: everything under them is repainted).
fn motion_damage(m: Motion) {
    match m {
        Motion::Unfold(a) => damage(win_box(a)),
        Motion::Grow => {
            damage(paint::panel_box(launcher_box()));
            damage(grow(circle_info_box(), 4));
        }
    }
}

/// Start a motion (motion on): the one running finishes first (one at a time).
fn start_motion(m: Motion) {
    finish_motion();
    if !st().motion {
        return;
    }
    let t = crate::sched::ticks();
    // SAFETY: mainline only.
    unsafe {
        ANIM = Some((m, t, 1));
        MLOG[1] = MLOG[0];
        MLOG[0] = Some(MotionLog { what: m, ticks: [t, 0, 0], early: false });
        MOTIONS += 1;
    }
    motion_damage(m);
    DIRTY.store(true, Ordering::Relaxed);
}

/// Show the final frame now: a new motion, a close, a minimise, a mode switch or a shell
/// command comes first.
fn finish_motion() {
    // SAFETY: mainline only.
    unsafe {
        HOLD = 0;
        if let Some((m, _, _)) = ANIM.take() {
            if let Some(l) = MLOG[0].as_mut() {
                l.early = true;
            }
            motion_damage(m);
            DIRTY.store(true, Ordering::Relaxed);
        }
    }
}

/// The frame clock, from `settle`: the step time says, the last one ends the motion. True
/// when a frame is due.
fn motion_tick() -> bool {
    let Some((m, t0, step)) = anim() else { return false };
    let s = st();
    let gone = match m {
        Motion::Unfold(a) => !s.wins[a].open || s.wins[a].min,
        Motion::Grow => !s.launcher,
    };
    if gone {
        // SAFETY: mainline only.
        unsafe { ANIM = None };
        return false;
    }
    // SAFETY: mainline only.
    let hold = unsafe { HOLD };
    let t = crate::sched::ticks();
    let mut due = (1 + t.saturating_sub(t0) / 2).min(3) as u8; // frame k is due 2(k-1) ticks in
    if hold != 0 {
        due = due.min(hold);
    }
    if due <= step {
        return false;
    }
    // SAFETY: mainline only.
    unsafe {
        if let Some(l) = MLOG[0].as_mut() {
            l.ticks[due as usize - 1] = t;
        }
        ANIM = if due == 3 { None } else { Some((m, t0, due)) };
    }
    motion_damage(m);
    true
}

/// For paint: the rows of window a shown now (its lintel and a third of its body per frame),
/// or None when it is not moving.
pub(super) fn unfold_rows(a: usize) -> Option<i32> {
    match anim() {
        Some((Motion::Unfold(w), _, step)) if w == a => {
            let (_, _, _, h) = win_rect(a);
            let th = title_h();
            Some(th + (h - th) * step as i32 / 3)
        }
        _ => None,
    }
}

/// For paint: the Circle's frame (1 or 2 of 3) while it grows.
pub(super) fn grow_step() -> Option<i32> {
    match anim() {
        Some((Motion::Grow, _, step)) => Some(step as i32),
        _ => None,
    }
}

/// `desk motion` reports whole motions: a running one (not held) first runs to its end by its
/// own clock, as `idle()` would draw it (at most 4 ticks, waited for up to 10). The shell does
/// not call `idle()` while it reads a typed command, so without this a report could come
/// between two frames.
fn run_out_motion() {
    let Some((_, t0, _)) = anim() else { return };
    // bounded by the cycle counter too (0.2 s), in case the timer tick does not advance here
    let (c0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while anim().is_some() && crate::sched::ticks() < t0 + 10 && crate::timer::cycles().wrapping_sub(c0) < hz / 5 {
        settle(snap());
        crate::canvas::flush();
        core::hint::spin_loop();
    }
}

/// Open the Circle (a Spine tap, Super, `desk launcher`): it grows from its button.
fn open_circle() {
    st().launcher = true;
    start_motion(Motion::Grow);
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
/// The keyboard walk (afriui.md section 14): arrows and Enter while the Circle is open,
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
/// Lines kept (afriui.md section 28: the owner's `hwscan` on the HP ran 72 devices past a 48-line Terminal).
const TL: usize = 2048;
const TC: usize = 96;
/// How many lines up from the bottom the Terminal shows (0: the live bottom). Raised by `term_feed` while scrolled
/// back so new output does not move the view; set in mainline from the counts below.
static TVIEW: AtomicUsize = AtomicUsize::new(0);
/// Asked for by the keyboard and mouse paths (interrupts), applied in mainline by `term_apply`: half pages
/// (Shift+PgUp +1, Shift+PgDn -1), wheel lines (positive: back), and "back to the bottom" (any typed key).
static TPAGES: AtomicI32 = AtomicI32::new(0);
static TWHEEL: AtomicI32 = AtomicI32::new(0);
static TBOTTOM: AtomicBool = AtomicBool::new(false);
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
                    // scrolled back: the view stays on the same text as the new line pushes it up
                    let v = TVIEW.load(Ordering::Relaxed);
                    if v > 0 {
                        TVIEW.store((v + 1).min(TLINES.saturating_sub(1)), Ordering::Relaxed);
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

/// Shift+PgUp (`up`) or Shift+PgDn with the desk showing: half the Terminal's rows. Safe from an interrupt.
pub fn term_page(up: bool) {
    TPAGES.fetch_add(if up { 1 } else { -1 }, Ordering::Relaxed);
    TERM_DIRTY.store(true, Ordering::Relaxed);
}

/// Wheel or touchpad notches with the desk showing (negative: rolled away, back through the text), three lines a
/// notch, for the Terminal when the pointer is over it. Safe from an interrupt.
pub fn term_wheel(notches: i32) {
    TWHEEL.fetch_add(-notches.clamp(-64, 64) * 3, Ordering::Relaxed);
    TERM_DIRTY.store(true, Ordering::Relaxed);
}

/// A key was typed: the Terminal goes back to its live bottom. Safe from an interrupt.
pub fn term_bottom() {
    if TVIEW.load(Ordering::Relaxed) != 0 || TPAGES.load(Ordering::Relaxed) != 0 || TWHEEL.load(Ordering::Relaxed) != 0 {
        TBOTTOM.store(true, Ordering::Relaxed);
        TERM_DIRTY.store(true, Ordering::Relaxed);
    }
}

/// The Terminal window's text rows (the pager's screenful when the desk shows).
pub fn term_rows() -> usize {
    let (_, _, _, h) = win_rect(App::Terminal as usize);
    (((h - title_h() - 2) - 12) / 13).max(1) as usize
}

/// The view offset now (0: the bottom).
pub(super) fn term_view() -> usize {
    TVIEW.load(Ordering::Relaxed)
}

/// Apply what the keyboard and mouse asked for (mainline). True if the view moved.
fn term_apply() -> bool {
    let s = st();
    let t = App::Terminal as usize;
    let before = TVIEW.load(Ordering::Relaxed);
    // SAFETY: a scalar read in mainline.
    let kept = unsafe { TLINES };
    let mut v = before as i64;
    if TBOTTOM.swap(false, Ordering::Relaxed) {
        v = 0;
        TPAGES.store(0, Ordering::Relaxed);
        TWHEEL.store(0, Ordering::Relaxed);
    }
    let pages = TPAGES.swap(0, Ordering::Relaxed);
    v += pages as i64 * (term_rows() / 2).max(1) as i64;
    let wheel = TWHEEL.swap(0, Ordering::Relaxed);
    if wheel != 0 && s.lang_list {
        lang_list_wheel(wheel);
        return false;
    }
    // the wheel scrolls the Terminal under the pointer, or the focused Terminal
    if wheel != 0 && s.wins[t].open && !s.wins[t].min && (inside(s.px, s.py, win_rect(t)) || focused() == Some(t)) {
        v += wheel as i64;
    }
    let v = v.clamp(0, kept.saturating_sub(1) as i64) as usize;
    TVIEW.store(v, Ordering::Relaxed);
    v != before
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
    b: [u8; LINE_MAX],
    n: usize,
}
/// A line of desk text in bytes: room for a sentence in a language longer than English, in UTF-8.
const LINE_MAX: usize = 128;
impl Line {
    fn new() -> Line {
        Line { b: [0; LINE_MAX], n: 0 }
    }
    /// The sentence `m` in the desk's language, its gaps filled with `args` (lang.rs).
    fn f(&mut self, m: lang::Msg, args: &[lang::Arg]) -> &mut Line {
        lang::fill_with(m, args, &mut |b: &[u8]| {
            self.s(b);
        });
        self
    }
    fn s(&mut self, s: &[u8]) -> &mut Line {
        for &c in s {
            if self.n < LINE_MAX {
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
/// A phone held upright (touch profile, taller than wide): the Spine lies down as a bar along
/// the bottom (afriui-phone-plan.md step P1).
fn lying() -> bool {
    st().touch && H() > W()
}

/// The Spine's width (its woven edge included): windows live right of it. 0 when it lies down.
fn spine_w() -> i32 {
    if lying() {
        0
    } else if st().touch {
        SPINE_T
    } else {
        SPINE_P
    }
}

/// The bar's height when the Spine lies down along the bottom, else 0.
fn bar_h() -> i32 {
    if lying() {
        SPINE_T
    } else {
        0
    }
}

/// The Spine (x, y, w, h): the whole left edge, or the bottom bar when it lies down.
fn dock_box() -> (i32, i32, i32, i32) {
    if lying() {
        return (0, H() - bar_h(), W(), bar_h());
    }
    (0, 0, spine_w(), H())
}

/// Tile i of the Spine (0 = the Circle button, 1.. = the apps): (x, y, size).
fn dock_tile(i: usize) -> (i32, i32, i32) {
    let s = tile();
    if lying() {
        // in a row along the bar, a gap after the Circle button
        let y = H() - bar_h() + WEAVE + (bar_h() - WEAVE - s) / 2;
        return (8 + i as i32 * (s + 10) + if i > 0 { 12 } else { 0 }, y, s);
    }
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
        return (sw, TOP, W() - sw, H() - TOP - bar_h());
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
    let (sw, fh) = (spine_w(), H() - bar_h()); // the free space right of the Spine or above the bar
    let size = if st().touch { 340 } else { 300 }.min(fh - 90).min(W() - sw - 16);
    (sw + (W() - sw - size) / 2, (fh - size - 56) / 2, size, size)
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

/// cos and sin of five directions 72 degrees apart, from the top, clockwise, x1000.
const FIVE: [(i32, i32); 5] = [(0, -1000), (951, -309), (588, 809), (-588, 809), (-951, -309)];

/// App i's tile in the Circle: standing in a ring round the monument (x, y, size).
fn launcher_tile(i: usize) -> (i32, i32, i32) {
    let (lx, ly, lw, lh) = launcher_box();
    let s = if st().touch { 52 } else { 44 };
    let (cx, cy) = (lx + lw / 2, ly + lh / 2 - 6);
    let r = lw * 33 / 100;
    let n = ntile();
    // evenly round the ring from the top: the compass points for four, a pentagon for five (section 32)
    let (c, si) = if n == 5 { FIVE[i % 5] } else { DIRS[(i * 16 / n) % 16] };
    (cx + r * c / 1000 - s / 2, cy + r * si / 1000 - s / 2, s)
}

/// The notification list opens beside the Spine, at its foot (by the bell).
fn notes_box() -> (i32, i32, i32, i32) {
    let w = 280.min(W() - spine_w() - 12);
    let h = 30 + 18 * NNOTE as i32;
    (spine_w() + 6, H() - bar_h() - h - 8, w, h)
}

/// The power button on the upright Spine, above network and power state.
fn power_btn_box() -> (i32, i32, i32, i32) {
    (0, H() - 134, spine_w() - WEAVE, 26)
}

/// The power panel beside the Spine's foot: the menu, or its confirm.
pub(super) fn power_box() -> (i32, i32, i32, i32) {
    let w = 240.min(W() - spine_w() - 12);
    let h = 96;
    (spine_w() + 6, (H() - bar_h() - h - 8).min(H() - 134 - 10), w, h)
}

/// The panel's two buttons: (first, second). The menu: Shut down above Restart. A confirm: the action left,
/// Cancel right.
pub(super) fn power_buttons() -> ((i32, i32, i32, i32), (i32, i32, i32, i32)) {
    let (x, y, w, _) = power_box();
    if st().power == 1 {
        ((x + 14, y + 32, w - 28, 24), (x + 14, y + 62, w - 28, 24))
    } else {
        let bw = (w - 28 - 10) / 2;
        ((x + 14, y + 58, bw, 26), (x + 14 + bw + 10, y + 58, bw, 26))
    }
}

/// Shut down or restart (the panel's confirm, or `desk power yes`): the shell's own `poweroff` and `reboot`, so the
/// same kernel paths as typing them (ACPI and UEFI on the PC, PSCI on the phone).
fn power_go(restart: bool) -> ! {
    crate::kprintln!("[desk] power: {}", if restart { "restarting" } else { "shutting down" });
    crate::shell::exec(if restart { "reboot" } else { "poweroff" });
    // neither returns; if one ever does, stop here rather than carry on half shut down
    loop {
        crate::cpu::wfi();
    }
}

/// The bell at the Spine's foot, above the clock.
fn bell_box() -> (i32, i32, i32, i32) {
    if lying() {
        return (W() - 104, H() - bar_h() + WEAVE, 40, bar_h() - WEAVE); // left of the clock
    }
    (0, H() - 52, spine_w() - WEAVE, 22)
}

/// The clock's place: the Spine's foot, or the bar's right end when it lies down.
fn clock_box() -> (i32, i32, i32, i32) {
    if lying() {
        return (W() - 60, H() - bar_h(), 60, bar_h());
    }
    (0, H() - 34, spine_w(), 34)
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
    let appears = !s.wins[a].open || s.wins[a].min; // opened or restored: it unfolds
    if !s.wins[a].open {
        let (x, y, w, h) = home(a);
        s.wins[a] = Win { open: true, min: false, x, y, w, h };
        if a == App::Files as usize {
            s.files_sel = None;
        }
        if a == App::Cell as usize {
            cell_clear();
        }
    }
    s.wins[a].min = false;
    raise(a);
    if appears {
        start_motion(Motion::Unfold(a));
    }
}

fn close(a: usize) {
    finish_motion();
    st().wins[a].open = false;
    st().wins[a].min = false;
    z_remove(a);
}

fn minimise(a: usize) {
    finish_motion();
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

// ---------------------------------------------------------------------------------------
// The cell window (afriui.md section 24, stage 1): a grid of 2-byte cells (a character, then an
// attribute: foreground role in the low 4 bits, background role in the high 4), drawn by paint.rs
// in 6 x 13 cells. The grid keeps a fixed stride of CELL_COLS, so a resize keeps every cell's place
// from the top left. Mainline only, as the rest of the desk state.
// ---------------------------------------------------------------------------------------
pub(super) const CELL_COLS: usize = 120;
pub(super) const CELL_ROWS: usize = 50;
pub(super) const CELL_W: i32 = 6;
pub(super) const CELL_H: i32 = 13;
static mut CELLS: [u8; CELL_COLS * CELL_ROWS * 2] = [0; CELL_COLS * CELL_ROWS * 2];
/// The cursor: column, row, shape (0 hidden, 1 bar, 2 block, 3 underline).
static mut CELL_CUR: (usize, usize, u8) = (0, 0, 0);

pub(super) fn cell_cells() -> &'static [u8; CELL_COLS * CELL_ROWS * 2] {
    // SAFETY: mainline only; read while drawing, written only by the cell calls below.
    unsafe { &*core::ptr::addr_of!(CELLS) }
}

fn cell_cells_mut() -> &'static mut [u8; CELL_COLS * CELL_ROWS * 2] {
    // SAFETY: mainline only.
    unsafe { &mut *core::ptr::addr_of_mut!(CELLS) }
}

pub(super) fn cell_cursor_at() -> (usize, usize, u8) {
    // SAFETY: mainline only.
    unsafe { CELL_CUR }
}

/// The cell window's body (x, y, w, h), as window_whole draws it.
pub(super) fn cell_body() -> Rect {
    let (x, y, w, h) = win_rect(App::Cell as usize);
    let th = title_h();
    (x + 1, y + th + 1, w - 2, h - th - 2)
}

/// The grid that fits the body: (columns, rows), 0 for a body too small for one cell (the subtractions
/// saturate at 0, the architect's condition), at most CELL_COLS x CELL_ROWS.
pub(super) fn cell_grid() -> (usize, usize) {
    let (_, _, bw, bh) = cell_body();
    let cols = (bw.max(0).saturating_sub(16) / CELL_W) as usize;
    let rows = (bh.max(0).saturating_sub(12) / CELL_H) as usize;
    (cols.min(CELL_COLS), rows.min(CELL_ROWS))
}

/// The box of cell (col, row) on the desktop: 8 px in from the body's left, bands of 13 rows from 5 px down.
pub(super) fn cell_box(col: usize, row: usize) -> Rect {
    let (bx, by, _, _) = cell_body();
    (bx + 8 + col as i32 * CELL_W, by + 5 + row as i32 * CELL_H, CELL_W, CELL_H)
}

fn cell_shown() -> bool {
    let w = st().wins[App::Cell as usize];
    st().shown && w.open && !w.min
}

/// Damage one row's band across the body (a put).
fn cell_damage_row(row: usize) {
    if cell_shown() {
        let (bx, _, bw, _) = cell_body();
        let (_, y, _, h) = cell_box(0, row);
        damage((bx, y, bw, h));
    }
}

/// Damage one cell (the cursor).
fn cell_damage_cell(col: usize, row: usize) {
    if cell_shown() {
        damage(cell_box(col, row));
    }
}

/// Every cell a space in role 0 on role 0, the cursor hidden at 0,0.
fn cell_clear() {
    let g = cell_cells_mut();
    for k in 0..CELL_COLS * CELL_ROWS {
        g[2 * k] = b' ';
        g[2 * k + 1] = 0;
    }
    // SAFETY: mainline only.
    unsafe { CELL_CUR = (0, 0, 0) };
}

/// Write a run of cells (2 bytes each) along one row from (col, row); it does not wrap. Returns the cells
/// written, 0 when (col, row) is outside the grid. `cols - col` bounds the run, so a huge col or length
/// cannot overflow (the architect's condition).
pub fn cell_put(col: usize, row: usize, run: &[u8]) -> usize {
    let (cols, rows) = cell_grid();
    if row >= rows || col >= cols {
        return 0;
    }
    let n = (run.len() / 2).min(cols - col);
    let at = (row * CELL_COLS + col) * 2;
    cell_cells_mut()[at..at + 2 * n].copy_from_slice(&run[..2 * n]);
    if n > 0 {
        cell_damage_row(row);
    }
    n
}

/// Move the cursor (outside the grid it is not drawn); damages the old cell and the new one.
pub fn cell_cursor(col: usize, row: usize, shape: u8) {
    let (oc, or, os) = cell_cursor_at();
    if os != 0 && oc < CELL_COLS && or < CELL_ROWS {
        cell_damage_cell(oc, or);
    }
    // SAFETY: mainline only.
    unsafe { CELL_CUR = (col, row, shape.min(3)) };
    if shape != 0 && col < CELL_COLS && row < CELL_ROWS {
        cell_damage_cell(col, row);
    }
}

/// A hash of the cells inside the grid (FNV-1a over the bytes, row by row).
fn cell_hash() -> u64 {
    let (cols, rows) = cell_grid();
    let g = cell_cells();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for r in 0..rows {
        for &b in &g[r * CELL_COLS * 2..(r * CELL_COLS + cols) * 2] {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Draw what the cell calls damaged, as `idle()` would, and say how many rows the frame redrew.
fn cell_settle() -> u64 {
    // SAFETY: mainline only.
    unsafe { FRAME_ROWS = 0 };
    settle(snap());
    crate::canvas::flush();
    // SAFETY: as above.
    unsafe { FRAME_ROWS }
}

/// Whether two cells drew the same pixels (the box check of the `desk-cell` suite).
fn cell_same(a: (usize, usize), b: (usize, usize)) -> bool {
    let cw = crate::canvas::width();
    let px = crate::canvas::pixels();
    let ((ax, ay, w, h), (bx, by, _, _)) = (cell_box(a.0, a.1), cell_box(b.0, b.1));
    for j in 0..h {
        for i in 0..w {
            let p = (ay + j) as usize * cw + (ax + i) as usize;
            let q = (by + j) as usize * cw + (bx + i) as usize;
            // SAFETY: both cells are inside the grid, so inside the window and the canvas (the caller checks).
            if unsafe { *px.add(p) != *px.add(q) } {
                return false;
            }
        }
    }
    true
}

fn hex1(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

fn hex2(s: &[u8]) -> Option<u8> {
    if s.len() != 2 || !s.iter().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex1(s[0]) << 4 | hex1(s[1]))
}

/// Text to cells in one attribute; `\xNN` is one byte (so a test can send the bytes the font lacks).
fn cell_run(t: &[u8], attr: u8, run: &mut [u8; 2 * CELL_COLS]) -> usize {
    let (mut n, mut k) = (0, 0);
    while k < t.len() && n < CELL_COLS {
        let (b, step) = match (t[k], t.get(k + 1)) {
            (b'\\', Some(&b'x')) if k + 4 <= t.len() && hex2(&t[k + 2..k + 4]).is_some() => (hex2(&t[k + 2..k + 4]).unwrap_or(0), 4),
            (b, _) => (b, 1),
        };
        run[2 * n] = b;
        run[2 * n + 1] = attr;
        n += 1;
        k += step;
    }
    n
}

/// The `desk cell demo` page: an editor-like view using all 16 roles, the box and a block cursor.
fn cell_demo() {
    cell_clear();
    // (text, foreground role of each cell as a hex digit; past the digits, role 0)
    let lines: [(&[u8], &[u8]); 9] = [
        (b" 1 fn main() {", b"77022000006606"),
        (b" 2     let name = \"Andika\";", b"770000022200000060333333336"),
        (b" 3     // a comment", b"7700000444444444444"),
        (b" 4     let n = 42 + 7;", b"8800000222000605506056"),
        (b" 5     print(name, n);", b"7700000000006000060066"),
        (b" 6 }", b"7706"),
        (b" 7 boxes: \x80 \x01 \xC3 end", b"7701111110f0f0f0111"),
        (b" 8 error ok accent dim", b"770ddddd0ee0ffffff0111"),
        (b" 9 status-dim", b"770bbbbbbbbbb"),
    ];
    let mut run = [0u8; 2 * CELL_COLS];
    for (r, (text, roles)) in lines.iter().enumerate() {
        let n = text.len().min(CELL_COLS);
        for (k, &b) in text.iter().take(n).enumerate() {
            run[2 * k] = b;
            run[2 * k + 1] = roles.get(k).map(|&c| hex1(c)).unwrap_or(0);
        }
        cell_put(0, r, &run[..2 * n]);
    }
    // row 3 on the cursor-line role (12), the selection role (9) under "name" on row 1
    let g = cell_cells_mut();
    for c in 0..CELL_COLS {
        let at = (3 * CELL_COLS + c) * 2 + 1;
        g[at] = (g[at] & 0x0f) | 0xc0;
    }
    for c in 10..14 {
        let at = (CELL_COLS + c) * 2 + 1;
        g[at] = (g[at] & 0x0f) | 0x90;
    }
    // a status line at the foot: role 10 on role 9
    let (_, rows) = cell_grid();
    if rows > lines.len() + 1 {
        let n = cell_run(b" Andika  demo.nia  UTF-8  Ln 4, Col 9 ", 0x9a, &mut run);
        cell_put(0, rows - 1, &run[..n * 2]);
    }
    cell_cursor(9, 3, 2);
}

/// `desk cell ...` (afriui.md section 24): the same calls an app will make, from the shell. Returns true when
/// the verb wants the usual full redraw; the verbs that only change cells draw their own partial frame.
fn cell_command(sub: &str, rest: &str) -> bool {
    let mut it = rest.split_whitespace();
    let mut num = || it.next().and_then(|v| v.parse::<usize>().ok());
    let c = App::Cell as usize;
    match sub {
        "open" => {
            let was = st().wins[c].open;
            open(c);
            log(if was { b"focus " } else { b"open " }, c);
            let (cols, rows) = cell_grid();
            crate::kprintln!("[desk] cell: {}x{} grid", cols, rows);
            true
        }
        "demo" => {
            if !st().wins[c].open {
                open(c);
            }
            cell_demo();
            let (cols, rows) = cell_grid();
            crate::kprintln!("[desk] cell demo: {}x{} grid, hash {:016x}", cols, rows, cell_hash());
            true
        }
        "size" => {
            let (cols, rows) = cell_grid();
            let (cc, cr, cs) = cell_cursor_at();
            crate::kprintln!("[desk] cell: {}x{} grid, cursor {},{} shape {}, hash {:016x}", cols, rows, cc, cr, cs, cell_hash());
            false
        }
        "titletest" => {
            // test only: an unsigned app's title with Close anyway offered, and no app (see TITLE_TEST)
            let name = it.next().unwrap_or("off");
            if name == "off" {
                TITLE_TEST.store(false, Ordering::Relaxed);
                crate::kprintln!("[desk] cell titletest off");
            } else if CELL_OWNED.load(Ordering::Relaxed) {
                // never dress a real app's window: its title shows its own name and mark (the architect, 13 Nov)
                crate::kprintln!("[desk] cell titletest: refused, an app owns the window");
            } else {
                let n = name.len().min(24);
                // SAFETY: mainline only.
                unsafe {
                    (&mut *core::ptr::addr_of_mut!(TEST_NAME))[..n].copy_from_slice(&name.as_bytes()[..n]);
                    TEST_LEN = n;
                }
                TITLE_TEST.store(true, Ordering::Relaxed);
                let win = &mut st().wins[c];
                win.w = win.w.max(cell_min_w());
                crate::kprintln!("[desk] cell titletest: {} (unsigned, Close anyway offered)", &name[..n]);
            }
            true
        }
        "titlehex" => {
            // test only (SHAPING_PLAN C1, the architect's title case): titletest with a name given as UTF-8 in hex,
            // so the suite can give the title Arabic letters the shell's line editor cannot type
            let mut b = [0u8; 24];
            let n = unhex(it.next().unwrap_or(""), &mut b);
            // SAFETY: mainline only.
            unsafe {
                (&mut *core::ptr::addr_of_mut!(TEST_NAME))[..n].copy_from_slice(&b[..n]);
                TEST_LEN = n;
            }
            TITLE_TEST.store(n > 0, Ordering::Relaxed);
            let win = &mut st().wins[c];
            win.w = win.w.max(cell_min_w());
            crate::kprintln!("[desk] cell titlehex: {} bytes (unsigned, Close anyway offered)", n);
            true
        }
        "titlecheck" => {
            // where the last drawn cell title put its parts (paint records them), and whether any overlap
            let (wx, ww, mx, mw, ax, aw, bx, cut) = paint::lintel_parts();
            let whole = mw > 0 && mx >= wx && mx + mw <= ax - 2;
            let clear = ax + aw <= bx && ax >= mx + mw;
            crate::kprintln!(
                "[desk] cell title: window {} wide (at least {}), mark {}..{} {}, Close anyway {}..{}, buttons from {}; {}; name {}",
                ww, cell_min_w(), mx - wx, mx + mw - wx, if whole { "whole" } else { "NOT WHOLE" }, ax - wx, ax + aw - wx, bx - wx,
                if clear { "nothing over Close anyway" } else { "OVERLAP" }, if cut { "cut" } else { "whole" }
            );
            false
        }
        "resize" => {
            // the window's own size (pointer mode); the body and the grid follow, the cells keep their place
            if let (Some(w), Some(h)) = (num(), num()) {
                let win = &mut st().wins[c];
                win.w = (w.min(4000) as i32).clamp(40, (W() - SPINE_P - 8).max(40)).max(cell_min_w());
                win.h = (h.min(4000) as i32).clamp(30, (H() - TOP - 8).max(30));
            }
            let (cols, rows) = cell_grid();
            crate::kprintln!("[desk] cell: {}x{} grid after resize", cols, rows);
            true
        }
        "put" => {
            // put <col> <row> <attr hex> <text>: the text is the rest of the line, spaces kept
            let (col, row) = (num(), num());
            let attr = it.next().and_then(|a| match a.len() {
                1 => a.as_bytes()[0].is_ascii_hexdigit().then(|| hex1(a.as_bytes()[0])),
                _ => hex2(a.as_bytes()),
            });
            let (Some(col), Some(row), Some(attr)) = (col, row, attr) else {
                crate::kprintln!("desk cell put <col> <row> <attr hex> <text>");
                return false;
            };
            let mut r = rest.trim_start();
            for _ in 0..3 {
                r = r.find(' ').map(|i| r[i..].trim_start()).unwrap_or("");
            }
            let mut run = [0u8; 2 * CELL_COLS];
            let n = cell_run(r.as_bytes(), attr, &mut run);
            let wrote = cell_put(col, row, &run[..2 * n]);
            let copied = cell_settle();
            crate::kprintln!("[desk] cell put: {} cell(s) at {},{}; {} row(s) redrawn", wrote, col, row, copied);
            false
        }
        "cursor" => {
            let (Some(col), Some(row), Some(shape)) = (num(), num(), num()) else {
                crate::kprintln!("desk cell cursor <col> <row> <shape 0-3>");
                return false;
            };
            cell_cursor(col, row, shape.min(3) as u8);
            let copied = cell_settle();
            crate::kprintln!("[desk] cell cursor: {},{} shape {}; {} row(s) redrawn", col, row, shape.min(3), copied);
            false
        }
        "roles" => {
            // the 16 colours and the title marks' grounds, for the contrast grade (section 32)
            paint::cell_roles_report();
            false
        }
        "plan" => {
            // plan <step>;<step>...: applied one per desk pass while an app owns the window (section 25)
            plan_set(rest.trim());
            false
        }
        "same" => {
            let v = [num(), num(), num(), num()];
            let (cols, rows) = cell_grid();
            match v {
                [Some(a), Some(b), Some(x), Some(y)] if a < cols && b < rows && x < cols && y < rows && cell_shown() => {
                    let same = cell_same((a, b), (x, y));
                    crate::kprintln!("[desk] cell same: {},{} and {},{} {}", a, b, x, y, if same { "draw the same" } else { "differ" });
                }
                _ => crate::kprintln!("desk cell same <col> <row> <col> <row> (both in the grid, the window shown)"),
            }
            false
        }
        _ => {
            crate::kprintln!("desk cell open|demo|size|resize <w> <h>|put <col> <row> <attr hex> <text>|cursor <col> <row> <shape>|same <c> <r> <c> <r>|plan <step>;...");
            false
        }
    }
}

// ---------------------------------------------------------------------------------------
// Cell windows for apps (afriui.md section 25, stage 2): the running Nia app owns the cell window. nia.rs calls
// the `app_*` functions and `cell_*` hooks below; they touch only desk state, never the interpreter's (the
// architect's condition): anything the app must hear goes through the cell queue.
// ---------------------------------------------------------------------------------------
use cell_abi as abi;

/// The running app owns the cell window (from `app_cell_open` until `cell_app_ended`).
static CELL_OWNED: AtomicBool = AtomicBool::new(false);
/// "Close anyway" was chosen: the interpreter traps at its next backward branch or wait.
static CELL_FORCED: AtomicBool = AtomicBool::new(false);
/// The app's events: head and tail count up for ever (QUEUE is a power of two); full at QUEUE apart.
static CQ: [AtomicU32; abi::QUEUE] = [const { AtomicU32::new(0) }; abi::QUEUE];
static CQ_HEAD: AtomicUsize = AtomicUsize::new(0);
static CQ_TAIL: AtomicUsize = AtomicUsize::new(0);
static CQ_DROPS: AtomicU32 = AtomicU32::new(0);
/// The owner's file name and whether it is a bundled or verified app: the title bar shows both, and the app
/// cannot set or cover them (security-model.md section 4).
static mut CELL_NAME: [u8; 24] = [0; 24];
static mut CELL_NAME_LEN: usize = 0;
static mut CELL_SIGNED: bool = false;
/// The first unanswered close request (ms on the always-running counter; 0 none). Only the end of the app's run
/// answers it: taking it from the queue does not, so an app that ignores requests still gets "Close anyway" 3 s
/// after the first one (section 25: a way out the app cannot block).
static mut CELL_REQ_AT: u64 = 0;
/// The grid size last told to the app, and whether "Close anyway" was last drawn.
static mut CELL_GRID: u32 = 0;
static mut CELL_OFFERED: bool = false;
/// When `app_tick` last ran the desk (counter cycles).
static mut CELL_TICK_AT: u64 = 0;
/// When "Close anyway" was chosen (ms), for the line that says how soon the app stopped.
static mut CELL_FORCED_AT: u64 = 0;

fn ms_now() -> u64 {
    (crate::timer::cycles() as u128 * 1000 / crate::timer::hz().max(1) as u128) as u64
}

fn cq_post(kind: u32, payload: u32) {
    let t = CQ_TAIL.load(Ordering::Relaxed);
    if t.wrapping_sub(CQ_HEAD.load(Ordering::Acquire)) >= abi::QUEUE {
        CQ_DROPS.fetch_add(1, Ordering::Relaxed); // the newest is dropped, never the oldest overwritten
        return;
    }
    CQ[t % abi::QUEUE].store(abi::event(kind, payload) as u32, Ordering::Relaxed);
    CQ_TAIL.store(t.wrapping_add(1), Ordering::Release);
}

fn cq_pop() -> Option<i32> {
    let h = CQ_HEAD.load(Ordering::Relaxed);
    if h == CQ_TAIL.load(Ordering::Acquire) {
        return None;
    }
    let v = CQ[h % abi::QUEUE].load(Ordering::Relaxed);
    CQ_HEAD.store(h.wrapping_add(1), Ordering::Release);
    Some(v as i32)
}

fn cq_clear() {
    CQ_HEAD.store(CQ_TAIL.load(Ordering::Acquire), Ordering::Release);
}

/// The app owns the window, it is open and it was not closed anyway.
fn cell_live() -> bool {
    CELL_OWNED.load(Ordering::Relaxed) && !CELL_FORCED.load(Ordering::Relaxed) && st().wins[App::Cell as usize].open
}

/// The running app owns a cell window (nia.rs: serial bytes go to `cell_serial_byte`, `getkey` returns -2).
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn cell_owned() -> bool {
    CELL_OWNED.load(Ordering::Relaxed)
}

/// "Close anyway" was chosen (nia.rs reads it on every backward branch and in its waits, and traps).
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn cell_forced() -> bool {
    CELL_FORCED.load(Ordering::Relaxed)
}

/// The title the desk draws for the cell window: the owner's file name and whether it is signed.
pub(super) fn cell_owner() -> Option<(&'static [u8], bool)> {
    if TITLE_TEST.load(Ordering::Relaxed) {
        // SAFETY: mainline only; set by `desk cell titletest`.
        return unsafe { Some((&(&*core::ptr::addr_of!(TEST_NAME))[..TEST_LEN], false)) };
    }
    if !CELL_OWNED.load(Ordering::Relaxed) {
        return None;
    }
    // SAFETY: mainline only; set once by `app_cell_open`.
    unsafe { Some((&(&*core::ptr::addr_of!(CELL_NAME))[..CELL_NAME_LEN], CELL_SIGNED)) }
}

/// A close request has gone unanswered for FORCE_AFTER_MS: "Close anyway" is offered.
fn req_old() -> bool {
    // SAFETY: mainline only.
    let at = unsafe { CELL_REQ_AT };
    at != 0 && ms_now().saturating_sub(at) >= abi::FORCE_AFTER_MS
}

pub(super) fn cell_offer() -> bool {
    TITLE_TEST.load(Ordering::Relaxed) || (cell_live() && req_old())
}

/// Test only (`desk cell titletest NAME|off`, the desk-font suite): the cell window's title shows NAME as an unsigned
/// app's with Close anyway offered, with no app running, so the narrowest title can be checked (afriui.md section 26,
/// the architect's follow-up 1). Clicking its Close anyway is refused as with no app.
static TITLE_TEST: AtomicBool = AtomicBool::new(false);
static mut TEST_NAME: [u8; 24] = [0; 24];
static mut TEST_LEN: usize = 0;

/// The narrowest a cell window may be while it wears an app's name: the lintel's icon, "..", the unsigned mark, Close
/// anyway and the buttons all fit whole. 40 for the test window with no app.
fn cell_min_w() -> i32 {
    let w = if cell_owner().is_some() { paint::lintel_min_w() } else { 40 };
    w.min((W() - SPINE_P - 8).max(40))
}

/// The "Close anyway" button in the cell window's title bar, left of the minimise button.
pub(super) fn anyway_box(a: usize) -> Rect {
    let (cl, mn) = title_buttons(a);
    let ts = if st().touch { 2 } else { 1 };
    let w = paint::anyway_w(lang::current(), ts);
    let x = if st().touch { cl.0 } else { mn.0 };
    (x - 8 * ts - w, cl.1, w, cl.3)
}

/// Close a window, or for an app's cell window send the app a close request (it decides). True when closed.
fn close_or_ask(a: usize) -> bool {
    if a == App::Cell as usize && cell_live() {
        cell_close_request("the close box or Alt+F4");
        return false;
    }
    close(a);
    true
}

fn cell_close_request(from: &str) {
    // SAFETY: mainline only.
    unsafe {
        if CELL_REQ_AT == 0 {
            CELL_REQ_AT = ms_now().max(1);
        }
    }
    cq_post(abi::EV_CLOSE_REQUEST, 0);
    crate::kprintln!("[desk] cell: close request ({})", from);
}

/// "Close anyway": only with a request at least FORCE_AFTER_MS old. The window goes at once; the app is stopped
/// at its next backward branch or wait, and every cell call before that returns -2.
fn cell_force(from: &str) -> bool {
    if !cell_live() {
        crate::kprintln!("[desk] cell: close anyway refused: no app window");
        return false;
    }
    if !req_old() {
        crate::kprintln!("[desk] cell: close anyway refused: no close request 3 s old");
        return false;
    }
    let c = App::Cell as usize;
    damage(win_box(c));
    close(c);
    // SAFETY: mainline only.
    unsafe { CELL_FORCED_AT = ms_now() };
    CELL_FORCED.store(true, Ordering::Relaxed);
    cq_clear();
    DIRTY.store(true, Ordering::Relaxed);
    crate::kprintln!("[desk] cell: closed anyway ({}); the app stops at its next loop step", from);
    true
}

/// A byte from the serial line while the app owns the window (nia.rs's console read, and `app_cell_event`'s
/// wait): Ctrl+\ is the close request, or "Close anyway" when a request is 3 s old; any other byte is a key,
/// Ctrl+C (3) included, while the window has the focus. Keys without the focus are dropped.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn cell_serial_byte(b: u8) {
    if !cell_live() {
        return;
    }
    if b == abi::SERIAL_CLOSE {
        if req_old() {
            cell_force("Ctrl+\\ again");
        } else {
            cell_close_request("Ctrl+\\");
        }
        return;
    }
    if focused() == Some(App::Cell as usize) {
        cq_post(abi::EV_KEY, b as u32);
    }
}

/// The grid changed (a drag, a plan step): one resize event per desk pass; the "Close anyway" offer appeared.
fn cell_watch() {
    if !cell_live() {
        return;
    }
    let (cols, rows) = cell_grid();
    let g = abi::grid(cols, rows);
    // SAFETY: mainline only.
    unsafe {
        if g != CELL_GRID {
            CELL_GRID = g;
            cq_post(abi::EV_RESIZE, g);
            crate::kprintln!("[desk] cell: resize event {}x{}", cols, rows);
        }
        let offer = req_old();
        if offer != CELL_OFFERED {
            CELL_OFFERED = offer;
            damage(win_box(App::Cell as usize));
            DIRTY.store(true, Ordering::Relaxed);
            if offer {
                crate::kprintln!("[desk] cell: close anyway offered");
            }
        }
    }
}

fn nonneg(v: i32) -> Result<usize, i32> {
    if v < 0 {
        Err(abi::ERR_BAD_ARG)
    } else {
        Ok(v as usize)
    }
}

fn check_win(win: i32) -> Result<(), i32> {
    if !cell_live() {
        return Err(abi::ERR_NO_WINDOW);
    }
    if win != abi::WIN {
        return Err(abi::ERR_BAD_WINDOW);
    }
    Ok(())
}

/// `cell_open(min_cols, min_rows)`: the running app takes the cell window (a test window from `desk cell open`
/// is taken over and cleared), grown to fit the minimum where the screen allows, with the focus once.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_cell_open(name: &[u8], signed: bool, min_cols: i32, min_rows: i32) -> i32 {
    let (Ok(mc), Ok(mr)) = (nonneg(min_cols), nonneg(min_rows)) else {
        return abi::ERR_BAD_ARG;
    };
    if CELL_FORCED.load(Ordering::Relaxed) {
        return abi::ERR_NO_WINDOW;
    }
    if CELL_OWNED.load(Ordering::Relaxed) {
        return abi::ERR_ALREADY_OPEN;
    }
    // a test label (`desk cell titletest`) never stays on a real app's window
    TITLE_TEST.store(false, Ordering::Relaxed);
    if !SHOWN.load(Ordering::Relaxed) {
        return abi::ERR_NO_DESK;
    }
    finish_motion();
    let c = App::Cell as usize;
    // SAFETY: mainline only.
    unsafe {
        let n = name.len().min(24);
        (&mut *core::ptr::addr_of_mut!(CELL_NAME))[..n].copy_from_slice(&name[..n]);
        CELL_NAME_LEN = n;
        CELL_SIGNED = signed;
        CELL_REQ_AT = 0;
        CELL_OFFERED = false;
    }
    cq_clear();
    CQ_DROPS.store(0, Ordering::Relaxed);
    if st().wins[c].open {
        damage(win_box(c));
    }
    open(c);
    cell_clear();
    let th = title_h();
    let w = &mut st().wins[c];
    w.w = w.w.max(mc.min(CELL_COLS) as i32 * CELL_W + 18).min((W() - SPINE_P - 8).max(40));
    w.w = w.w.max(paint::lintel_min_w().min((W() - SPINE_P - 8).max(40)));
    w.h = w.h.max(mr.min(CELL_ROWS) as i32 * CELL_H + 14 + th).min((H() - TOP - 8).max(30));
    w.x = w.x.min(W() - 8 - w.w).max(SPINE_P);
    w.y = w.y.min(H() - 8 - w.h).max(TOP);
    damage(win_box(c));
    DIRTY.store(true, Ordering::Relaxed);
    CELL_OWNED.store(true, Ordering::Relaxed);
    let (cols, rows) = cell_grid();
    // SAFETY: mainline only.
    unsafe { CELL_GRID = abi::grid(cols, rows) };
    crate::kprintln!(
        "[desk] cell: opened by {} ({}), {}x{} grid",
        core::str::from_utf8(&name[..name.len().min(24)]).unwrap_or("?"),
        if signed { "signed" } else { "unsigned" },
        cols,
        rows
    );
    abi::WIN
}

/// `cell_put(win, col, row, cells)`: nia.rs has capped the length and checked the memory; the clip is stage 1's.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_cell_put(win: i32, col: i32, row: i32, cells: &[u8]) -> i32 {
    if let Err(e) = check_win(win) {
        return e;
    }
    let (Ok(c), Ok(r)) = (nonneg(col), nonneg(row)) else {
        return abi::ERR_BAD_ARG;
    };
    cell_put(c, r, cells) as i32
}

/// `cell_cursor(win, col, row, shape)`: shapes 0 to 3.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_cell_cursor(win: i32, col: i32, row: i32, shape: i32) -> i32 {
    if let Err(e) = check_win(win) {
        return e;
    }
    let (Ok(c), Ok(r), Ok(sh)) = (nonneg(col), nonneg(row), nonneg(shape)) else {
        return abi::ERR_BAD_ARG;
    };
    if sh > 3 {
        return abi::ERR_BAD_ARG;
    }
    cell_cursor(c, r, sh as u8);
    0
}

/// `cell_size(win)`: cols << 14 | rows.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_cell_size(win: i32) -> i32 {
    if let Err(e) = check_win(win) {
        return e;
    }
    let (cols, rows) = cell_grid();
    abi::grid(cols, rows) as i32
}

/// `cell_event(timeout_ms)`: 0 returns after one desk pass, any negative value waits with no limit (never cast to
/// a long u64), a positive one up to that many ms; 0 means no event in time. While it waits the desk runs (input,
/// redraws, plan steps) and serial bytes become keys. After "Close anyway" it returns kind 4 at once.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_cell_event(timeout_ms: i32) -> i32 {
    let closed = abi::event(abi::EV_CLOSED, 0);
    if CELL_FORCED.load(Ordering::Relaxed) || (cell_owned() && !cell_live()) {
        return closed;
    }
    if !cell_owned() {
        return abi::ERR_NO_WINDOW;
    }
    let limit = if timeout_ms < 0 { None } else { Some(timeout_ms as u64) };
    let t0 = ms_now();
    loop {
        while let Some(b) = crate::uart::getc() {
            cell_serial_byte(b);
        }
        idle();
        if CELL_FORCED.load(Ordering::Relaxed) || !cell_live() {
            return closed;
        }
        if let Some(v) = cq_pop() {
            return v;
        }
        if let Some(l) = limit {
            if ms_now().saturating_sub(t0) >= l {
                return 0;
            }
        }
        crate::cpu::wfi(); // the 100 Hz timer or a received byte wakes it
    }
}

/// From the interpreter's loop poll (once per 4096 backward branches): while the app owns a cell window, run the
/// desk at most every TICK_MS, so a stuck app cannot stop the "Close anyway" click from being seen.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn app_tick() {
    if !cell_owned() {
        return;
    }
    let now = crate::timer::cycles();
    let step = crate::timer::hz().max(1) * abi::TICK_MS / 1000;
    // SAFETY: mainline only.
    unsafe {
        if now.wrapping_sub(CELL_TICK_AT) < step {
            return;
        }
        CELL_TICK_AT = now;
    }
    idle();
}

/// The app's run returned or trapped: its cell window closes and everything it left is cleared.
#[allow(dead_code)] // called by nia.rs (section 25)
pub fn cell_app_ended() {
    if !cell_owned() {
        return;
    }
    let c = App::Cell as usize;
    let hash = cell_hash();
    let forced = CELL_FORCED.load(Ordering::Relaxed);
    if st().wins[c].open {
        damage(win_box(c));
        close(c);
    }
    if forced {
        // SAFETY: mainline only.
        let ms = ms_now().saturating_sub(unsafe { CELL_FORCED_AT });
        crate::kprintln!("[desk] cell: the app stopped {} ms after Close anyway", ms);
    }
    CELL_OWNED.store(false, Ordering::Relaxed);
    CELL_FORCED.store(false, Ordering::Relaxed);
    cq_clear();
    // SAFETY: mainline only.
    unsafe {
        CELL_REQ_AT = 0;
        PLAN_N = 0;
        PLAN_I = 0;
    }
    DIRTY.store(true, Ordering::Relaxed);
    crate::kprintln!(
        "[desk] cell window closed: the app ended ({} event(s) dropped, grid hash {:016x})",
        CQ_DROPS.swap(0, Ordering::Relaxed),
        hash
    );
}

// The test plan (`desk cell plan`): steps a suite cannot type while the shell waits for the app.
const PLAN_MAX: usize = 8;
static mut PLAN: [[u8; 32]; PLAN_MAX] = [[0; 32]; PLAN_MAX];
static mut PLAN_LEN: [usize; PLAN_MAX] = [0; PLAN_MAX];
static mut PLAN_N: usize = 0;
static mut PLAN_I: usize = 0;
static mut PLAN_UNTIL: u64 = 0;

fn plan_set(steps: &str) {
    // SAFETY: mainline only.
    unsafe {
        PLAN_N = 0;
        PLAN_I = 0;
        PLAN_UNTIL = 0;
        for one in steps.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            if PLAN_N == PLAN_MAX {
                break;
            }
            let n = one.len().min(32);
            (&mut *core::ptr::addr_of_mut!(PLAN))[PLAN_N][..n].copy_from_slice(&one.as_bytes()[..n]);
            PLAN_LEN[PLAN_N] = n;
            PLAN_N += 1;
        }
        crate::kprintln!("[desk] cell plan: {} step(s)", PLAN_N);
    }
}

/// `desk later`: (when, tile + 1), run by the desk's pass once the shell waits again.
static mut LATER: (u64, usize) = (0, 0);

fn later_step() {
    // SAFETY: mainline only.
    let (when, t) = unsafe { LATER };
    if t == 0 || ms_now() < when {
        return;
    }
    // SAFETY: as above.
    unsafe { LATER = (0, 0) };
    crate::kprintln!("[desk] later: tap {}", tile_id(t - 1));
    tap_tile(t - 1);
}

/// A tap on the Circle's tile i, as a person's click (the Circle opened first if it is closed).
fn tap_tile(i: usize) {
    if !st().launcher {
        open_circle();
    }
    let (tx, ty, ts) = launcher_tile(i);
    down(tx + ts / 2, ty + ts / 2);
    up();
}

/// One plan step per desk pass while an app owns the window.
fn plan_step() {
    if !cell_owned() {
        return;
    }
    // SAFETY: mainline only.
    let (i, n, until) = unsafe { (PLAN_I, PLAN_N, PLAN_UNTIL) };
    if i >= n || ms_now() < until {
        return;
    }
    // SAFETY: mainline only; the step is copied out before it runs.
    let mut buf = [0u8; 32];
    let len = unsafe {
        buf = (*core::ptr::addr_of!(PLAN))[i];
        PLAN_I = i + 1;
        PLAN_LEN[i]
    };
    let step = core::str::from_utf8(&buf[..len]).unwrap_or("");
    crate::kprintln!("[desk] plan: {}", step);
    let mut it = step.split_whitespace();
    let verb = it.next().unwrap_or("");
    let mut num = || it.next().and_then(|v| v.parse::<i32>().ok());
    let c = App::Cell as usize;
    match verb {
        "resize" => {
            if let (Some(w), Some(h)) = (num(), num()) {
                damage(win_box(c));
                let win = &mut st().wins[c];
                win.w = w.clamp(40, (W() - SPINE_P - 8).max(40)).max(cell_min_w());
                win.h = h.clamp(30, (H() - TOP - 8).max(30));
                damage(win_box(c));
                DIRTY.store(true, Ordering::Relaxed);
            }
        }
        "close" => {
            close_or_ask(c);
        }
        "tile" => {
            // a Circle tile tapped while the app runs (section 32's suite): refused with the toast
            if let Some(i) = step.split_whitespace().nth(1).and_then(|id| (0..ntile()).position(|i| tile_id(i) == id)) {
                tap_tile(i);
            }
        }
        "wait" => {
            let ms = num().unwrap_or(0).max(0) as u64;
            // SAFETY: mainline only.
            unsafe { PLAN_UNTIL = ms_now() + ms };
        }
        "force" => {
            cell_force("the plan");
        }
        "run" => {
            // the re-entry test: an app started from inside the desk while one runs must be refused
            let name = step.split_whitespace().nth(1).unwrap_or("");
            let before = cell_hash();
            crate::nia::run_file(name);
            crate::kprintln!("[desk] plan: grid hash {:016x} before the run, {:016x} after", before, cell_hash());
        }
        "verify" => command("verify"),
        _ => crate::kprintln!("[desk] plan: unknown step"),
    }
}

/// Hex digits to bytes, at most `out.len()`; stops at the first pair that is not hex. Returns how many.
fn unhex(h: &str, out: &mut [u8]) -> usize {
    let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
    let mut n = 0;
    for p in h.as_bytes().chunks(2) {
        match (p.first().copied().and_then(d), p.get(1).copied().and_then(d)) {
            (Some(a), Some(b)) if n < out.len() => {
                out[n] = a << 4 | b;
                n += 1;
            }
            _ => break,
        }
    }
    n
}

/// Post a notification (shown as a toast, kept in the bell's list).
pub fn notify(msg: &str) {
    notify_bytes(msg.as_bytes());
}

/// Post one of the desk's own notices, in the desk's language at the moment it is posted.
fn notify_msg(m: lang::Msg) {
    notify_bytes(lang::t(m));
}

fn notify_bytes(b: &[u8]) {
    let s = st();
    let i = s.nnotes % NNOTE;
    // cut at a character's start, never inside a UTF-8 sequence
    let mut n = b.len().min(NOTE_LEN);
    while n > 0 && n < b.len() && (b[n] & 0xC0) == 0x80 {
        n -= 1;
    }
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

    // 0. the language list: a row picks, outside closes (the click goes no further)
    if s.lang_list {
        let (lx, ly, lw, lh) = lang_list_box();
        if inside(px, py, (lx, ly, lw, lh)) {
            let row = (py - ly - 8) / LANG_ROW;
            let i = s.lang_top + row.max(0) as usize;
            if py >= ly + 8 && row < LANG_ROWS as i32 && i < lang::LANGS.len() {
                lang_pick(i);
            }
        } else {
            lang_list_close();
        }
        return;
    }
    // 1. the Circle is modal-ish: a tile opens, outside closes it
    if s.launcher {
        if inside(px, py, launcher_box()) || inside(px, py, circle_info_box()) {
            for i in 0..ntile() {
                let (tx, ty, ts) = launcher_tile(i);
                if inside(px, py, (tx, ty, ts, ts + 14)) {
                    circle_pick(i);
                    break;
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
    // 1b. the power panel: its buttons, or a click elsewhere closes it
    if s.power > 0 {
        let (b1, b2) = power_buttons();
        if inside(px, py, power_box()) {
            match (s.power, inside(px, py, b1), inside(px, py, b2)) {
                (1, true, _) => s.power = 2,
                (1, _, true) => s.power = 3,
                (2, true, _) => power_go(false),
                (3, true, _) => power_go(true),
                (_, _, true) => s.power = 0, // Cancel
                _ => return,
            }
            crate::kprintln!("[desk] power: {}", ["closed", "menu", "shut down now?", "restart now?"][s.power as usize]);
            FULL.store(true, Ordering::Relaxed);
            return;
        }
        s.power = 0;
        FULL.store(true, Ordering::Relaxed);
        if !lying() && inside(px, py, power_btn_box()) {
            crate::kprintln!("[desk] power: closed");
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
        if !lying() && inside(px, py, power_btn_box()) {
            s.power = 1;
            s.notes_open = false;
            crate::kprintln!("[desk] power: menu");
            FULL.store(true, Ordering::Relaxed);
            return;
        }
        if inside(px, py, bell_box()) {
            s.notes_open = true;
            s.unread = 0;
            crate::kprintln!("[desk] notifications open");
            return;
        }
        for i in 0..=NTILES {
            let (_, ty, ts) = dock_tile(i);
            let (tx, _, _) = dock_tile(i);
            let hit = if lying() { (tx - 4, ty - 4, ts + 8, ts + 8) } else { (0, ty - 4, spine_w() - WEAVE, ts + 8) };
            if inside(px, py, hit) {
                s.dock_clicks += 1;
                if i == 0 {
                    open_circle();
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
        if a == App::Cell as usize && cell_offer() && inside(px, py, anyway_box(a)) {
            cell_force("the click");
        } else if inside(px, py, cl) {
            if close_or_ask(a) {
                log(b"close ", a);
            }
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
    // the Language chip under the pointer turns gold (only while Settings is the front window)
    let lh = focused() == Some(App::Settings as usize) && inside(px, py, lang_chip_box());
    if lh != s.lang_hover {
        s.lang_hover = lh;
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
        KEY_UP | KEY_DOWN if s.lang_list => {
            let n = lang::LANGS.len();
            s.lang_sel = if k == KEY_DOWN { (s.lang_sel + 1).min(n - 1) } else { s.lang_sel.saturating_sub(1) };
            lang_list_show_sel();
            crate::kprintln!("[desk] lang: list on {}", lang::LANGS[s.lang_sel].code());
        }
        KEY_ENTER if s.lang_list => lang_pick(s.lang_sel),
        KEY_ESC if s.lang_list => lang_list_close(),
        KEY_ENTER if !s.launcher && focused() == Some(App::Settings as usize) => lang_list_open(),
        KEY_SUPER => {
            if s.launcher {
                s.launcher = false;
            } else {
                open_circle();
            }
            s.notes_open = false;
            crate::kprintln!("[desk] launcher {}", if s.launcher { "open" } else { "closed" });
        }
        KEY_UP | KEY_DOWN | KEY_LEFT | KEY_RIGHT if s.launcher => {
            // walk round the ring: Right and Down clockwise, Left and Up back
            let n = ntile();
            let step = if k == KEY_RIGHT || k == KEY_DOWN { 1 } else { n - 1 };
            s.sel = (s.sel + step) % n;
            crate::kprintln!("[desk] circle: {}", tile_id(s.sel));
        }
        KEY_ENTER if s.launcher => {
            circle_pick(s.sel.min(ntile() - 1));
        }
        _ if (KEY_SUPER_1..KEY_SUPER_1 + NTILES as u32).contains(&k) => {
            s.launcher = false;
            s.notes_open = false;
            activate((k - KEY_SUPER_1) as usize); // as a click on the Spine's tile
        }
        KEY_ESC => {
            if s.power > 0 {
                s.power = 0;
                FULL.store(true, Ordering::Relaxed);
                crate::kprintln!("[desk] power: closed");
            }
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
            } else if let Some(a) = (0..NWIN).find(|&a| s.wins[a].open && s.wins[a].min) {
                open(a); // the only window is minimised: bring it back
                log(b"focus ", a);
            }
        }
        KEY_ALT_F4 => {
            if let Some(a) = focused() {
                if close_or_ask(a) {
                    log(b"close ", a);
                }
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
    damage(win_box(a)); // a selected file, a switch
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
                        3 => set_motion(!s.motion),
                        4 => {
                            if bx >= settings_seg_x(a) && bx < settings_seg_x(a) + 128 {
                                lang_list_open();
                            }
                        }
                        _ => {}
                    }
                    if k == 1 || k == 2 || k == 3 {
                        let on = match k {
                            1 => s.wallpaper,
                            2 => s.clock24,
                            _ => s.motion,
                        };
                        crate::kprintln!(
                            "[desk] settings: {} {}",
                            ["", "wallpaper", "24-hour clock", "motion"][k],
                            if on { "on" } else { "off" }
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

/// The language list (the designer's spec, 14 Nov): 22 px a row, at most LANG_ROWS rows shown, 200 px wide.
const LANG_ROW: i32 = 22;
const LANG_ROWS: usize = 10;
const LANG_W: i32 = 200;

/// The Language chip in Settings, on the screen (as settings_app draws it: the body starts 1 px in).
pub(super) fn lang_chip_box() -> Rect {
    let a = App::Settings as usize;
    let (x, y, _, _) = win_rect(a);
    (x + 1 + settings_seg_x(a), y + title_h() + 1 + settings_rows()[4], 128, 20)
}

/// The list panel: under the chip (above it when it would pass the screen's foot), its right edge on the chip's
/// right edge, or its left edge on the chip's left edge in a right-to-left language.
pub(super) fn lang_list_box() -> Rect {
    let (cx, cy, cw, ch) = lang_chip_box();
    let n = lang::LANGS.len().min(LANG_ROWS) as i32;
    let h = 8 + LANG_ROW * n + 8;
    let x = if lang::current().dir() == lang::Dir::Rtl { cx } else { cx + cw - LANG_W };
    let x = x.clamp(0, (W() - LANG_W).max(0));
    let below = cy + ch + 2;
    let y = if below + h > H() { (cy - 2 - h).max(0) } else { below };
    (x, y, LANG_W, h)
}

/// The rows the list shows: (first, how many).
pub(super) fn lang_list_rows() -> (usize, usize) {
    let s = st();
    (s.lang_top, lang::LANGS.len().saturating_sub(s.lang_top).min(LANG_ROWS))
}

fn lang_list_show_sel() {
    let s = st();
    if s.lang_sel < s.lang_top {
        s.lang_top = s.lang_sel;
    } else if s.lang_sel >= s.lang_top + LANG_ROWS {
        s.lang_top = s.lang_sel + 1 - LANG_ROWS;
    }
}

fn lang_list_open() {
    let s = st();
    s.lang_list = true;
    s.lang_sel = lang::LANGS.iter().position(|&l| l == lang::current()).unwrap_or(0);
    s.lang_top = 0;
    lang_list_show_sel();
    FULL.store(true, Ordering::Relaxed);
    DIRTY.store(true, Ordering::Relaxed);
    crate::kprintln!("[desk] lang: list open");
}

fn lang_list_close() {
    let s = st();
    if s.lang_list {
        s.lang_list = false;
        FULL.store(true, Ordering::Relaxed);
        DIRTY.store(true, Ordering::Relaxed);
        crate::kprintln!("[desk] lang: list closed");
    }
}

/// The wheel over the open list: a row a notch (positive: back up the list).
fn lang_list_wheel(rows: i32) {
    let s = st();
    let max = lang::LANGS.len().saturating_sub(LANG_ROWS) as i32;
    s.lang_top = (s.lang_top as i32 - rows.signum()).clamp(0, max) as usize;
    DIRTY.store(true, Ordering::Relaxed);
}

fn lang_pick(i: usize) {
    let s = st();
    s.lang_list = false;
    let l = lang::LANGS[i.min(lang::LANGS.len() - 1)];
    if l != lang::current() {
        set_lang(l);
    } else {
        FULL.store(true, Ordering::Relaxed);
        DIRTY.store(true, Ordering::Relaxed);
        crate::kprintln!("[desk] lang: {} (already)", l.code());
    }
    crate::kprintln!("[desk] lang: list closed");
}

/// The desk speaks another language (the Settings chip, `desk lang <code>`): every word may change, so the next
/// frame is whole; the choice is kept in afrisovera.conf (on the disk when there is one).
fn set_lang(l: lang::Lang) {
    lang::set(l);
    FULL.store(true, Ordering::Relaxed);
    DIRTY.store(true, Ordering::Relaxed);
    let (_, draft, _, english) = lang::counts(l);
    crate::kprintln!("[desk] lang: {} ({}), {} drafts, {} in English", l.code(), l.own_name(), draft, english);
    if let Err(why) = crate::config::set_lang(l.code()) {
        crate::kprintln!("[desk] lang: not saved: {}", why);
    }
}

/// At the first `gui`: the language saved in afrisovera.conf. English (the default) prints nothing, so a desk with
/// no saved language prints what it always did.
fn apply_saved_lang() {
    let code = crate::config::lang();
    match lang::Lang::from_code(code) {
        Some(lang::Lang::En) => {}
        Some(l) => {
            lang::set(l);
            crate::kprintln!("[desk] lang: {} (from afrisovera.conf)", l.code());
        }
        None => crate::kprintln!("[desk] lang: '{}' in afrisovera.conf is not a language the desk has; English", code),
    }
}

/// Motion on or off (Settings, `desk motion on|off`); off shows every final frame at once.
fn set_motion(on: bool) {
    if !on {
        finish_motion();
    }
    st().motion = on;
}

fn set_touch(on: bool) {
    let s = st();
    if s.touch != on {
        finish_motion();
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

/// Settings: the y of each interactive row in the body (profile, wallpaper, clock, motion, language). The language
/// row is the first one drawn (the top of Display) but the last index, so the older rows keep their numbers.
fn settings_rows() -> [i32; 5] {
    [62, 112, 134, 156, 40]
}
fn settings_seg_x(a: usize) -> i32 {
    let (_, _, w, _) = win_rect(a);
    w - 150
}

// ---------------------------------------------------------------------------------------
// The frame.
// ---------------------------------------------------------------------------------------
/// The time the desktop draws and checks, pinned while `desk verify` runs: its settle and its full redraw then
/// read one instant, so a minute or a second that turns between them is not a damage error. (uptime ticks, RTC)
static mut PINNED: Option<(u64, u64)> = None;
/// The uptime ticks the desktop uses (pinned during `desk verify`).
pub(super) fn now_ticks() -> u64 {
    // SAFETY: mainline only.
    match unsafe { PINNED } {
        Some((t, _)) => t,
        None => crate::sched::ticks(),
    }
}
fn clock_secs() -> u64 {
    // SAFETY: mainline only.
    if let Some((_, c)) = unsafe { PINNED } {
        return c;
    }
    rtc_secs()
}
fn rtc_secs() -> u64 {
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
        apply_saved_lang();
        notify_msg(lang::Msg::NoteWelcome);
    }
    paint::draw();
}

/// `clear`: the text console comes back.
pub fn hide() {
    finish_motion();
    st().shown = false;
    st().drag = None;
    SHOWN.store(false, Ordering::Relaxed);
}

/// Called by the shell before it sleeps: drain input events, redraw when something changed.
pub fn idle() {
    if !SHOWN.load(Ordering::Relaxed) {
        return;
    }
    let before = snap();
    let mut moved: Option<(i32, i32)> = None;
    TERM_MUTE.store(true, Ordering::Relaxed);
    loop {
        let h = EV_HEAD.load(Ordering::Relaxed);
        if h == EV_TAIL.load(Ordering::Acquire) {
            break;
        }
        let v = EVQ[h].load(Ordering::Relaxed);
        EV_HEAD.store((h + 1) % RING, Ordering::Release);
        // SAFETY: mainline only. Any input releases `desk motion hold` (the clock runs on).
        unsafe { HOLD = 0 };
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
    later_step();
    // an app tile is bright only while the shell is free: redraw the Circle when that changes
    let busy = circle_busy();
    if WAS_BUSY.swap(busy, Ordering::Relaxed) != busy && st().launcher && ntile() > NTILES {
        FULL.store(true, Ordering::Relaxed);
    }
    TERM_MUTE.store(false, Ordering::Relaxed);
    cell_watch();
    settle(before);
    WALK.store(st().launcher, Ordering::Relaxed);
    // the copy to the screen happens here, in mainline, never in the timer interrupt
    crate::canvas::flush();
    plan_step();
}

/// The canvas changed size (`screen ui N`): keep every window's lintel on the screen and
/// redraw (afriui.md section 13).
pub fn relayout() {
    finish_motion();
    let s = st();
    for a in 0..NWIN {
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

/// After a batch of input: find the damaged boxes (the snapshot taken before it, the changes
/// that mark themselves) and draw them, or the whole desktop.
fn settle(before: Snap) {
    let s = st();
    let mut want = DIRTY.swap(false, Ordering::Relaxed);
    if TERM_DIRTY.load(Ordering::Relaxed) {
        term_apply();
    }
    if TERM_DIRTY.swap(false, Ordering::Relaxed) && s.wins[App::Terminal as usize].open && !s.wins[App::Terminal as usize].min {
        damage(win_box(App::Terminal as usize)); // new shell text
        want = true;
    }
    let moving = motion_tick();
    want |= moving;
    let t = now_ticks();
    if s.toast_until != 0 && t >= s.toast_until {
        s.toast_until = 0;
        want = true; // the toast's box, from the snapshots
    }
    if t / 100 != LAST_CHECK.swap(t / 100, Ordering::Relaxed) {
        // once a second: what changes with time alone
        let minute = clock_secs() / 60;
        if minute != s.last_minute {
            // the clock at the Spine's foot or the bar's end. Only here is last_minute set, with the damage that
            // redraws it: paint used to set it whenever it drew the bar, even under a clip that left the clock
            // out, and then the new minute never reached the screen
            damage(clock_box());
            s.last_minute = minute;
            want = true;
        }
        // the uptime line in About and Settings and under the Circle
        for a in [App::Settings as usize, App::About as usize] {
            if s.wins[a].open && !s.wins[a].min && !s.touch {
                damage(paint::uptime_box(a));
                want = true;
            } else if s.wins[a].open && !s.wins[a].min {
                damage(win_box(a));
                want = true;
            }
        }
        if s.launcher {
            let (ix, iy, iw, _) = circle_info_box();
            damage((ix, iy + 26, iw, 9));
            want = true;
        }
        // the network: the Spine's glyph, Settings and the Circle show it
        let net = (crate::virtio::net_present(), crate::virtio::dhcp_ip());
        // SAFETY: mainline only.
        if unsafe { core::mem::replace(&mut *core::ptr::addr_of_mut!(LAST_NET), Some(net)) } != Some(net) {
            damage(dock_box());
            for a in [App::Settings as usize] {
                if s.wins[a].open && !s.wins[a].min {
                    damage(win_box(a));
                }
            }
            if s.launcher {
                damage(grow(circle_info_box(), 4));
            }
            want = true;
        }
    }
    let after = snap();
    if want && before != after && !diff(&before, &after) {
        FULL.store(true, Ordering::Relaxed);
    }
    let t0 = crate::timer::cycles();
    frame(want);
    if moving {
        let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
        // SAFETY: mainline only.
        unsafe { MOTION_US = (MOTION_US.0.max(us), MOTION_US.1 + 1) };
    }
}

fn redraw_now() {
    // SAFETY: mainline only.
    unsafe { NDMG_USED = 0 };
    FULL.store(false, Ordering::Relaxed);
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
    // every desk command releases a hold, and all but tap and drag (input, as a click would be)
    // show a running motion's final frame first, so a script sees only final frames
    // SAFETY: mainline only.
    unsafe { HOLD = 0 };
    if verb != "tap" && verb != "drag" && verb != "motion" {
        finish_motion();
    }
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
                            if close_or_ask(a) {
                                log(b"close ", a);
                            }
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
        "cell" => {
            // afriui.md section 24: the verbs that change only cells draw their own partial frame and return
            let tail = rest.trim_start().strip_prefix("cell").unwrap_or("").trim_start();
            let tail = tail.strip_prefix(arg).unwrap_or(tail);
            if !cell_command(arg, tail) {
                return;
            }
        }
        "verify" => {
            // the rows as the partial frames left them, against a full redraw (section 15). The time is pinned
            // from the settle to the full redraw, so both draw the same clock, uptime and toast; and the
            // once-a-second check runs at once, so what turned since the last one is damaged first
            let ch = crate::canvas::height();
            // SAFETY: mainline only; unpinned below.
            unsafe { PINNED = Some((crate::sched::ticks(), rtc_secs())) };
            LAST_CHECK.store(u64::MAX, Ordering::Relaxed);
            settle(snap());
            // SAFETY: mainline only.
            let rows = unsafe { &mut *core::ptr::addr_of_mut!(ROWS) };
            row_hashes(rows);
            paint::draw();
            // SAFETY: as above.
            unsafe { PINNED = None };
            // SAFETY: mainline only.
            let now = unsafe { &mut *core::ptr::addr_of_mut!(ROWS_NOW) };
            row_hashes(now);
            let differ = (0..ch).filter(|&y| now[y] != rows[y]).count();
            // the first rows that differ, so a miss can be found
            let mut first = [usize::MAX; 4];
            for (k, y) in (0..ch).filter(|&y| now[y] != rows[y]).take(4).enumerate() {
                first[k] = y;
            }
            // SAFETY: scalar read.
            let parts = unsafe { paint::PART_US.2 };
            crate::kprintln!("[desk] verify: {} of {} rows differ after a full redraw ({} partial frame(s) drawn before it)", differ, ch, parts);
            if differ > 0 {
                let mut l = Line::new();
                for &y in first.iter().filter(|&&y| y != usize::MAX) {
                    l.s(b" ").u(y as u64);
                }
                crate::kprintln!("[desk] verify: the first rows that differ:{}", core::str::from_utf8(l.get()).unwrap_or(""));
            }
            return;
        }
        "term" => {
            // afriui.md section 28: the Terminal's scrollback from the shell (serial, the suite)
            match arg {
                "up" => term_page(true),
                "down" => term_page(false),
                "bottom" => {
                    TVIEW.store(0, Ordering::Relaxed);
                    TERM_DIRTY.store(true, Ordering::Relaxed);
                }
                "wheel" => term_wheel(num_arg(it.next()).unwrap_or(-1)),
                "status" | "" => {}
                _ => {
                    crate::kprintln!("desk: term up|down|bottom|wheel <notches>|status");
                    return;
                }
            }
            term_apply();
            let v = TVIEW.load(Ordering::Relaxed);
            // the line in the window's top row, found as the window finds its lines (desk log lines left out)
            let rows = term_rows();
            let (mut k, mut seen, mut top_k) = (v, 0, None);
            while seen < rows && k < TL {
                if !term_line(k).starts_with(b"[desk]") {
                    top_k = Some(k);
                    seen += 1;
                }
                k += 1;
            }
            let mut first = [0u8; 40];
            let top = top_k.map(term_line).unwrap_or(&[]);
            let n = top.len().min(40);
            first[..n].copy_from_slice(&top[..n]);
            crate::kprintln!(
                "[desk] term: {} lines kept, {} rows shown, view {} line(s) back; top row: {}",
                // SAFETY: a scalar read in mainline.
                unsafe { TLINES },
                rows,
                v,
                core::str::from_utf8(&first[..n]).unwrap_or("?")
            );
            DIRTY.store(true, Ordering::Relaxed);
        }
        "power" => {
            // the power panel from the shell (serial, the suite): open, off or restart (the confirm), cancel, yes
            match arg {
                "" | "menu" => s.power = 1,
                "off" => s.power = 2,
                "restart" => s.power = 3,
                "cancel" => s.power = 0,
                "yes" if s.power == 2 => power_go(false),
                "yes" if s.power == 3 => power_go(true),
                "yes" => {
                    crate::kprintln!("[desk] power: nothing to confirm (desk power off|restart first)");
                    return;
                }
                _ => {
                    crate::kprintln!("desk: power [menu|off|restart|cancel|yes]");
                    return;
                }
            }
            s.notes_open = false;
            crate::kprintln!("[desk] power: {}", ["closed", "menu", "shut down now?", "restart now?"][s.power as usize]);
        }
        "shape" => match arg {
            // SHAPING_PLAN.md C1: Arabic joined and right to left
            "sample" => {
                // Arabic with lam-alef, a Latin name and Arabic-Indic digits in one notice (43 of its 44 bytes)
                let msg = "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64A}\u{643}\u{645} AfriSovera \u{662}\u{660}\u{662}\u{666}";
                notify(msg);
                crate::kprintln!("[desk] shape sample: a notice with {} characters in {} bytes", msg.chars().count(), msg.len());
            }
            "probe" => {
                if !s.shown {
                    crate::kprintln!("[desk] shape probe: the desk is not showing");
                } else {
                    paint::shape_probe();
                }
                return;
            }
            "hex" => {
                // shape any UTF-8 given in hex (up to 256 bytes) and print its visual order
                let mut b = [0u8; 256];
                let n = unhex(rest.split_whitespace().nth(2).unwrap_or(""), &mut b);
                paint::shape_report(&b[..n]);
                return;
            }
            _ => {
                crate::kprintln!("[desk] shape: desk shape sample|probe|hex <utf-8 in hex>");
                return;
            }
        },
        "look" => {
            // afriui.md section 30: the corners, the shadows, the see-through panels, the run cache, and the probe
            use paint::look::{self, Corners};
            let on = it.next().unwrap_or("");
            match arg {
                "hard" => look::set_corners(Corners::Hard),
                "cut" => look::set_corners(Corners::Cut),
                "round" => look::set_corners(Corners::Round),
                "shadow" if on == "soft" || on == "hard" => look::set_soft(on == "soft"),
                "glass" if on == "on" || on == "off" => look::set_glass(on == "on"),
                "runs" if on == "on" || on == "off" => paint::RUN_ON.store(on == "on", Ordering::Relaxed),
                "flat" => {
                    look::set_corners(Corners::Hard);
                    look::set_soft(false);
                    look::set_glass(false);
                }
                "premium" => {
                    look::set_corners(Corners::Cut);
                    look::set_soft(true);
                    look::set_glass(true);
                }
                "probe" => {
                    if s.shown {
                        paint::look_probe();
                    } else {
                        crate::kprintln!("[desk] look probe: the desk is not showing");
                    }
                    return;
                }
                "" => {}
                _ => {
                    crate::kprintln!("desk: look hard|cut|round, look shadow soft|hard, look glass on|off, look runs on|off, look flat|premium|probe");
                    return;
                }
            }
            let c = match look::corners() {
                Corners::Hard => "hard",
                Corners::Cut => "cut (smooth)",
                Corners::Round => "round",
            };
            crate::kprintln!(
                "[desk] look: corners {}, shadows {}, see-through {}, run cache {}",
                c,
                if look::soft() { "soft" } else { "hard" },
                if look::glass_switch() { "on" } else { "off" },
                if paint::RUN_ON.load(Ordering::Relaxed) { "on" } else { "off" }
            );
            if arg.is_empty() {
                return;
            }
        }
        "font" => match arg {
            // afriui.md section 26: the outline face (the default) or the bitmap face, and the probe
            "outline" | "vector" | "bitmap" => {
                paint::VECTOR.store(arg != "bitmap", Ordering::Relaxed);
                crate::kprintln!("[desk] font: {}", if arg == "bitmap" { "bitmap (the Sovereign face)" } else { "outline (AfriSovera Sans)" });
            }
            "sample" => {
                // a notice in the languages the face is for, from inside the kernel: the shell's line editor takes
                // ASCII only today, so these letters cannot be typed on the serial line yet
                let msg = "Akwaaba! \u{25B} \u{254} \u{14B} \u{1EB9} \u{1ECD} \u{1E63} \u{181} \u{257} \u{199} \u{1B4}";
                notify(msg);
                crate::kprintln!("[desk] font sample: a notice with {} characters in {} bytes", msg.chars().count(), msg.len());
                // and the capitals (the languages engineer's question, 13 Nov)
                let caps = "Capitals: \u{190} \u{186} \u{181} \u{18A} \u{198} \u{1B3} \u{14A}";
                notify(caps);
                crate::kprintln!("[desk] font sample: a notice with {} characters in {} bytes", caps.chars().count(), caps.len());
            }
            "probe" => {
                if !s.shown {
                    crate::kprintln!("[desk] font probe: the desk is not showing");
                } else {
                    paint::font_probe();
                }
                return;
            }
            _ => {
                let on = paint::VECTOR.load(Ordering::Relaxed);
                crate::kprintln!("[desk] font: {} (desk font outline|bitmap|probe|sample)", if on { "outline" } else { "bitmap" });
                return;
            }
        },
        "perf" => {
            // how long the desktop takes on this machine (the 2012 laptop target, afriui.md 8)
            // SAFETY: scalar reads.
            let (dl, dw) = unsafe { paint::DRAW_US };
            // SAFETY: scalar reads.
            let (pl, pw, pn) = unsafe { paint::PART_US };
            let (cl, cw, n, rows) = crate::canvas::copy_perf();
            // SAFETY: scalar reads.
            let (mw, mn) = unsafe { MOTION_US };
            crate::kprintln!("[desk] perf: the slowest motion frame {} us over {} motion frame(s)", mw, mn);
            crate::kprintln!(
                "[desk] perf: {}x{} at UI scale {}x; draw {} us (worst {} us) over {} frame(s); partial {} us (worst {} us) over {} frame(s); copy to the screen {} us (worst {} us) over {} copies, the last {} row(s)",
                W(), H(), crate::canvas::scale(), dl, dw, s.frames - pn, pl, pw, pn, cl, cw, n, rows
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
        "motion" => match arg {
            "on" | "off" => {
                set_motion(arg == "on");
                crate::kprintln!("[desk] motion {}", arg);
            }
            "hold" => {
                // test only: start a motion and stop its frame clock at a step (1 or 2)
                let step = num_arg(it.next()).unwrap_or(0);
                let what = it.next().unwrap_or("");
                if !(1..=2).contains(&step) || !(what == "circle" || App::from(what).is_some()) || !s.motion {
                    crate::kprintln!("desk: motion hold <1|2> <app|circle> (test only; motion must be on)");
                    return;
                }
                finish_motion();
                if what == "circle" {
                    s.notes_open = false;
                    open_circle();
                    crate::kprintln!("[desk] launcher open");
                } else if let Some(a) = App::from(what) {
                    let a = a as usize;
                    if s.wins[a].open && !s.wins[a].min {
                        minimise(a); // so it appears again, and unfolds
                    }
                    let was = s.wins[a].open;
                    open(a);
                    log(if was { b"focus " } else { b"open " }, a);
                }
                // SAFETY: mainline only.
                unsafe { HOLD = step as u8 };
                crate::kprintln!("[desk] motion: {} held at {}/3", if what == "circle" { "grow circle" } else { "unfold window" }, step);
            }
            _ => {
                run_out_motion();
                // SAFETY: mainline only; copies.
                let (n, logs) = unsafe { (MOTIONS, MLOG) };
                crate::kprintln!("[desk] motion {}; {} motion(s) run", if s.motion { "on" } else { "off" }, n);
                // each recorded motion: its frames' ticks (0 = skipped), and whether it was cut short
                for (k, m) in logs.iter().enumerate() {
                    let Some(m) = m else { continue };
                    let mut l = Line::new();
                    match m.what {
                        Motion::Unfold(a) => l.s(b"unfold ").s(APPS[a].id().as_bytes()),
                        Motion::Grow => l.s(b"grow circle"),
                    };
                    l.s(b" ticks");
                    for &t in m.ticks.iter() {
                        l.s(b" ").u(t);
                    }
                    if m.early {
                        l.s(b" early");
                    }
                    crate::kprintln!("[desk] motion {}: {}", if k == 0 { "last" } else { "before" }, core::str::from_utf8(l.get()).unwrap_or("?"));
                }
            }
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
        "circle" => {
            // the Circle's tiles, in the walk's order (section 32)
            let mut l = Line::new();
            for i in 0..ntile() {
                if i > 0 {
                    l.s(b" ");
                }
                l.s(tile_id(i).as_bytes());
            }
            crate::kprintln!(
                "[desk] circle: {} tiles: {}; app tiles {}",
                ntile(),
                core::str::from_utf8(l.get()).unwrap_or("?"),
                if circle_busy() { "dim (the shell is busy)" } else { "ready" }
            );
        }
        "later" => match (num_arg(Some(arg)), it.next(), it.next()) {
            // a suite's step for when the shell is waiting again (a tile tapped while the shell is at its prompt,
            // or while a command waits for a key): `desk later <ms> tile <id>`, one at a time
            (Some(ms), Some("tile"), Some(id)) if (0..ntile()).any(|i| tile_id(i) == id) => {
                let i = (0..ntile()).position(|i| tile_id(i) == id).unwrap_or(0);
                // SAFETY: mainline only.
                unsafe { LATER = (ms_now() + ms.max(0) as u64, i + 1) };
                crate::kprintln!("[desk] later: tile {} in {} ms", id, ms);
            }
            _ => crate::kprintln!("desk: later <ms> tile <id> (ids from desk circle)"),
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
        "lang" => match arg {
            // the desk's language (LANGUAGES_PLAN.md): `desk lang`, `desk lang check`, `desk lang <code>`
            "" => {
                let l = lang::current();
                let (n, draft, checked, english) = lang::counts(l);
                crate::kprintln!(
                    "[desk] lang: {} ({}), {} strings, {} drafts, {} checked, {} in English",
                    l.code(), l.own_name(), n, draft, checked, english
                );
            }
            "selftest" => {
                // the filler can never be steered by its arguments (the architect's condition 2, 13 Nov)
                let (mut ok, mut all) = (0u32, 0u32);
                let mut case = |what: &str, got: &[u8], want: &[u8]| {
                    all += 1;
                    if got == want {
                        ok += 1;
                    } else {
                        crate::kprintln!("[desk] lang selftest: FAIL {}: got {:?}", what, core::str::from_utf8(got).unwrap_or("?"));
                    }
                };
                let fill = |t: &[u8], args: &[lang::Arg]| {
                    let mut l = Line::new();
                    lang::fill_text(t, args, &mut |b: &[u8]| {
                        l.s(b);
                    });
                    l
                };
                case("a file name holding {0} prints as itself", fill(b"{0} bytes", &[lang::Arg::S(b"a{0}b{1}.txt")]).get(), b"a{0}b{1}.txt bytes");
                case("a gap past the arguments draws nothing", fill(b"{0}-{1}-{9}", &[lang::Arg::N(5)]).get(), b"5--");
                case("a lone or unmatched brace is literal", fill(b"a{b{9 }{x}{", &[lang::Arg::N(1)]).get(), b"a{b{9 }{x}{");
                case("two digits are not a gap", fill(b"{12}", &[lang::Arg::N(7)]).get(), b"{12}");
                case("the largest number", fill(b"{0}", &[lang::Arg::N(u64::MAX)]).get(), b"18446744073709551615");
                let long = [b'x'; 300];
                let over = fill(b"<{0}>", &[lang::Arg::S(&long)]);
                all += 1;
                if over.get().len() == LINE_MAX && over.get()[0] == b'<' {
                    ok += 1;
                } else {
                    crate::kprintln!("[desk] lang selftest: FAIL an argument past the line's end stops at {} bytes", LINE_MAX);
                }
                // a safety word in a language that is not checked draws the English
                for &l in lang::LANGS.iter() {
                    for &m in lang::SAFETY.iter() {
                        all += 1;
                        let (_, d, _, _) = lang::counts(l);
                        if l == lang::Lang::En || lang::text_in(l, m) == lang::text_in(lang::Lang::En, m) || d == 0 {
                            ok += 1;
                        } else {
                            crate::kprintln!("[desk] lang selftest: FAIL a safety word in {} is drawn from a draft", l.code());
                        }
                    }
                }
                crate::kprintln!("[desk] lang selftest: {}/{} checks held", ok, all);
            }
            "check" => {
                let mut bad = 0usize;
                lang::check(&mut |code, id, why| {
                    bad += 1;
                    crate::kprintln!("[desk] lang check: {} {}: {}", code, id, why);
                });
                crate::kprintln!("[desk] lang check: {} language(s), {} ids, {} problem(s)", lang::LANGS.len(), lang::COUNT, bad);
            }
            "list" => lang_list_open(),
            "close" => lang_list_close(),
            "pick" => match it.next().and_then(lang::Lang::from_code) {
                Some(l) => {
                    if !st().lang_list {
                        lang_list_open();
                    }
                    lang_pick(lang::LANGS.iter().position(|&o| o == l).unwrap_or(0));
                }
                None => crate::kprintln!("desk: lang pick <code>; codes: {}", lang::codes()),
            },
            "fit" => {
                let mut bad = 0usize;
                for &l in lang::LANGS.iter() {
                    paint::fit_report(l, &mut |id, w, room| {
                        bad += 1;
                        crate::kprintln!("[desk] lang fit: {} {}: {} px in {} px", l.code(), id, w, room);
                    });
                }
                crate::kprintln!("[desk] lang fit: {} language(s), {} too wide", lang::LANGS.len(), bad);
            }
            code => match lang::Lang::from_code(code) {
                Some(l) => set_lang(l),
                None => crate::kprintln!("desk: lang [check|fit|<code>]; codes: {}", lang::codes()),
            },
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
            for a in 0..NWIN {
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
            crate::kprintln!("      drag <x> <y> <dx> <dy>, mode pointer|touch, notify <text>, notes, perf, verify, select <file>,");
            crate::kprintln!("      move <app> <dx> <dy>, motion [on|off], font outline|bitmap|probe, list");
            crate::kprintln!("      motion hold <1|2> <app|circle> (test only: stops a motion at a frame until the next command or input)");
            return;
        }
    }
    if verb != "list" {
        redraw_now();
    }
}
