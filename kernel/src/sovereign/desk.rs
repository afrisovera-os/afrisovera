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
const NIGHT: u32 = 0x10_0C_0A; // the Spine: warm black, not grey
const WALL_TOP: u32 = 0x1B_15_12; // soil gradient
const WALL_BOT: u32 = 0x0A_08_07;
const MUD: u32 = 0x24_1C_17; // mud-cloth marks, one shade above the soil
const SURFACE: u32 = 0x1E_18_14;
const SURFACE2: u32 = 0x2A_21_1B; // selected rows, lit tiles
const TITLE_ON: u32 = 0x2A_21_1B; // the lintel (title bar) with focus
const TITLE_OFF: u32 = 0x1C_16_13;
const SURFACE_OFF: u32 = 0x16_12_10; // unfocused window bodies sink back (review D1)
const LINE: u32 = 0x4A_3B_2F; // lifted from 3A2E25 (review S1: window edges must separate)
const TEXT: u32 = 0xF2_EA_DC; // shea
const DIM: u32 = 0xB0_A3_91;
const GOLD: u32 = 0xF2_C1_4E;
const GOLD_DEEP: u32 = 0xC8_96_3A;
const GOLD_DARK: u32 = 0x5E_47_21;
const GOLD_PALE: u32 = 0xFF_EB_B8;
const RED: u32 = 0xC1_12_1F;
const GREEN: u32 = 0x2E_9E_6A; // malachite
const OCHRE: u32 = 0xD0_8A_2E;
const INDIGO: u32 = 0x34_50_8F; // adire
const CHARCOAL: u32 = 0x26_1F_1A;
const SHADOW: u32 = 0x05_04_03;
const TERM_BG: u32 = 0x0E_0B_09;
const TERM_TEXT: u32 = 0xD8_CE_BE;
/// The system cloth (the Spine's woven edge): the 8 Nov kente band, kept.
const KENTE: [u32; 6] = [GOLD_DEEP, NIGHT, GREEN, GOLD_DEEP, RED, NIGHT];
/// Each app's cloth (section 4): its focus mark, woven along the top of its lintel.
const CLOTH: [[u32; 3]; 4] = [[OCHRE, NIGHT, GOLD], [GREEN, NIGHT, GOLD], [INDIGO, TEXT, GOLD], [GOLD, RED, GREEN]];

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

/// Default pointer-profile windows: a cascade that keeps every title bar visible.
/// (x is from the screen's left edge; every x is right of the Spine.)
const HOME: [(i32, i32, i32, i32); NAPPS] = [(60, 20, 372, 250), (210, 120, 410, 260), (130, 40, 380, 300), (270, 60, 330, 300)];

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
/// While an app draws its body, everything it draws is clipped to its window, and the
/// window's bottom-right cut corner stays cut: (x, y, w, h, cut). Mainline only.
static mut CLIP: Option<(i32, i32, i32, i32, i32)> = None;

fn rect(x: i32, y: i32, w: i32, h: i32, c: u32) {
    let (mut x0, mut y0) = (x.max(0), y.max(0));
    let (mut x1, mut y1) = ((x + w).min(W()), (y + h).min(H()));
    // SAFETY: CLIP is set and cleared around an app's body drawing, in mainline only.
    if let Some((cx, cy, cw, ch, k)) = unsafe { CLIP } {
        x0 = x0.max(cx);
        y0 = y0.max(cy);
        x1 = x1.min(cx + cw);
        y1 = y1.min(cy + ch);
        let zone = cy + ch - k; // the rows the bottom-right cut runs through
        if k > 0 && y1 > zone && x1 > cx + cw - k && x1 > x0 {
            if y0 < zone {
                crate::canvas::fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (zone - y0) as usize, c);
            }
            for yy in y0.max(zone)..y1 {
                let r = (cx + cw - (yy - zone)).min(x1); // row j of the cut is inset by j + 1
                if r > x0 {
                    crate::canvas::fill_rect(x0 as usize, yy as usize, (r - x0) as usize, 1, c);
                }
            }
            return;
        }
    }
    if x1 > x0 && y1 > y0 {
        crate::canvas::fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, c);
    }
}

/// How far row j of a shape h rows high is inset on the left and on the right, for corners
/// cut at 45 degrees: tl, tr, br, bl pixels (sovereign-ui.md section 3).
fn cut_inset(j: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32) -> (i32, i32) {
    let up = |c: i32| if j < c { c - j } else { 0 };
    let down = |c: i32| if h - 1 - j < c { c - (h - 1 - j) } else { 0 };
    (up(tl).max(down(bl)), up(tr).max(down(br)))
}

/// A filled shape with cut corners.
fn cut(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    let top = tl.max(tr).min(h);
    let bot = (h - br.max(bl)).max(top);
    for j in (0..top).chain(bot..h) {
        let (l, r) = cut_inset(j, h, tl, tr, br, bl);
        rect(x + l, y + j, w - l - r, 1, c);
    }
    rect(x, y + top, w, bot - top, c); // the straight middle in one fill
}

/// The one-pixel outline of a cut shape.
fn cut_frame(x: i32, y: i32, w: i32, h: i32, tl: i32, tr: i32, br: i32, bl: i32, c: u32) {
    for j in 0..h {
        let (l, r) = cut_inset(j, h, tl, tr, br, bl);
        if j == 0 || j == h - 1 {
            rect(x + l, y + j, w - l - r, 1, c);
        } else {
            rect(x + l, y + j, 1, 1, c);
            rect(x + w - r - 1, y + j, 1, 1, c);
        }
    }
}

/// A tile, button or chip: all four corners cut, k pixels.
fn chip(x: i32, y: i32, w: i32, h: i32, k: i32, c: u32) {
    cut(x, y, w, h, k, k, k, k, c);
}

/// A woven band (kente rhythm): `rows` rows of 2 px, blocks of 8 px in the cloth's colours,
/// each row offset by half a block, as strips of cloth are. Row j of the band is kept inside
/// [x + inset(j).0, x + w - inset(j).1) so it follows a cut corner.
fn weave(x: i32, y: i32, w: i32, rows: i32, cloth: &[u32], inset: &dyn Fn(i32) -> (i32, i32)) {
    for row in 0..rows {
        for j in 0..2 {
            let yy = row * 2 + j;
            let (l, r) = inset(yy);
            let (lo, hi) = (x + l, x + w - r);
            let mut bx = x - (row % 2) * 4;
            let mut k = row as usize * 3;
            while bx < hi {
                let (a, b) = (bx.max(lo), (bx + 8).min(hi));
                if b > a {
                    rect(a, y + yy, b - a, 1, cloth[k % cloth.len()]);
                }
                bx += 8;
                k += 1;
            }
        }
    }
}

/// The same weave standing up (the Spine's edge): columns of 2 px, blocks 8 px tall.
fn weave_v(x: i32, y: i32, h: i32, cols: i32, cloth: &[u32]) {
    for col in 0..cols {
        let mut by = y - (col % 2) * 4;
        let mut k = col as usize * 3;
        while by < y + h {
            let (a, b) = (by.max(y), (by + 8).min(y + h));
            rect(x + col * 2, a, 2, b - a, cloth[k % cloth.len()]);
            by += 8;
            k += 1;
        }
    }
}

fn lerp(a: u32, b: u32, t: i32) -> u32 {
    let t = t.clamp(0, 100);
    let ch = |sh: u32| -> u32 {
        let av = ((a >> sh) & 0xff) as i32;
        let bv = ((b >> sh) & 0xff) as i32;
        (av + (bv - av) * t / 100) as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// The glyph for a byte: lower case from the PC font table on both kernels (the desktop
/// only; the ARM text console keeps its capitals), the rest from the shared font.
fn glyph(c: u8) -> [u8; 8] {
    if let Some(g) = crate::font_lower::glyph(c) {
        return g;
    }
    if let Some(g) = punct(c) {
        return g;
    }
    crate::font::glyph(c)
}

/// Punctuation the shared font lacks (it draws a block for them), in its style: 5 pixels
/// wide in bits 7..3, 7 rows. The Terminal shows real shell output, which uses all of these.
fn punct(c: u8) -> Option<[u8; 8]> {
    Some(match c {
        b';' => [0, 0b01100000, 0b01100000, 0, 0b01100000, 0b00100000, 0b01000000, 0],
        b'?' => [0b01110000, 0b10001000, 0b00001000, 0b00110000, 0b00100000, 0, 0b00100000, 0],
        b'"' => [0b01010000, 0b01010000, 0b01010000, 0, 0, 0, 0, 0],
        b'#' => [0b01010000, 0b01010000, 0b11111000, 0b01010000, 0b11111000, 0b01010000, 0b01010000, 0],
        b'@' => [0b01110000, 0b10001000, 0b10111000, 0b10101000, 0b10111000, 0b10000000, 0b01110000, 0],
        b'%' => [0b11000000, 0b11001000, 0b00010000, 0b00100000, 0b01000000, 0b10011000, 0b00011000, 0],
        b'&' => [0b01100000, 0b10010000, 0b10100000, 0b01000000, 0b10101000, 0b10010000, 0b01101000, 0],
        b'$' => [0b00100000, 0b01111000, 0b10100000, 0b01110000, 0b00101000, 0b11110000, 0b00100000, 0],
        b'|' => [0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0b00100000, 0],
        b'\\' => [0b10000000, 0b10000000, 0b01000000, 0b00100000, 0b00010000, 0b00001000, 0b00001000, 0],
        b'{' => [0b00110000, 0b01000000, 0b01000000, 0b10000000, 0b01000000, 0b01000000, 0b00110000, 0],
        b'}' => [0b01100000, 0b00010000, 0b00010000, 0b00001000, 0b00010000, 0b00010000, 0b01100000, 0],
        b'~' => [0, 0, 0b01000000, 0b10101000, 0b00010000, 0, 0, 0],
        b'^' => [0b00100000, 0b01010000, 0b10001000, 0, 0, 0, 0, 0],
        b'`' => [0b01000000, 0b00100000, 0, 0, 0, 0, 0, 0],
        _ => return None,
    })
}

fn ink(g: &[u8; 8]) -> Option<(i32, i32)> {
    let (mut l, mut r) = (8, -1);
    for &bits in g.iter() {
        for col in 0..8 {
            if (bits >> (7 - col)) & 1 == 1 {
                l = l.min(col);
                r = r.max(col);
            }
        }
    }
    if r < l {
        None
    } else {
        Some((l, r))
    }
}

/// Proportional text (as framebuffer::draw_string) with lower case; returns the end x.
fn text(x0: i32, y0: i32, s: &[u8], c: u32, scale: i32) -> i32 {
    let mut x = x0;
    for &b in s {
        if b == b' ' {
            x += 4 * scale;
            continue;
        }
        let g = glyph(b);
        let Some((l, r)) = ink(&g) else {
            x += 4 * scale;
            continue;
        };
        for (row, bits) in g.iter().enumerate() {
            for col in l..=r {
                if (bits >> (7 - col)) & 1 == 1 {
                    rect(x + (col - l) * scale, y0 + row as i32 * scale, scale, scale, c);
                }
            }
        }
        x += (r - l + 2) * scale;
    }
    x
}

fn text_w(s: &[u8], scale: i32) -> i32 {
    let mut w = 0;
    for &b in s {
        w += match if b == b' ' { None } else { ink(&glyph(b)) } {
            Some((l, r)) => (r - l + 2) * scale,
            None => 4 * scale,
        };
    }
    (w - scale).max(0)
}

/// Monospace text (the Terminal), clipped to `cols` characters of 6 px.
fn mono(x: i32, y: i32, s: &[u8], c: u32, cols: usize) {
    for (i, &b) in s.iter().take(cols).enumerate() {
        let g = glyph(b);
        for (row, bits) in g.iter().enumerate() {
            for col in 0..6 {
                if (bits >> (7 - col)) & 1 == 1 {
                    rect(x + i as i32 * 6 + col, y + row as i32, 1, 1, c);
                }
            }
        }
    }
}

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
fn monument(cx: i32, top: i32, h: i32, c_top: u32, c_bot: u32) {
    for t in 0..h {
        let f = t * 100 / h.max(1);
        let outer = (6 + (128 * f * f) / 10000 + (30 * f) / 100) * h / 288;
        let outer = outer.max(1);
        let seam = (10 * (100 - f) / 100) * h / 288;
        let c = lerp(c_top, c_bot, f);
        let y = top + t;
        if cx - seam > cx - outer {
            rect(cx - outer, y, outer - seam, 1, c);
        }
        if cx + outer > cx + seam {
            rect(cx + seam, y, outer - seam, 1, c);
        }
    }
    let pw = (300 * h / 288).max(6);
    let ph = (6 * h / 288).max(1);
    rect(cx - pw / 2, top + h, pw, ph, c_top);
}

// ---------------------------------------------------------------------------------------
// Icons (section 5), drawn at any size s >= 12.
// ---------------------------------------------------------------------------------------
fn icon(app: Option<App>, x: i32, y: i32, s: i32) {
    let u = |v: i32| v * s / 28; // design units on a 28 px grid
    let r = (s / 7).max(2); // the tile's cut corners
    match app {
        None => {
            // the Circle button: the monument on night, in a gold-dark cut frame
            chip(x, y, s, s, r, NIGHT);
            cut_frame(x, y, s, s, r, r, r, r, GOLD_DARK);
            monument(x + s / 2, y + u(4), u(17), GOLD_PALE, GOLD_DEEP);
        }
        Some(App::Files) => {
            chip(x, y, s, s, r, 0x33_22_12);
            rect(x + u(4), y + u(7), u(9), u(3), OCHRE); // tab
            cut(x + u(4), y + u(9), u(20), u(13), 0, (s / 14).max(1), (s / 14).max(1), 0, OCHRE);
            rect(x + u(4), y + u(12), u(20), u(1).max(1), 0xE8_A8_52); // fold highlight
        }
        Some(App::Terminal) => {
            chip(x, y, s, s, r, CHARCOAL);
            rect(x + u(3), y + u(4), u(22), u(1).max(1), LINE);
            // ">" then "_"
            let t = u(2).max(1);
            for k in 0..u(5).max(2) {
                rect(x + u(6) + k, y + u(10) + k, t, t, GOLD);
                rect(x + u(6) + k, y + u(10) + 2 * u(5).max(2) - k - 1, t, t, GOLD);
            }
            rect(x + u(14), y + u(19), u(8), t, GOLD);
        }
        Some(App::Settings) if s < 20 => {
            // at title-bar size: two bold sliders (three thin ones turn to mush, review D1)
            chip(x, y, s, s, r, INDIGO);
            rect(x + 2, y + s / 3, s - 4, 1, 0xB8_C0_E0);
            rect(x + 2, y + 2 * s / 3, s - 4, 1, 0xB8_C0_E0);
            rect(x + s / 4, y + s / 3 - 2, 3, 5, GOLD);
            rect(x + s - s / 4 - 3, y + 2 * s / 3 - 2, 3, 5, GOLD);
        }
        Some(App::Settings) => {
            chip(x, y, s, s, r, INDIGO);
            for (k, at) in [(0, 9), (1, 16), (2, 6)] {
                let yy = y + u(7) + k * u(7);
                rect(x + u(5), yy, u(18), u(1).max(1), 0xB8_C0_E0);
                rect(x + u(5) + u(at), yy - u(2), u(3).max(2), u(5).max(3), GOLD);
            }
        }
        Some(App::About) => {
            // the About tile wears the brand's kente cloth along its foot
            chip(x, y, s, s, r, 0x1A_14_10);
            monument(x + s / 2, y + u(4), u(15), GOLD_PALE, GOLD_DEEP);
            if s >= 20 {
                let k = r;
                weave(x, y + s - u(5), s, 1, &CLOTH[3], &|j: i32| {
                    let h = u(5);
                    cut_inset(j + s - h, s, k, k, k, k)
                });
            }
        }
    }
}

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
        let (x, y, w, h) = HOME[a];
        // keep the default inside the screen whatever its size (section 7)
        let sw = SPINE_P;
        let w = w.min(W() - sw - 8);
        let h = h.min(H() - TOP - 8);
        s.wins[a] = Win { open: true, min: false, x: x.clamp(sw + 4, W() - w), y: y.clamp(TOP, H() - h), w, h };
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

fn wallpaper() {
    let sw = spine_w();
    for y in 0..H() {
        rect(sw, y, W() - sw, 1, lerp(WALL_TOP, WALL_BOT, y * 100 / H()));
    }
    if st().wallpaper {
        // mud cloth (bogolanfini): a grid of small marks, a dot and a chevron in turn, one
        // shade above the soil, so they are felt more than seen (sovereign-ui.md section 6)
        let (mut y, mut j) = (12, 0);
        while y < H() - 8 {
            let (mut x, mut i) = (sw + 12, 0);
            while x < W() - 8 {
                if (i + j) % 2 == 0 {
                    rect(x, y, 2, 2, MUD);
                } else {
                    for k in 0..4 {
                        rect(x - 3 + k, y - 1 + k, 1, 1, MUD);
                        rect(x + 3 - k, y - 1 + k, 1, 1, MUD);
                    }
                }
                x += 24;
                i += 1;
            }
            y += 24;
            j += 1;
        }
        // the monument, faint, standing behind the windows; a warm glow at its foot
        let cx = sw + (W() - sw) / 2;
        let mh = (H() * 55 / 100).max(60);
        let top = (H() - mh) / 2 - 4;
        for k in 0..24 {
            let c = lerp(0x24_18_0E, WALL_BOT, k * 100 / 24);
            rect(cx - 200 + k * 4, top + mh + 6 + k / 3, 400 - k * 8, 1, c);
        }
        monument(cx, top, mh, 0x40_30_17, 0x20_17_0C);
        let word = b"AfriSovera";
        let tw = text_w(word, 2);
        text(cx - tw / 2, top + mh + 14, word, 0x33_27_15, 2);
    }
}

/// The Spine: the Circle button, the app tiles with their beads, then at the foot the
/// network, power, the bell and the clock; a woven edge on its right (section 6).
fn spine() {
    let s = st();
    let body = spine_w() - WEAVE;
    rect(0, 0, body, H(), NIGHT);
    weave_v(body, 0, H(), 2, &KENTE);
    // the Circle button
    let (tx, ty, ts) = dock_tile(0);
    if s.launcher {
        chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, GOLD_DEEP);
    }
    icon(None, tx, ty, ts);
    rect(8, ty + ts + 6, body - 16, 1, LINE);
    // the apps, each with a bead on the left edge: long and gold with focus, short when
    // open, a dot when minimised
    let foc = focused();
    for i in 1..=NAPPS {
        let (tx, ty, ts) = dock_tile(i);
        let a = i - 1;
        if foc == Some(a) {
            chip(tx - 2, ty - 2, ts + 4, ts + 4, 5, SURFACE2);
        }
        icon(Some(APPS[a]), tx, ty, ts);
        let w = s.wins[a];
        if w.open {
            let focus = foc == Some(a);
            let bh = if focus { ts - 8 } else if w.min { 4 } else { 10 };
            chip(0, ty + (ts - bh) / 2, 3, bh, 1, if focus { GOLD } else { GOLD_DEEP });
        }
    }

    // the foot, bottom up: clock, bell, power, network
    let mid = body / 2;
    let secs = clock_secs();
    let (hh, mm) = ((secs / 3600) % 24, (secs / 60) % 60);
    let mut l = Line::new();
    let mut under = Line::new();
    if s.clock24 {
        l.two(hh).s(b":").two(mm);
        under.s(b"UTC");
    } else {
        let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
        l.u(h12).s(b":").two(mm);
        under.s(if hh < 12 { b"am" } else { b"pm" });
    }
    s.last_minute = secs / 60;
    rect(8, H() - 30, body - 16, 1, LINE);
    text(mid - text_w(l.get(), 1) / 2, H() - 23, l.get(), GOLD, 1);
    text(mid - text_w(under.get(), 1) / 2, H() - 12, under.get(), DIM, 1);

    // bell, with the count of unread notices
    let (_, by, _, _) = bell_box();
    let gx = mid - 13;
    let gy = by - 3;
    let bc = if s.notes_open { GOLD } else { TEXT };
    rect(gx + 9, gy + 6, 8, 7, bc);
    rect(gx + 8, gy + 12, 10, 2, bc);
    rect(gx + 11, gy + 15, 4, 2, bc);
    if s.unread > 0 {
        chip(gx + 17, gy + 2, 10, 10, 2, RED);
        let mut t = [0u8; 20];
        text(gx + 20, gy + 3, num(s.unread as u64, &mut t), TEXT, 1);
    }
    // power: no battery driver yet, so a plug and "AC" (no fake levels)
    let (px, py) = (mid - 11, H() - 78);
    rect(px, py + 3, 7, 6, TEXT);
    rect(px + 1, py, 1, 3, TEXT);
    rect(px + 5, py, 1, 3, TEXT);
    rect(px + 3, py + 9, 1, 3, TEXT);
    text(px + 10, py + 2, b"AC", DIM, 1);
    // network: a link glyph, gold with a lease, dim without, crossed without a device
    let (nx, ny) = (mid - 10, H() - 100);
    let (present, ip) = (crate::virtio::net_present(), crate::virtio::dhcp_ip());
    let c = if !present {
        LINE
    } else if ip != [0u8; 4] {
        GOLD
    } else {
        DIM
    };
    rect(nx, ny + 1, 8, 6, c);
    rect(nx + 12, ny + 7, 8, 6, c);
    rect(nx + 3, ny + 7, 2, 4, c);
    rect(nx + 3, ny + 10, 12, 2, c);
    rect(nx + 15, ny + 4, 2, 4, c);
    if !present {
        for k in 0..14 {
            rect(nx + 3 + k, ny + k, 2, 1, RED);
        }
    }
}

fn window(a: usize) {
    let s = st();
    let (x, y, w, h) = win_rect(a);
    let th = title_h();
    let focus = focused() == Some(a);
    let k = win_cut();
    let ins = |j: i32| cut_inset(j, h, k, 0, k, 0);
    if !s.touch {
        // a hard relief shadow, down and right, as a carving's
        cut(x + 4, y + 4, w, h, k, 0, k, 0, SHADOW);
    }
    cut(x, y, w, h, k, 0, k, 0, if focus { SURFACE } else { SURFACE_OFF });
    // the lintel (title bar)
    for j in 0..th {
        let (l, r) = ins(j);
        rect(x + l, y + j, w - l - r, 1, if focus { TITLE_ON } else { TITLE_OFF });
    }
    rect(x, y + th, w, 1, LINE);
    let band = 6; // the cloth: three woven rows (4 px was too thin to read focus at a glance, review S1)
    let isz = if s.touch { 16 } else { 12 };
    icon(Some(APPS[a]), x + 12, y + band + (th - band - isz) / 2, isz);
    let ts = if s.touch { 2 } else { 1 };
    text(x + 12 + isz + 8, y + band + (th - band - 8 * ts) / 2, APPS[a].name().as_bytes(), if focus { TEXT } else { DIM }, ts);
    // buttons: minimise (pointer only) and close, cut-corner chips
    let (cl, mn) = title_buttons(a);
    let hot = s.hover_close == Some(a);
    if hot {
        chip(cl.0, cl.1, cl.2, cl.3, 3, RED);
    }
    let c = if focus || hot { TEXT } else { DIM };
    let (cx, cy) = (cl.0 + cl.2 / 2, cl.1 + cl.3 / 2);
    let arm = if s.touch { 6 } else { 4 };
    for k in -arm..=arm {
        rect(cx + k, cy + k, 2, 1, c);
        rect(cx + k, cy - k, 2, 1, c);
    }
    if !s.touch {
        rect(mn.0 + mn.2 / 2 - 4, mn.1 + mn.3 / 2 + 2, 9, 2, c);
    }
    // body: whatever the app draws stays inside the window and keeps its cut corner
    let (bx, by, bw, bh) = (x + 1, y + th + 1, w - 2, h - th - 2);
    // SAFETY: mainline only; cleared right after the body is drawn.
    unsafe { CLIP = Some((bx, by, bw, bh, (k - 1).max(0))) };
    match APPS[a] {
        App::Files => files_app(a, bx, by, bw, bh),
        App::Terminal => terminal_app(bx, by, bw, bh),
        App::Settings => settings_app(a, bx, by, bw, bh),
        App::About => about_app(bx, by, bw, bh),
    }
    // SAFETY: as above.
    unsafe { CLIP = None };
    // the outline, then the cloth along the top of the focused window's lintel
    cut_frame(x, y, w, h, k, 0, k, 0, if focus { GOLD_DEEP } else { LINE }); // review S1: gold-dark was too muddy
    if focus {
        weave(x, y, w, band / 2, &CLOTH[a], &ins);
    }
}

fn files_app(a: usize, x: i32, y: i32, w: i32, h: i32) {
    let s = st();
    let split = files_split(a);
    rect(x, y, split, 20, SURFACE2);
    text(x + 10, y + 6, b"Name", DIM, 1);
    text(x + split - 44, y + 6, b"Size", DIM, 1);
    rect(x + split, y, 1, h, LINE);
    let mut row = 0;
    for i in 0..crate::vfs::MAX_FILES {
        if !crate::vfs::is_used(i) {
            continue;
        }
        let rh = file_row_h();
        let ry = y + 22 + row * rh;
        if ry + rh > y + h {
            break;
        }
        let sel = s.files_sel == Some(i);
        if sel {
            rect(x + 2, ry, split - 4, rh - 1, 0x2C_25_14);
            rect(x + 2, ry, 2, rh - 1, GOLD);
        }
        let name = crate::vfs::name_of(i).as_bytes();
        let wasm = name.ends_with(b".wasm");
        // a small page icon, ochre for data, gold for programs
        let cy = ry + (rh - 16) / 2; // the row's content, centred in a touch row
        if st().touch && !sel && row > 0 {
            rect(x + 10, ry, split - 20, 1, LINE); // touch rows are separated, as on a phone list
        }
        rect(x + 10, cy + 3, 7, 9, if wasm { GOLD_DEEP } else { OCHRE });
        rect(x + 14, cy + 3, 3, 3, if sel { 0x2C_25_14 } else { SURFACE });
        text(x + 23, cy + 4, name, if sel { GOLD } else { TEXT }, 1);
        let mut t = [0u8; 20];
        let n = num(crate::vfs::size_of(i) as u64, &mut t);
        text(x + split - 10 - text_w(n, 1), cy + 4, n, DIM, 1);
        row += 1;
    }
    if row == 0 {
        text(x + 10, y + 30, b"No files yet", DIM, 1);
    }
    // the preview pane
    let (px, pw) = (x + split + 1, w - split - 1);
    match s.files_sel {
        Some(i) if crate::vfs::is_used(i) => {
            let name = crate::vfs::name_of(i);
            text(px + 10, y + 8, name.as_bytes(), GOLD, 1);
            let mut l = Line::new();
            l.u(crate::vfs::size_of(i) as u64).s(b" bytes");
            text(px + 10, y + 20, l.get(), DIM, 1);
            rect(px + 10, y + 32, pw - 20, 1, LINE);
            let data = crate::vfs::read_raw(name).unwrap_or(&[]);
            let printable = data.iter().take(512).all(|&c| c == b'\n' || c == b'\r' || c == b'\t' || (0x20..0x7f).contains(&c));
            if name.ends_with(".wasm") {
                text(px + 10, y + 42, b"WebAssembly program", TEXT, 1);
                text(px + 10, y + 56, b"Run it from Terminal:", DIM, 1);
                let mut l = Line::new();
                l.s(b"wasm ").s(name.as_bytes());
                text(px + 10, y + 70, l.get(), GOLD_DEEP, 1);
            } else if printable {
                let cols = ((pw - 20) / 6).max(1) as usize;
                let mut yy = y + 42;
                'lines: for line in data.split(|&c| c == b'\n') {
                    // wrapped at the pane's width, at a space when there is one
                    let mut rest = line;
                    loop {
                        if yy + 10 > y + h {
                            break 'lines;
                        }
                        let mut cut = rest.len().min(cols);
                        if cut < rest.len() {
                            if let Some(sp) = rest[..cut].iter().rposition(|&c| c == b' ') {
                                if sp > 0 {
                                    cut = sp + 1;
                                }
                            }
                        }
                        mono(px + 10, yy, &rest[..cut], TEXT, cols);
                        yy += 11;
                        rest = &rest[cut..];
                        if rest.is_empty() {
                            break;
                        }
                    }
                }
            } else {
                text(px + 10, y + 42, b"Binary data", TEXT, 1);
            }
        }
        _ => {
            let m = b"Select a file";
            text(px + (pw - text_w(m, 1)) / 2, y + h / 2 - 4, m, DIM, 1);
        }
    }
}

fn terminal_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, TERM_BG);
    let cols = ((w - 16) / 6).max(1) as usize;
    let rows = ((h - 12) / 11).max(1) as usize;
    // the newest lines that are not desktop log lines, bottom up
    let mut shown = [0usize; TL];
    let mut n = 0;
    let mut k = 0;
    while n < rows && k < TL {
        let l = term_line(k);
        if !l.starts_with(b"[desk]") {
            shown[n] = k;
            n += 1;
        }
        k += 1;
    }
    for j in 0..n {
        let line = term_line(shown[j]);
        let yy = y + 6 + (n - 1 - j) as i32 * 11;
        let prompt = line.starts_with(b"afrisovera>");
        if prompt {
            mono(x + 8, yy, &line[..11], GOLD, cols);
            mono(x + 8 + 12 * 6, yy, &line[11.min(line.len())..], TEXT, cols.saturating_sub(12));
        } else {
            mono(x + 8, yy, line, TERM_TEXT, cols);
        }
        if j == 0 {
            // the cursor after the line being typed
            let cx = x + 8 + (line.len().min(cols) as i32) * 6;
            rect(cx + 1, yy, 5, 8, GOLD);
        }
    }
}

fn switch(x: i32, y: i32, on: bool) {
    chip(x, y, 30, 14, 3, if on { GOLD_DEEP } else { LINE });
    chip(if on { x + 17 } else { x + 3 }, y + 2, 10, 10, 2, if on { NIGHT } else { DIM });
}

fn settings_app(a: usize, x: i32, y: i32, w: i32, _h: i32) {
    let s = st();
    let label = |yy: i32, t: &[u8]| {
        text(x + 14, yy, t, TEXT, 1);
    };
    let head = |yy: i32, t: &[u8]| {
        text(x + 14, yy, t, GOLD_DEEP, 1);
        rect(x + 14 + text_w(t, 1) + 6, yy + 4, w - 40 - text_w(t, 1), 1, LINE);
    };
    head(y + 10, b"Display");
    let rows = settings_rows();
    let mut l = Line::new();
    l.u(W() as u64).s(b" x ").u(H() as u64).s(b" logical pixels");
    text(x + 14, y + 24, l.get(), DIM, 1);
    // input profile: a two-part switch
    label(y + rows[0] + 5, b"Input");
    let sx = x + settings_seg_x(a);
    chip(sx, y + rows[0], 128, 20, 4, SURFACE2);
    let on = if s.touch { 64 } else { 0 };
    chip(sx + on + 2, y + rows[0] + 2, 60, 16, 3, GOLD_DEEP);
    text(sx + 10, y + rows[0] + 6, b"Pointer", if s.touch { DIM } else { NIGHT }, 1);
    text(sx + 64 + 14, y + rows[0] + 6, b"Touch", if s.touch { NIGHT } else { DIM }, 1);

    head(y + 70, b"Desktop");
    label(y + rows[1] + 3, b"Monument wallpaper");
    switch(x + w - 50, y + rows[1], s.wallpaper);
    label(y + rows[2] + 3, b"24-hour clock");
    switch(x + w - 50, y + rows[2], s.clock24);

    head(y + 142, b"Network");
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(b"No network device");
    } else if ip == [0u8; 4] {
        l.s(b"Link up, no address yet");
    } else {
        rect(x + 14, y + 161, 5, 5, GREEN);
        l.s(b"   Connected  ");
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                l.s(b".");
            }
            l.u(*o as u64);
        }
    }
    label(y + 158, l.get());

    head(y + 182, b"System");
    let mut l = Line::new();
    l.u(crate::cores_online() as u64).s(b" core(s), ").u(crate::ram_mib()).s(b" MiB memory");
    label(y + 198, l.get());
    let mut l = Line::new();
    l.s(b"Booted ").u(crate::config::boots() as u64).s(b" time(s), up ").u(crate::sched::ticks() / 100).s(b" s");
    text(x + 14, y + 212, l.get(), DIM, 1);
    let _ = (w, a);
}

fn about_app(x: i32, y: i32, w: i32, h: i32) {
    rect(x, y, w, h, 0x0C_09_07);
    let mh = (h * 38 / 100).max(40);
    monument(x + w / 2, y + 16, mh, GOLD_PALE, 0x9A_67_18);
    let mut yy = y + 16 + mh + 14;
    let t = b"AfriSovera OS";
    text(x + (w - text_w(t, 2)) / 2, yy, t, GOLD, 2);
    yy += 22;
    let sub = b"Own your device - own your future";
    text(x + (w - text_w(sub, 1)) / 2, yy, sub, GOLD_DEEP, 1);
    yy += 18;
    rect(x + 24, yy, w - 48, 1, LINE);
    yy += 10;
    let arch: &[u8] = if cfg!(target_arch = "aarch64") { b"ARM64 (phone)" } else { b"x86-64 (PC)" };
    let mut rows: [(&[u8], Line); 4] = [(b"Kernel", Line::new()), (b"Desktop", Line::new()), (b"Device", Line::new()), (b"Memory", Line::new())];
    rows[0].1.s(b"Mansa microkernel, Rust");
    rows[1].1.s(b"Tonga, Sovereign UI");
    rows[2].1.s(arch).s(b", ").u(crate::cores_online() as u64).s(b" core(s)");
    rows[3].1.u((crate::frame::total_frames() * 4 / 1024) as u64).s(b" MiB, up ").u(crate::sched::ticks() / 100).s(b" s");
    for (k, v) in rows.iter() {
        if yy + 10 > y + h {
            break;
        }
        text(x + 30, yy, k, DIM, 1);
        text(x + 100, yy, v.get(), TEXT, 1);
        yy += 14;
    }
}

/// The Circle (the launcher): an octagon with the monument at its centre and the apps
/// standing in a ring round it; under it, this device and how to close (section 6).
fn launcher() {
    let s = st();
    let (lx, ly, lw, lh) = launcher_box();
    let k = lw * 29 / 100; // a regular octagon cuts about 29% off each corner
    cut(lx + 4, ly + 4, lw, lh, k, k, k, k, SHADOW);
    cut(lx, ly, lw, lh, k, k, k, k, 0x16_11_0E);
    cut_frame(lx, ly, lw, lh, k, k, k, k, GOLD_DARK);
    cut_frame(lx + 3, ly + 3, lw - 6, lh - 6, k - 1, k - 1, k - 1, k - 1, LINE);
    let (cx, cy) = (lx + lw / 2, ly + lh / 2 - 6);
    // the ring: sixteen beads, the apps stand on it
    let r = lw * 33 / 100;
    for (c, si) in DIRS {
        chip(cx + r * c / 1000 - 2, cy + r * si / 1000 - 2, 5, 5, 1, GOLD_DARK);
    }
    let mh = lh * 16 / 100;
    monument(cx, cy - mh / 2 - 6, mh, GOLD_PALE, GOLD_DEEP);
    let word = b"AfriSovera";
    text(cx - text_w(word, 1) / 2, cy + mh / 2 + 6, word, GOLD, 1);
    for i in 0..NAPPS {
        let (tx, ty, ts) = launcher_tile(i);
        icon(Some(APPS[i]), tx, ty, ts);
        let n = APPS[i].name().as_bytes();
        text(tx + (ts - text_w(n, 1)) / 2, ty + ts + 6, n, if s.wins[i].open { GOLD } else { TEXT }, 1);
    }
    // honest use of the space: what this device is doing now (review D2)
    let (ix, iy, iw, ih) = circle_info_box();
    chip(ix, iy, iw, ih, 6, 0x16_11_0E);
    cut_frame(ix, iy, iw, ih, 6, 6, 6, 6, LINE);
    let ip = crate::virtio::dhcp_ip();
    let mut l = Line::new();
    if !crate::virtio::net_present() {
        l.s(b"No network");
    } else if ip == [0u8; 4] {
        l.s(b"Link up, no address");
    } else {
        for (k, o) in ip.iter().enumerate() {
            if k > 0 {
                l.s(b".");
            }
            l.u(*o as u64);
        }
    }
    l.s(b",  ").u(crate::ram_mib()).s(b" MiB,  ");
    l.u((0..crate::vfs::MAX_FILES).filter(|&i| crate::vfs::is_used(i)).count() as u64).s(b" files");
    let x0 = text(ix + 14, iy + 10, b"This device", GOLD_DEEP, 1);
    text(x0 + 10, iy + 10, l.get(), TEXT, 1);
    let mut l = Line::new();
    l.s(b"Up ").u(crate::sched::ticks() / 100).s(b" s.  Super or Esc closes the Circle.");
    text(ix + 14, iy + 26, l.get(), DIM, 1);
}

fn notes_panel() {
    let s = st();
    let (nx, ny, nw, nh) = notes_box();
    cut(nx + 4, ny + 4, nw, nh, CUT, 0, CUT, 0, SHADOW);
    cut(nx, ny, nw, nh, CUT, 0, CUT, 0, 0x16_11_0E);
    cut_frame(nx, ny, nw, nh, CUT, 0, CUT, 0, GOLD_DARK);
    text(nx + 14, ny + 10, b"Notifications", GOLD, 1);
    let total = s.nnotes.min(NNOTE);
    if total == 0 {
        text(nx + 14, ny + 32, b"Nothing new", DIM, 1);
    }
    for k in 0..total {
        let i = (s.nnotes - 1 - k) % NNOTE;
        let yy = ny + 28 + k as i32 * 18;
        chip(nx + 14, yy + 2, 4, 4, 1, GOLD_DEEP);
        text(nx + 24, yy, &s.notes[i][..s.note_len[i]], TEXT, 1);
    }
}

fn toast() {
    let s = st();
    if s.nnotes == 0 || s.notes_open || crate::sched::ticks() >= s.toast_until {
        return;
    }
    let i = (s.nnotes - 1) % NNOTE;
    let msg = &s.notes[i][..s.note_len[i]];
    let w = text_w(msg, 1) + 40;
    // pointer: the top right corner; touch: a snackbar at the foot, never over a lintel
    // (design review D2)
    let sw = spine_w();
    let (x, y) = if s.touch { (sw + (W() - sw - w) / 2, H() - 46) } else { (W() - w - 10, TOP + 10) };
    cut(x + 3, y + 3, w, 28, 6, 0, 6, 0, SHADOW);
    cut(x, y, w, 28, 6, 0, 6, 0, SURFACE2);
    cut_frame(x, y, w, 28, 6, 0, 6, 0, GOLD_DARK);
    chip(x + 12, y + 10, 8, 8, 2, GOLD);
    text(x + 28, y + 10, msg, TEXT, 1);
}

/// Draw the whole desktop into the shared framebuffer.
fn draw() {
    let s = st();
    let t0 = crate::timer::cycles();
    wallpaper();
    for k in 0..s.nz {
        window(s.z[k]);
    }
    spine();
    toast();
    if s.notes_open {
        notes_panel();
    }
    if s.launcher {
        launcher();
    }
    s.frames += 1;
    let us = crate::timer::cycles().wrapping_sub(t0) * 1_000_000 / crate::timer::hz().max(1);
    // SAFETY: mainline only.
    unsafe { DRAW_US = (us, DRAW_US.1.max(us)) };
    crate::screen::mark();
}

/// The last and the worst whole-desktop draw, in microseconds (`desk perf`).
static mut DRAW_US: (u64, u64) = (0, 0);

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
    draw();
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
        draw();
    }
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
    if st().shown {
        draw();
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
            let (dl, dw) = unsafe { DRAW_US };
            let (cl, cw, n) = crate::screen::copy_perf();
            crate::kprintln!(
                "[desk] perf: {}x{} at UI scale {}x; draw {} us (worst {} us) over {} frame(s); copy to the screen {} us (worst {} us) over {} copies",
                W(), H(), crate::canvas::scale(), dl, dw, s.frames, cl, cw, n
            );
            return;
        }
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
            _ => crate::kprintln!("desk: keys are super alt-tab alt-f4 esc super-left super-right super-up super-down"),
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
            crate::kprintln!("      drag <x> <y> <dx> <dy>, mode pointer|touch, notify <text>, notes, perf, list");
            return;
        }
    }
    if verb != "list" {
        redraw_now();
    }
}
