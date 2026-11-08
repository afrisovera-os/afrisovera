//! `write!` into a fixed buffer — the fallible counterpart to `format!`.
//!
//! 2.736. The heap's largest customer now refuses by name and the refusal reaches a
//! caller (2.735). Its *small* customers are the ones left: every `format!` in the
//! kernel allocates infallibly, so a null from a full arena goes to Rust's default
//! allocation error handler and out as `panic!("memory allocation of N bytes failed")` —
//! the machine dies for a string it was building. `#[alloc_error_handler]` would let the
//! kernel intercept that, but it is unstable and this toolchain is pinned to stable, so
//! the fix is to not need the allocator for text at all.
//!
//! This module is that: a caller-owned `[u8; N]`, a `core::fmt::Write` that fills it, and
//! a bound it states by name when the text does not fit. Two properties matter and both
//! are checked in [`crate::config::selftest`]:
//!
//! * **It never writes past the buffer, and never splits a character.** `write_str`
//!   copies the longest prefix that fits AND ends on a char boundary, so the accumulated
//!   bytes are valid UTF-8 at every point, not just at the end. A buffer that filled
//!   mid-`é` would otherwise hand the console half a character.
//! * **Over-long text is refused, not silently shortened.** A truncated config file is
//!   worse than no config file: it parses, and it lies. [`render`] is all-or-nothing;
//!   [`render_lossy`] is the deliberate exception, for the panic path, where a partial
//!   message still beats silence.

use core::fmt::{self, Write};

/// Refused: the rendered text is longer than the buffer it was given.
pub const BOUND_BUF: &str = "the text is longer than the fixed buffer it renders into";

/// A fixed byte buffer that `write!` can format into, counting what did not fit.
pub struct FmtBuf<'a> {
    buf: &'a mut [u8],
    len: usize,
    dropped: usize,
}

impl<'a> FmtBuf<'a> {
    /// Wrap a caller-owned buffer. Nothing here allocates — that is the whole point.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0, dropped: 0 }
    }

    /// The whole text, or the bound — all-or-nothing.
    ///
    /// Valid UTF-8 by construction (see the module note); the check is kept anyway,
    /// because "by construction" is a claim about this file and the caller is the one
    /// who pays if it stops being true.
    pub fn finish(self) -> Result<&'a str, &'static str> {
        if self.dropped > 0 {
            return Err(BOUND_BUF);
        }
        match core::str::from_utf8(&self.buf[..self.len]) {
            Ok(s) => Ok(s),
            Err(_) => Err(BOUND_BUF),
        }
    }

    /// What fits, and how many bytes did not — the deliberate exception, for the panic
    /// path. Total: it cannot fail, because it is the reporter of last resort.
    pub fn finish_lossy(self) -> (&'a str, usize) {
        let text = match core::str::from_utf8(&self.buf[..self.len]) {
            Ok(s) => s,
            Err(_) => "",
        };
        (text, self.dropped)
    }
}

impl Write for FmtBuf<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let room = self.buf.len() - self.len;
        // The longest prefix that fits and does not cut a character in half. `room` can
        // land inside a multi-byte sequence; walking back to a boundary drops that whole
        // character rather than writing its first byte.
        let mut n = if room < s.len() { room } else { s.len() };
        while n > 0 && !s.is_char_boundary(n) {
            n -= 1;
        }
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        if n < s.len() {
            self.dropped += s.len() - n;
            // Reported so `write!`/`render` short-circuit; the bytes that fit are kept
            // regardless, which is what makes `render_lossy` possible.
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Render `args` into `buf`, **all-or-nothing**: text that does not fit is refused by
/// name instead of written short.
///
/// The replacement for `format!` on any path where a truncated result would be taken for
/// a whole one — `config::persist` above all, where the file is re-read and parsed on the
/// next boot.
pub fn render<'a>(buf: &'a mut [u8], args: fmt::Arguments) -> Result<&'a str, &'static str> {
    let mut f = FmtBuf::new(buf);
    // The error is redundant with `dropped` today and is checked anyway: `write_fmt` can
    // also fail inside a `Display` impl, and that text is no more complete than a
    // truncated one.
    if f.write_fmt(args).is_err() {
        return Err(BOUND_BUF);
    }
    f.finish()
}

/// Render as much of `args` as fits, and say how much did not.
///
/// For the panic path only. A panic message is the last thing the operator gets; half of
/// it names the failure, and refusing to print it because it was three bytes too long
/// would trade the whole diagnosis for the buffer's dignity.
pub fn render_lossy<'a>(buf: &'a mut [u8], args: fmt::Arguments) -> (&'a str, usize) {
    let mut f = FmtBuf::new(buf);
    let _ = f.write_fmt(args); // partial output is the point; the error is the truncation
    f.finish_lossy()
}
