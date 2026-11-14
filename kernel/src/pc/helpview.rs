//! `help` split by word or by page on the PC (9 Nov 2026, HP ENVY field item F7, second
//! half: the shared `help` prints over 600 lines, which ran off the top of the laptop's
//! screen with nothing to bring them back).
//!
//! The list itself is the shared shell.rs's (so the ARM image does not change). On the PC,
//! `help <word>` and `help <page>` set a filter here, and the shared `help` then runs as
//! usual: every finished line it prints passes through `feed` (from uart.rs's `_print`,
//! with the print lock held), and only the lines asked for reach the console. The shell's
//! next prompt ("afrisovera> " at the start of a line) ends the filter with one summary
//! line. Plain `help` prints everything, as before, plus that summary line with the hint.
//!
//! While the filter is on, everything printed is filtered, the other core's prints too;
//! `help` runs in a few milliseconds, so at most a stray line of theirs could be missed.

use core::fmt::Write;
use core::ptr::{addr_of, addr_of_mut};

/// Lines on one page: the console shows 48, leaving room for the wrapped long lines, the
/// summary and the prompt.
const PER: usize = 36;
const PROMPT: &str = "afrisovera> ";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    All,
    Word,
    Page(usize),
}

struct State {
    mode: Mode,
    word: [u8; 32],
    wlen: usize,
    line: [u8; 512],
    llen: usize,
    lines: usize, // lines of the list seen (the "commands:" title not counted)
    shown: usize,
    titled: bool,
}

static mut ST: State = State {
    mode: Mode::Off,
    word: [0; 32],
    wlen: 0,
    line: [0; 512],
    llen: 0,
    lines: 0,
    shown: 0,
    titled: false,
};

fn st() -> &'static mut State {
    // SAFETY: only touched from `start` (the shell, before `help` prints) and `feed` (under
    // the print lock), never at the same time.
    unsafe { &mut *addr_of_mut!(ST) }
}

/// True while a `help` filter is on (uart.rs routes prints to `feed` then).
pub fn on() -> bool {
    // SAFETY: a plain read of a Copy field; see `st`.
    unsafe { (*addr_of!(ST)).mode != Mode::Off }
}

/// From shell_pc.rs's intercept of `help`, before the shared `help` runs. `rest` is what
/// followed `help`: nothing (everything, then the hint), a page number, or a word.
pub fn start(rest: &str) {
    let s = st();
    s.llen = 0;
    s.lines = 0;
    s.shown = 0;
    s.titled = false;
    s.mode = if rest.is_empty() {
        Mode::All
    } else if let Ok(p) = rest.parse::<usize>() {
        Mode::Page(p)
    } else {
        let b = rest.as_bytes();
        s.wlen = b.len().min(s.word.len());
        s.word[..s.wlen].copy_from_slice(&b[..s.wlen]);
        Mode::Word
    };
}

fn has_word(line: &[u8], word: &[u8]) -> bool {
    word.is_empty()
        || line.len() >= word.len()
            && line.windows(word.len()).any(|w| w.eq_ignore_ascii_case(word))
}

/// One finished line of `help` (without its newline): shown or held back.
fn line_done(s: &mut State) {
    let n = s.llen.min(s.line.len());
    let text = core::str::from_utf8(&s.line[..n]).unwrap_or("");
    // the shared list says how the ARM kernel does these (PSCI); on the PC (14 Nov 2026)
    let text = if text.trim_start().starts_with("reboot ") && text.contains("PSCI") {
        "  reboot, restart  save the files, then restart the PC (the firmware, then ACPI, then the chipset)"
    } else if text.trim_start().starts_with("poweroff ") && text.contains("PSCI") {
        "  poweroff, shutdown  save the files, then switch the PC off (the firmware, then ACPI S5)"
    } else {
        text
    };
    let title = !s.titled && text.trim() == "commands:";
    if title {
        s.titled = true;
    } else {
        s.lines += 1;
    }
    let show = match s.mode {
        Mode::Off | Mode::All => true,
        Mode::Word => title || has_word(text.as_bytes(), &s.word[..s.wlen]),
        Mode::Page(p) => title || (p >= 1 && s.lines > (p - 1) * PER && s.lines <= p * PER),
    };
    if show {
        if !title && s.mode != Mode::All {
            s.shown += 1;
        }
        crate::uart::tee_str(text);
        crate::uart::tee_str("\n");
    }
    s.llen = 0;
}

/// The summary line, then the filter off (the prompt that ended it follows).
fn finish(s: &mut State) {
    let pages = (s.lines + PER - 1) / PER;
    let word = core::str::from_utf8(&s.word[..s.wlen]).unwrap_or("?");
    let mode = s.mode;
    s.mode = Mode::Off;
    match mode {
        Mode::Off => {}
        Mode::All => crate::uart::tee_fmt(format_args!(
            "help: {} commands above (Shift+PgUp scrolls back); help <word> shows only the lines with that word, help 1 to help {} a page at a time\n",
            s.lines, pages
        )),
        Mode::Word if s.shown == 0 => crate::uart::tee_fmt(format_args!(
            "help: no line of the {} has '{}'; help 1 to help {} shows them a page at a time\n",
            s.lines, word, pages
        )),
        Mode::Word => crate::uart::tee_fmt(format_args!(
            "help: {} of {} lines have '{}'; help 1 to help {} shows them all a page at a time\n",
            s.shown, s.lines, word, pages
        )),
        Mode::Page(p) if p == 0 || p > pages => crate::uart::tee_fmt(format_args!(
            "help: there is no page {}; the {} lines are pages 1 to {}\n",
            p, s.lines, pages
        )),
        Mode::Page(p) if p == pages => crate::uart::tee_fmt(format_args!(
            "help: page {} of {} (lines {}-{} of {}), the last\n",
            p, pages, (p - 1) * PER + 1, s.lines, s.lines
        )),
        Mode::Page(p) => crate::uart::tee_fmt(format_args!(
            "help: page {} of {} (lines {}-{} of {}); help {} for the next\n",
            p, pages, (p - 1) * PER + 1, p * PER, s.lines, p + 1
        )),
    }
}

struct Feed;
impl Write for Feed {
    fn write_str(&mut self, mut t: &str) -> core::fmt::Result {
        let s = st();
        while !t.is_empty() {
            if s.mode == Mode::Off {
                crate::uart::tee_str(t);
                return Ok(());
            }
            if s.llen == 0 && t.starts_with(PROMPT) {
                finish(s);
                continue; // the prompt itself goes out through the line above
            }
            match t.find('\n') {
                Some(i) => {
                    push(s, &t.as_bytes()[..i]);
                    line_done(s);
                    t = &t[i + 1..];
                }
                None => {
                    push(s, t.as_bytes());
                    t = "";
                }
            }
        }
        Ok(())
    }
}

fn push(s: &mut State, b: &[u8]) {
    let room = s.line.len() - s.llen.min(s.line.len());
    let k = b.len().min(room);
    let at = s.llen.min(s.line.len());
    s.line[at..at + k].copy_from_slice(&b[..k]);
    s.llen += b.len(); // a longer line is cut at 512 bytes, still counted once
}

/// From uart.rs's `_print` while the filter is on, with the print lock held.
pub fn feed(args: core::fmt::Arguments) {
    let _ = Feed.write_fmt(args);
}
