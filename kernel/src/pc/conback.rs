//! Console scrollback on the PC (9 Nov 2026, HP ENVY field item F7: the long `help`
//! output scrolled off the top and nothing could bring it back).
//!
//! The text console is drawn by the shared `framebuffer.rs`, which keeps no text: when it
//! scrolls, the top row's pixels are gone. So that the ARM image does not change, this
//! PC-only file keeps its own copy of every character the console is sent (`feed`, from
//! uart.rs's Tee, wrapped the way `framebuffer::con_putc` wraps it), the last KEEP lines.
//! Shift+PgUp / Shift+PgDn (ps2.rs, xhci.rs) call `page`: the first PgUp saves the live
//! console panel's pixels and the panel is redrawn from the kept text, half a screen
//! further back each time; PgDn walks forward, and reaching the bottom puts the saved
//! pixels back. Any new output while scrolled back also puts them back first (`live`), so
//! the shared console carries on exactly where it was. The desktop is left alone.
//! The mouse wheel (9 Nov 2026, F7 last part; ps2.rs's IntelliMouse mode, so a touchpad
//! that offers it scrolls too) calls `wheel`: three lines a notch, the same way back.
//!
//! All of it runs with the print lock held (uart.rs), so the two cores never draw at once.

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicU64, Ordering};

// The console panel's geometry and colours: copies of framebuffer.rs's private consts.
const CON_X: usize = 12;
const CON_Y: usize = 86;
const CH: usize = 8;
const COLS: usize = (crate::framebuffer::WIDTH - 2 * CON_X) / CH; // 77
const ROWS: usize = (crate::framebuffer::HEIGHT - CON_Y - 10) / CH; // 48
const CON_BG: u32 = 0x08_0B_12;
const TEXT: u32 = 0xD8_E0_E8;

/// Lines kept (each COLS bytes): a few hundred, as the field report asks.
const KEEP: usize = 512;
/// One Shift+PgUp/PgDn moves half a screen.
const STEP: usize = ROWS / 2;
/// One wheel notch (or a touchpad's scroll step, sent as one) moves three lines, as most
/// desktops do.
const WHEEL_LINES: usize = 3;

static mut BUF: [[u8; COLS]; KEEP] = [[0; COLS]; KEEP];
static mut LINES: usize = 1; // lines ever started; the one being written is LINES - 1
static mut COL: usize = 0;
static mut UP: usize = 0; // 0: the live console; k: the view ends k lines above it
static mut SNAP: [u32; COLS * CH * ROWS * CH] = [0; COLS * CH * ROWS * CH];

static PAGES: AtomicU64 = AtomicU64::new(0); // Shift+PgUp/PgDn presses that moved the view
static RETURNS: AtomicU64 = AtomicU64::new(0); // times the live console was put back
static WHEELS: AtomicU64 = AtomicU64::new(0); // wheel movements that moved the view

unsafe fn newline() {
    LINES += 1;
    COL = 0;
    (*addr_of_mut!(BUF))[(LINES - 1) % KEEP] = [0; COLS];
}

/// Every string the console is sent. Caller holds the print lock.
pub fn feed(s: &str) {
    // SAFETY: the print lock (uart.rs) serialises every caller.
    unsafe {
        for &c in s.as_bytes() {
            match c {
                b'\n' => newline(),
                b'\r' => {}
                0x08 => COL = COL.saturating_sub(1), // backspace steps back, as the console does (14 Nov)
                _ => {
                    if COL >= COLS {
                        newline();
                    }
                    (*addr_of_mut!(BUF))[(LINES - 1) % KEEP][COL] = c;
                    COL += 1;
                }
            }
        }
    }
}

/// Put the live console back if it is scrolled back (new output is about to be drawn).
/// Caller holds the print lock.
pub fn live() {
    // SAFETY: the print lock serialises every caller.
    unsafe {
        if UP > 0 {
            restore();
        }
    }
}

unsafe fn restore() {
    crate::framebuffer::blit(CON_X, CON_Y, COLS * CH, ROWS * CH, addr_of!(SNAP) as *const u32, COLS * CH);
    UP = 0;
    RETURNS.fetch_add(1, Ordering::Relaxed);
    crate::screen::mark();
}

unsafe fn save() {
    let fb = crate::framebuffer::pixels();
    let snap = addr_of_mut!(SNAP) as *mut u32;
    for y in 0..ROWS * CH {
        for x in 0..COLS * CH {
            snap.add(y * COLS * CH + x).write(*fb.add((CON_Y + y) * crate::framebuffer::WIDTH + CON_X + x));
        }
    }
}

/// The panel shows the kept lines ending UP lines above the line being written.
unsafe fn draw() {
    let kept = LINES.min(KEEP);
    let bottom = LINES - 1 - UP;
    let first = (bottom + 1).saturating_sub(ROWS);
    crate::framebuffer::fill_rect(CON_X, CON_Y, COLS * CH, ROWS * CH, CON_BG);
    for (r, l) in (first..=bottom).enumerate() {
        if l + kept < LINES {
            continue; // older than what is kept
        }
        let line = &(*addr_of!(BUF))[l % KEEP];
        for (c, &b) in line.iter().enumerate() {
            if b != 0 && b != b' ' {
                crate::framebuffer::draw_char(CON_X + c * CH, CON_Y + r * CH, b, TEXT, 1);
            }
        }
    }
    crate::screen::mark();
}

/// Shift+PgUp (`up`) or Shift+PgDn. Caller holds the print lock with interrupts off.
pub fn page(up: bool) {
    if scroll(up, STEP) {
        PAGES.fetch_add(1, Ordering::Relaxed);
    }
}

/// Wheel notches (ps2.rs): `notches` < 0 rolls away from the user, which scrolls back.
/// WHEEL_LINES lines a notch. Caller holds the print lock with interrupts off.
pub fn wheel(notches: i32) {
    if notches != 0 && scroll(notches < 0, notches.unsigned_abs() as usize * WHEEL_LINES) {
        WHEELS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Move the view `n` lines back (`up`) or forward; false when it could not move (already at
/// the top or the bottom, or the desktop is shown).
fn scroll(up: bool, n: usize) -> bool {
    if crate::screen::desktop() || crate::screen::wm_view() {
        return false; // the desktop has its own windows; the console is not on the screen
    }
    // SAFETY: the print lock serialises every caller.
    unsafe {
        let kept = LINES.min(KEEP);
        let most = kept.saturating_sub(ROWS);
        let to = if up { (UP + n).min(most) } else { UP.saturating_sub(n) };
        if to == UP {
            return false;
        }
        if to == 0 {
            restore();
            crate::uart::com1_fmt(format_args!("\r\n[con] scrollback: back to the live console\r\n"));
            return true;
        }
        if UP == 0 {
            save();
        }
        UP = to;
        draw();
        let top = (LINES - 1 - UP + 1).saturating_sub(ROWS);
        let (line, up) = (&(*addr_of!(BUF))[top % KEEP], UP);
        let len = line.iter().rposition(|&b| b != 0 && b != b' ').map_or(0, |i| i + 1);
        crate::uart::com1_fmt(format_args!(
            "\r\n[con] scrollback: {} lines up of {} kept; top line: {}\r\n",
            up,
            kept,
            core::str::from_utf8(&line[..len]).unwrap_or("?")
        ));
        true
    }
}

/// The `scrollback` command.
pub fn info() {
    // SAFETY: a plain read; a torn value only misprints this one line.
    let lines = unsafe { LINES };
    crate::kprintln!(
        "scrollback: {} console line(s) written, the last {} kept; Shift+PgUp/PgDn moved the view {} time(s), {} back to live",
        lines,
        lines.min(KEEP),
        PAGES.load(Ordering::Relaxed),
        RETURNS.load(Ordering::Relaxed)
    );
    crate::kprintln!(
        "scrollback: the mouse wheel moved the view {} time(s), {} lines a notch",
        WHEELS.load(Ordering::Relaxed),
        WHEEL_LINES
    );
}
