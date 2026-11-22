//! The TPM rules that do not depend on the interface (architect's CRB verdict Q3, 14 Nov 2026), written once over a
//! `Link` that moves one command and its response: TIS (tpm_tis.rs) and CRB (tpm_crb.rs) each provide one. Pure: no
//! MMIO, no clock and no statics of its own; drbg-host's tests run these rules against both links.
//! - Startup only when the TPM answers "not started", once per boot, then the command once more (condition 2);
//! - TPM 1.2 is counted only when its `disable` and `deactivated` flags both say on;
//! - the start-up test (the first STARTUP_SAMPLES bytes) has a total ceiling, and a slow TPM says how slow.
#![allow(dead_code)]

use crate::tpm_wire::{self as w, Cmd, Flags, Ver, WireErr};

/// Architect's condition 5: the start-up test's total ceiling.
pub const STARTUP_CEILING_MS: u64 = 2000;

/// The TPM's registers (offsets in its 4 KiB locality-0 page) and a clock. The kernel's is MMIO and the TSC; the
/// host tests' is a script. Both TIS and CRB drive the chip through it.
pub trait Bus {
    fn r8(&mut self, off: u64) -> u8;
    fn r32(&mut self, off: u64) -> u32;
    fn w8(&mut self, off: u64, v: u8);
    fn w32(&mut self, off: u64, v: u32);
    /// A clock that only goes forward, in ticks.
    fn now(&mut self) -> u64;
    /// Ticks per second.
    fn hz(&self) -> u64;
}

pub fn ticks<B: Bus>(b: &B, ms: u64) -> u64 {
    b.hz().saturating_mul(ms) / 1000
}

/// Wait until `ok` or `ms` milliseconds have passed on the bus's clock; true when it came true.
pub fn wait<B: Bus>(b: &mut B, ms: u64, mut ok: impl FnMut(&mut B) -> bool) -> bool {
    let start = b.now();
    let limit = ticks(b, ms);
    loop {
        if ok(b) {
            return true;
        }
        if b.now().wrapping_sub(start) > limit {
            return ok(b);
        }
        core::hint::spin_loop();
    }
}

/// One command in, its whole response out (size-checked by the link), and a clock.
pub trait Link {
    fn transact(&mut self, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr>;
    /// A clock that only goes forward, in ticks.
    fn now(&mut self) -> u64;
    /// Ticks per second.
    fn hz(&self) -> u64;
}

/// What one boot remembers between calls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Session {
    pub ver: Ver,
    /// Set once this boot has sent Startup; it is never sent twice.
    pub started_by_us: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IoErr {
    NotReady,
    NoBurst,
    Expect,
    Timeout,
    Extra,
    /// CRB: the TPM set its fatal-error bit (CTRL_STS tpmSts).
    Fatal,
    /// CRB: the command does not fit the command buffer.
    TooBig,
    Wire(WireErr),
}

/// Send `c` and parse the answer's header; the body (a copy, bounded) goes to `body`, its length is returned.
pub fn call<L: Link>(l: &mut L, s: &Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    let mut cmd = [0u8; w::CMD_MAX];
    let n = w::build(s.ver, c, &mut cmd).map_err(IoErr::Wire)?;
    let mut rsp = [0u8; w::RSP_MAX];
    let len = l.transact(&cmd[..n], &mut rsp)?;
    let r = match w::parse(s.ver, &rsp[..len]) {
        Ok(x) => {
            body[..x.len()].copy_from_slice(x);
            Ok(x.len())
        }
        Err(e) => Err(IoErr::Wire(e)),
    };
    crate::drbg::wipe(&mut rsp);
    r
}

pub fn not_started(e: IoErr) -> bool {
    matches!(e, IoErr::Wire(WireErr::Rc(w::RC12_INVALID_POSTINIT)) | IoErr::Wire(WireErr::Rc(w::RC20_INITIALIZE)))
}

/// `c`, and if the TPM says it was never started: Startup once, then `c` once more (condition 2).
pub fn call_starting<L: Link>(l: &mut L, s: &mut Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    match call(l, s, c, body) {
        Err(e) if not_started(e) && !s.started_by_us => {
            s.started_by_us = true;
            call(l, s, Cmd::Startup, body)?;
            call(l, s, c, body)
        }
        r => r,
    }
}

/// One GetRandom of up to 32 bytes into `dst`; returns the bytes written.
pub fn get_random_once<L: Link>(l: &mut L, s: &mut Session, dst: &mut [u8]) -> Result<usize, IoErr> {
    let ask = dst.len().min(w::RANDOM_MAX);
    let mut body = [0u8; w::RSP_MAX];
    let n = call_starting(l, s, Cmd::GetRandom(ask as u16), &mut body)?;
    let r = match w::random_bytes(s.ver, &body[..n], ask) {
        Ok(x) => {
            dst[..x.len()].copy_from_slice(x);
            Ok(x.len())
        }
        Err(e) => Err(IoErr::Wire(e)),
    };
    crate::drbg::wipe(&mut body);
    r
}

/// TPM 1.2: the permanent `disable` and volatile `deactivated` flags; Ok(true) only when both say on.
pub fn tpm12_switched_on<L: Link>(l: &mut L, s: &mut Session) -> Result<bool, IoErr> {
    let mut body = [0u8; w::RSP_MAX];
    let n = call_starting(l, s, Cmd::GetCapFlags(Flags::Permanent), &mut body)?;
    let disabled = w::first_flag(&body[..n], Flags::Permanent).map_err(IoErr::Wire)?;
    let n = call(l, s, Cmd::GetCapFlags(Flags::Volatile), &mut body)?;
    let deactivated = w::first_flag(&body[..n], Flags::Volatile).map_err(IoErr::Wire)?;
    Ok(!disabled && !deactivated)
}

/// The boot probe, with locality held by the caller: for 1.2 the flags must say on, then one GetRandom of 32 bytes
/// must come back whole. Ok carries that call's time in microseconds; Err says why the TPM is not counted.
pub fn probe<L: Link>(l: &mut L, s: &mut Session) -> Result<u64, &'static str> {
    if s.ver == Ver::V12 {
        match tpm12_switched_on(l, s) {
            Ok(true) => {}
            Ok(false) => return Err("switched off in the firmware setup; switch it on there to use it"),
            Err(_) => return Err("its on/off flags could not be read, so it is not counted"),
        }
    }
    let mut x = [0u8; w::RANDOM_MAX];
    let t0 = l.now();
    let r = get_random_once(l, s, &mut x);
    let dt = l.now().wrapping_sub(t0);
    crate::drbg::wipe(&mut x);
    match r {
        Ok(n) if n == w::RANDOM_MAX => Ok(dt.saturating_mul(1_000_000) / l.hz().max(1)),
        Ok(_) => Err("GetRandom gave fewer bytes than asked for"),
        Err(IoErr::Wire(WireErr::Rc(_))) => Err("GetRandom returned an error code"),
        Err(IoErr::Wire(_)) => Err("GetRandom's answer was malformed"),
        Err(IoErr::Fatal) => Err("the TPM reported a fatal error"),
        Err(_) => Err("GetRandom timed out"),
    }
}

/// One batch, with locality held by the caller: GetRandom until `buf` is full or the TPM stops answering.
/// Returns the bytes written, the GetRandom calls made, and whether it stopped answering.
pub fn fill_batch<L: Link>(l: &mut L, s: &mut Session, buf: &mut [u8]) -> (usize, u64, bool) {
    let mut n = 0;
    let mut calls = 0u64;
    while n < buf.len() {
        calls += 1;
        match get_random_once(l, s, &mut buf[n..]) {
            Ok(k) => n += k,
            Err(_) => return (n, calls, true),
        }
    }
    (n, calls, false)
}

/// The start-up test's clock (condition 5): bytes, ticks and GetRandom calls spent until `samples` bytes have gone.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Budget {
    pub served: usize,
    pub spent: u64,
    pub calls: u64,
}

/// How slow a TPM that missed the start-up ceiling was (the "too slow" line prints these).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slow {
    pub total_ms: u64,
    pub calls: u64,
    pub us_per_call: u64,
}

impl Budget {
    /// Charge one batch of `n` bytes that took `dt` ticks over `calls` GetRandom calls. Only batches that start
    /// before `samples` bytes have been served count. Err when the total has passed `ceiling_ms`.
    pub fn charge(
        &mut self,
        n: usize,
        dt: u64,
        calls: u64,
        hz: u64,
        samples: usize,
        ceiling_ms: u64,
    ) -> Result<(), Slow> {
        if self.served >= samples {
            return Ok(());
        }
        self.served = self.served.saturating_add(n);
        self.spent = self.spent.saturating_add(dt);
        self.calls = self.calls.saturating_add(calls);
        if self.spent > hz.saturating_mul(ceiling_ms) / 1000 {
            let us = self.spent.saturating_mul(1_000_000) / hz.max(1);
            return Err(Slow { total_ms: us / 1000, calls: self.calls, us_per_call: us / self.calls.max(1) });
        }
        Ok(())
    }
}
