//! Terminal key sequences to app key codes (A4, NIA_A4_PLAN.md and the architect's layout of 14 Nov 2026).
//!
//! The serial line and, while a Nia app runs, the PC keyboards deliver special keys as the bytes an xterm sends
//! (`ESC [ A`, `ESC O P`, `ESC [ 15 ~`, `ESC [ 1 ; 5 C`). This turns them into one key code each, in the cell ABI's
//! payload (cell_abi.rs): bits 0 to 20 a Unicode code point or a special key at KEY_BASE + n, bit 21 SHIFT, 22 CTRL,
//! 23 ALT. It is used only on the app's side (Nia's key queue and the desk's cell events); the shell, the desk's own
//! keys and the history see the bytes as before.
//!
//! - A plain byte is given back as itself (plain bytes stay as they are in A4), Ctrl+letter included (1 to 26).
//! - ESC then `[` or `O` starts a sequence; ESC then another printable byte within ESC_MS is ALT plus that byte; ESC
//!   alone for ESC_MS is Esc (27).
//! - A sequence this does not know is given back as its bytes, in order, so nothing is lost.
//!
//! Pure: no `crate::` beyond the ABI's constants, and the time comes from the caller, so fuzz-host compiles it
//! unchanged (host test keyseq.rs and the fuzz target key_seq).

use crate::desk::cell_abi::{ALT, CTRL, KEY_BASE, SHIFT};

/// How long ESC waits for the rest of a sequence. At 115,200 baud a sequence's bytes are well under 1 ms apart.
pub const ESC_MS: u64 = 30;
const ESC: u8 = 0x1B;
const MAXLEN: usize = 8;

/// The special keys' numbers (n in KEY_BASE + n).
pub const UP: u32 = 0;
pub const DOWN: u32 = 1;
pub const LEFT: u32 = 2;
pub const RIGHT: u32 = 3;
pub const HOME: u32 = 4;
pub const END: u32 = 5;
pub const PGUP: u32 = 6;
pub const PGDN: u32 = 7;
pub const INS: u32 = 8;
pub const DEL: u32 = 9;
pub const F1: u32 = 0x10; // to F12 = 0x1B

/// What one byte (or the clock) gives: nothing yet, one key, or bytes given back as they came.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Out {
    None,
    Key(u32),
    /// Bytes given back one by one as keys (an unknown sequence, or a lone ESC followed by a control byte).
    Bytes([u8; MAXLEN], usize),
}

/// The decoder's state: the bytes of an unfinished sequence and when its ESC came.
pub struct Seq {
    buf: [u8; MAXLEN],
    n: usize,
    t0: u64,
}

impl Seq {
    pub const fn new() -> Self {
        Seq { buf: [0; MAXLEN], n: 0, t0: 0 }
    }

    /// Whether a sequence is waiting for more bytes (the caller then calls `tick` as time passes).
    pub fn pending(&self) -> bool {
        self.n > 0
    }

    /// Forget any unfinished sequence (a new app starts).
    pub fn reset(&mut self) {
        self.n = 0;
    }

    /// The clock moved: a sequence that has waited ESC_MS ends. A lone ESC is Esc (27); more bytes come back as
    /// they are.
    pub fn tick(&mut self, now_ms: u64) -> Out {
        if self.n == 0 || now_ms.saturating_sub(self.t0) < ESC_MS {
            return Out::None;
        }
        self.flush()
    }

    fn flush(&mut self) -> Out {
        let n = core::mem::replace(&mut self.n, 0);
        if n == 1 {
            Out::Key(ESC as u32)
        } else {
            Out::Bytes(self.buf, n)
        }
    }

    /// One byte in. The caller first calls `tick` with the same time, so a timed-out ESC comes out before it.
    pub fn feed(&mut self, b: u8, now_ms: u64) -> Out {
        if self.n == 0 {
            if b == ESC {
                self.buf[0] = ESC;
                self.n = 1;
                self.t0 = now_ms;
                return Out::None;
            }
            return Out::Key(b as u32);
        }
        if self.n == 1 {
            return match b {
                b'[' | b'O' => {
                    self.buf[1] = b;
                    self.n = 2;
                    Out::None
                }
                ESC => Out::Key(ESC as u32), // the first ESC alone; the second starts again (t0 kept fresh below)
                0x20..=0x7E => {
                    self.n = 0;
                    Out::Key(ALT | b as u32) // Alt+character
                }
                _ => {
                    self.buf[1] = b;
                    self.n = 2;
                    self.flush()
                }
            }
            .restart_if_esc(self, b, now_ms);
        }
        if self.n >= MAXLEN {
            return self.flush_with(b);
        }
        self.buf[self.n] = b;
        self.n += 1;
        // a parameter byte: wait for more
        if self.buf[1] == b'[' && (b.is_ascii_digit() || b == b';') {
            return Out::None;
        }
        match self.decode() {
            Some(k) => {
                self.n = 0;
                Out::Key(k)
            }
            None => self.flush(),
        }
    }

    /// The byte that overflowed the buffer, and the buffer, given back.
    fn flush_with(&mut self, b: u8) -> Out {
        let out = self.flush();
        // the overflowing byte is rare (a sequence longer than MAXLEN); keep it as the start of what comes next
        if b == ESC {
            self.buf[0] = ESC;
            self.n = 1;
        }
        out
    }

    /// The finished sequence in buf[..n] as a key, or None when this does not know it.
    fn decode(&self) -> Option<u32> {
        let s = &self.buf[..self.n];
        let last = s[s.len() - 1];
        if s[1] == b'O' {
            // SS3: ESC O P..S are F1 to F4; ESC O A..D, H, F as the cursor keys
            if s.len() != 3 {
                return None;
            }
            return match last {
                b'P'..=b'S' => Some(KEY_BASE + F1 + (last - b'P') as u32),
                _ => letter_key(last).map(|n| KEY_BASE + n),
            };
        }
        // CSI: ESC [ params final
        let params = &s[2..s.len() - 1];
        let (a, m) = parse_params(params)?;
        let mods = mod_bits(m)?;
        let n = match last {
            b'~' => tilde_key(a?)?,
            _ => {
                // a letter key takes no number but 1 (ESC [ 1 ; 5 C); P to S are F1 to F4 in xterm's modifier form
                // (ESC [ 1 ; 3 S is Alt+F4)
                if a.is_some_and(|v| v != 1) {
                    return None;
                }
                match last {
                    b'P'..=b'S' => F1 + (last - b'P') as u32,
                    _ => letter_key(last)?,
                }
            }
        };
        Some((KEY_BASE + n) | mods)
    }
}

trait Restart {
    fn restart_if_esc(self, s: &mut Seq, b: u8, now_ms: u64) -> Out;
}

impl Restart for Out {
    /// After a lone ESC is given out because another ESC came, that second ESC starts a new sequence.
    fn restart_if_esc(self, s: &mut Seq, b: u8, now_ms: u64) -> Out {
        if b == ESC && self == Out::Key(ESC as u32) {
            s.buf[0] = ESC;
            s.n = 1;
            s.t0 = now_ms;
        }
        self
    }
}

/// The cursor keys' final letters.
fn letter_key(c: u8) -> Option<u32> {
    Some(match c {
        b'A' => UP,
        b'B' => DOWN,
        b'C' => RIGHT,
        b'D' => LEFT,
        b'H' => HOME,
        b'F' => END,
        _ => return None,
    })
}

/// ESC [ n ~ keys (VT and xterm numbering).
fn tilde_key(n: u32) -> Option<u32> {
    Some(match n {
        1 | 7 => HOME,
        2 => INS,
        3 => DEL,
        4 | 8 => END,
        5 => PGUP,
        6 => PGDN,
        11..=15 => F1 + (n - 11),
        17..=21 => F1 + 5 + (n - 17),
        23 | 24 => F1 + 10 + (n - 23),
        _ => return None,
    })
}

/// "a" or "a;m" or "" as (the number, the modifier number); None for anything else.
fn parse_params(p: &[u8]) -> Option<(Option<u32>, u32)> {
    let mut parts = p.split(|&c| c == b';');
    let first = parts.next().unwrap_or(&[]);
    let a = if first.is_empty() { None } else { Some(number(first)?) };
    let m = match parts.next() {
        Some(x) => number(x)?,
        None => 1,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((a, m))
}

fn number(d: &[u8]) -> Option<u32> {
    if d.is_empty() || d.len() > 3 {
        return None;
    }
    let mut v = 0u32;
    for &c in d {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    Some(v)
}

/// xterm's modifier number m (1 + bits: 1 Shift, 2 Alt, 4 Ctrl) as the ABI's bits.
fn mod_bits(m: u32) -> Option<u32> {
    if !(1..=8).contains(&m) {
        return None;
    }
    let b = m - 1;
    let mut bits = 0;
    if b & 1 != 0 {
        bits |= SHIFT;
    }
    if b & 2 != 0 {
        bits |= ALT;
    }
    if b & 4 != 0 {
        bits |= CTRL;
    }
    Some(bits)
}

/// For the PC keyboards while a Nia app runs (A4): inject the bytes for special key `n` with `mods` into the
/// console's input, as a terminal would send them. Alt+F4 is never injected: it stays the desk's close. Returns false
/// when nothing was injected.
pub fn inject_special(n: u32, mods: u32, inject: impl Fn(u8)) -> bool {
    if n == F1 + 3 && mods & ALT != 0 {
        return false;
    }
    let mut b = [0u8; 12];
    let len = encode(n, mods, &mut b);
    b[..len].iter().for_each(|&c| inject(c));
    len > 0
}

/// For the PC keyboards while a Nia app runs (A4): Ctrl+letter as its control byte (1 to 26, so Ctrl+C is 3 and stops
/// the app), Alt+character as ESC then the character (keyseq reads that as ALT plus it).
pub fn inject_char(c: u8, ctrl: bool, alt: bool, inject: impl Fn(u8)) -> bool {
    if ctrl && c.is_ascii_alphabetic() {
        inject(c.to_ascii_lowercase() & 0x1F);
        return true;
    }
    if alt && (0x20..0x7F).contains(&c) {
        inject(ESC);
        inject(c);
        return true;
    }
    false
}

/// The bytes an xterm sends for special key `n` (UP to F1 + 11) with the modifier bits `mods` (SHIFT, CTRL, ALT),
/// for the PC keyboards to inject while an app runs (A4), so serial and keyboard meet in `Seq` and give one code.
/// Cursor keys and Home/End as `ESC [ X` or `ESC [ 1 ; m X`; the rest as `ESC [ k ~` or `ESC [ k ; m ~`. Returns the
/// length written into `out`, or 0 for a number that is not a special key.
pub fn encode(n: u32, mods: u32, out: &mut [u8; 12]) -> usize {
    let mut m = 1u32;
    if mods & SHIFT != 0 {
        m += 1;
    }
    if mods & ALT != 0 {
        m += 2;
    }
    if mods & CTRL != 0 {
        m += 4;
    }
    let (num, fin): (u32, u8) = match n {
        UP => (1, b'A'),
        DOWN => (1, b'B'),
        RIGHT => (1, b'C'),
        LEFT => (1, b'D'),
        HOME => (1, b'H'),
        END => (1, b'F'),
        INS => (2, b'~'),
        DEL => (3, b'~'),
        PGUP => (5, b'~'),
        PGDN => (6, b'~'),
        f if (F1..F1 + 12).contains(&f) => ([11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 23, 24][(f - F1) as usize], b'~'),
        _ => return 0,
    };
    let mut i = 0;
    let mut put = |b: u8| {
        out[i] = b;
        i += 1;
    };
    put(ESC);
    put(b'[');
    if fin == b'~' || m > 1 {
        if num >= 10 {
            put(b'0' + (num / 10) as u8);
        }
        put(b'0' + (num % 10) as u8);
    }
    if m > 1 {
        put(b';');
        put(b'0' + m as u8);
    }
    put(fin);
    i
}
