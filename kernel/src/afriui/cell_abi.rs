//! The cell window ABI (docs/design/afriui.md sections 24 and 25): the one copy of the event kinds, the packing,
//! the error codes and the serial close key, used by nia.rs (the five host calls) and desk.rs (the window and its
//! event queue). Change them here only.
#![allow(dead_code)] // nia.rs uses some of these from its own branch commits

/// The one handle `cell_open` gives: one cell window per app in stage 2.
pub const WIN: i32 = 1;

/// Event kinds, bits 28 to 30 of an event; bit 31 is always clear, so every event is a positive i32.
pub const EV_KEY: u32 = 1; // the key code with its modifier bits (section 24)
pub const EV_RESIZE: u32 = 2; // the new grid, packed as `grid` packs it
pub const EV_CLOSE_REQUEST: u32 = 3; // the close box, Alt+F4 or Ctrl+\ on the serial line
pub const EV_CLOSED: u32 = 4; // the window is gone ("Close anyway"); returned at once from then on
/// The payload: bits 0 to 27.
pub const PAYLOAD: u32 = (1 << 28) - 1;

/// Key modifier bits (stage 3 sets them; plain keys keep the serial line's values).
pub const SHIFT: u32 = 0x1000;
pub const CTRL: u32 = 0x2000;
pub const ALT: u32 = 0x4000;

/// Errors, negative as everywhere in Nia.
pub const ERR_BAD_WINDOW: i32 = -1; // not the handle `cell_open` gave
pub const ERR_NO_WINDOW: i32 = -2; // closed, or never opened
pub const ERR_ALREADY_OPEN: i32 = -3; // the app has its one cell window already
pub const ERR_NO_DESK: i32 = -4; // the desk is not showing, so there is nowhere to draw
pub const ERR_BAD_ARG: i32 = -5; // a negative argument, or a cursor shape outside 0 to 3

/// The serial line's close request (Ctrl+\); a second one at least FORCE_AFTER_MS after an unanswered request is
/// the "Close anyway" click.
pub const SERIAL_CLOSE: u8 = 0x1C;
/// How long a close request stays unanswered before "Close anyway" is offered.
pub const FORCE_AFTER_MS: u64 = 3000;
/// Event slots per cell window; when they are full the newest event is dropped and counted.
pub const QUEUE: usize = 32;
/// The longest a stuck app keeps the desk from drawing: `app_tick` runs the desk at most this often.
pub const TICK_MS: u64 = 10;

/// One event as the i32 `cell_event` returns.
pub const fn event(kind: u32, payload: u32) -> i32 {
    (((kind & 7) << 28) | (payload & PAYLOAD)) as i32
}

/// A grid size as `cell_size` returns it and a resize event carries it: cols << 14 | rows.
pub const fn grid(cols: usize, rows: usize) -> u32 {
    ((cols as u32 & 0x3FFF) << 14) | (rows as u32 & 0x3FFF)
}
