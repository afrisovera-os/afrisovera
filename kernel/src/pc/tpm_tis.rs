//! The TPM's TIS (FIFO) protocol and its rules, generic over the register bus (architect's TPM merge follow-up 2,
//! 13 Nov 2026). Pure: no MMIO, no clock and no statics of its own. tpm.rs supplies the real bus (MMIO at 0xFED40000
//! and the TSC); drbg-host/tests/tpm_tis.rs supplies a scripted TPM, so the rules below are tested on the host:
//! - one command at a time, and every wait has a clock ceiling, never a fixed sleep or an unbounded spin;
//! - a response is read whole and size-checked (`tpm_wire`), and bytes left over after it are refused;
//! - Startup only when the TPM answers "not started", once per boot, then the command once more (condition 2);
//! - TPM 1.2 is counted only when its `disable` and `deactivated` flags both say on;
//! - the start-up test (the first STARTUP_SAMPLES bytes) has a total ceiling, and a slow TPM says how slow.
#![allow(dead_code)]

use crate::tpm_wire::{self as w, Cmd, Flags, Ver, WireErr};

pub const ACCESS: u64 = 0x00;
pub const STS: u64 = 0x18;
pub const FIFO: u64 = 0x24;
pub const DID_VID: u64 = 0xF00;

pub const ACCESS_REQUEST_USE: u8 = 1 << 1;
pub const ACCESS_ACTIVE: u8 = 1 << 5;
pub const ACCESS_VALID: u8 = 1 << 7;

pub const STS_VALID: u32 = 1 << 7;
pub const STS_COMMAND_READY: u32 = 1 << 6;
pub const STS_GO: u32 = 1 << 5;
pub const STS_DATA_AVAIL: u32 = 1 << 4;
pub const STS_EXPECT: u32 = 1 << 3;

/// TIS timeout A (locality), and the ceiling for a command to finish (TIS timeout B; GetRandom is a short one).
pub const TIMEOUT_A_MS: u64 = 750;
pub const TIMEOUT_CMD_MS: u64 = 2000;
/// Architect's condition 5: the start-up test's total ceiling.
pub const STARTUP_CEILING_MS: u64 = 2000;

/// The TIS registers of locality 0 and a clock. The kernel's is MMIO and the TSC; the host test's is a script.
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
    Wire(WireErr),
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

fn burst<B: Bus>(b: &mut B) -> usize {
    ((b.r32(STS) >> 8) & 0xFFFF) as usize
}

fn active<B: Bus>(b: &mut B) -> bool {
    b.r8(ACCESS) & (ACCESS_VALID | ACCESS_ACTIVE) == ACCESS_VALID | ACCESS_ACTIVE
}

pub fn request_locality<B: Bus>(b: &mut B) -> bool {
    if active(b) {
        return true;
    }
    b.w8(ACCESS, ACCESS_REQUEST_USE);
    wait(b, TIMEOUT_A_MS, |b| active(b))
}

/// Give locality 0 up (writing activeLocality relinquishes it), so firmware that uses the TPM later finds it free.
pub fn release_locality<B: Bus>(b: &mut B) {
    b.w8(ACCESS, ACCESS_ACTIVE);
    let _ = wait(b, TIMEOUT_A_MS, |b| b.r8(ACCESS) & ACCESS_ACTIVE == 0);
}

fn data_avail<B: Bus>(b: &mut B) -> bool {
    b.r32(STS) & (STS_VALID | STS_DATA_AVAIL) == STS_VALID | STS_DATA_AVAIL
}

/// One command, one response (whole, size-checked) into `rsp`; returns the response length.
pub fn transact<B: Bus>(b: &mut B, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    b.w32(STS, STS_COMMAND_READY);
    if !wait(b, TIMEOUT_CMD_MS, |b| b.r32(STS) & STS_COMMAND_READY != 0) {
        return Err(IoErr::NotReady);
    }
    let mut sent = 0;
    while sent < cmd.len() {
        if !wait(b, TIMEOUT_A_MS, |b| burst(b) > 0) {
            b.w32(STS, STS_COMMAND_READY);
            return Err(IoErr::NoBurst);
        }
        let k = burst(b).min(cmd.len() - sent);
        for &x in &cmd[sent..sent + k] {
            b.w8(FIFO, x);
        }
        sent += k;
    }
    // All bytes in: the TPM must say it expects no more.
    if !wait(b, TIMEOUT_A_MS, |b| b.r32(STS) & STS_VALID != 0) || b.r32(STS) & STS_EXPECT != 0 {
        b.w32(STS, STS_COMMAND_READY);
        return Err(IoErr::Expect);
    }
    b.w32(STS, STS_GO);
    if !wait(b, TIMEOUT_CMD_MS, |b| data_avail(b)) {
        b.w32(STS, STS_COMMAND_READY);
        return Err(IoErr::Timeout);
    }
    let r = read_rsp(b, rsp);
    b.w32(STS, STS_COMMAND_READY); // done: the TPM may drop the response and go idle
    r
}

fn read_into<B: Bus>(b: &mut B, dst: &mut [u8]) -> Result<(), IoErr> {
    let mut got = 0;
    while got < dst.len() {
        if !wait(b, TIMEOUT_A_MS, |b| burst(b) > 0) {
            return Err(IoErr::NoBurst);
        }
        let k = burst(b).min(dst.len() - got);
        for x in &mut dst[got..got + k] {
            *x = b.r8(FIFO);
        }
        got += k;
    }
    Ok(())
}

fn read_rsp<B: Bus>(b: &mut B, rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    read_into(b, &mut rsp[..w::HDR])?;
    let n = w::rsp_size(&rsp[..w::HDR]).map_err(IoErr::Wire)?;
    read_into(b, &mut rsp[w::HDR..n])?;
    // Nothing may be left over (a response longer than its size field is not one we understand).
    if wait(b, 1, |b| data_avail(b)) {
        return Err(IoErr::Extra);
    }
    Ok(n)
}

/// Send `c` and parse the answer's header; the body (a copy, bounded) goes to `body`, its length is returned.
pub fn call<B: Bus>(b: &mut B, s: &Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    let mut cmd = [0u8; w::CMD_MAX];
    let n = w::build(s.ver, c, &mut cmd).map_err(IoErr::Wire)?;
    let mut rsp = [0u8; w::RSP_MAX];
    let len = transact(b, &cmd[..n], &mut rsp)?;
    let r = match w::parse(s.ver, &rsp[..len]) {
        Ok(x) => {
            body[..x.len()].copy_from_slice(x);
            Ok(x.len())
        }
        Err(e) => Err(IoErr::Wire(e)),
    };
    rsp.fill(0);
    r
}

pub fn not_started(e: IoErr) -> bool {
    matches!(e, IoErr::Wire(WireErr::Rc(w::RC12_INVALID_POSTINIT)) | IoErr::Wire(WireErr::Rc(w::RC20_INITIALIZE)))
}

/// `c`, and if the TPM says it was never started: Startup once, then `c` once more (condition 2).
pub fn call_starting<B: Bus>(b: &mut B, s: &mut Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    match call(b, s, c, body) {
        Err(e) if not_started(e) && !s.started_by_us => {
            s.started_by_us = true;
            call(b, s, Cmd::Startup, body)?;
            call(b, s, c, body)
        }
        r => r,
    }
}

/// One GetRandom of up to 32 bytes into `dst`; returns the bytes written.
pub fn get_random_once<B: Bus>(b: &mut B, s: &mut Session, dst: &mut [u8]) -> Result<usize, IoErr> {
    let ask = dst.len().min(w::RANDOM_MAX);
    let mut body = [0u8; w::RSP_MAX];
    let n = call_starting(b, s, Cmd::GetRandom(ask as u16), &mut body)?;
    let r = match w::random_bytes(s.ver, &body[..n], ask) {
        Ok(x) => {
            dst[..x.len()].copy_from_slice(x);
            Ok(x.len())
        }
        Err(e) => Err(IoErr::Wire(e)),
    };
    body.fill(0);
    r
}

/// TPM 1.2: the permanent `disable` and volatile `deactivated` flags; Ok(true) only when both say on.
pub fn tpm12_switched_on<B: Bus>(b: &mut B, s: &mut Session) -> Result<bool, IoErr> {
    let mut body = [0u8; w::RSP_MAX];
    let n = call_starting(b, s, Cmd::GetCapFlags(Flags::Permanent), &mut body)?;
    let disabled = w::first_flag(&body[..n], Flags::Permanent).map_err(IoErr::Wire)?;
    let n = call(b, s, Cmd::GetCapFlags(Flags::Volatile), &mut body)?;
    let deactivated = w::first_flag(&body[..n], Flags::Volatile).map_err(IoErr::Wire)?;
    Ok(!disabled && !deactivated)
}

/// The boot probe, with locality held by the caller: for 1.2 the flags must say on, then one GetRandom of 32 bytes
/// must come back whole. Ok carries that call's time in microseconds; Err says why the TPM is not counted.
pub fn probe<B: Bus>(b: &mut B, s: &mut Session) -> Result<u64, &'static str> {
    if s.ver == Ver::V12 {
        match tpm12_switched_on(b, s) {
            Ok(true) => {}
            Ok(false) => return Err("switched off in the firmware setup; switch it on there to use it"),
            Err(_) => return Err("its on/off flags could not be read, so it is not counted"),
        }
    }
    let mut x = [0u8; w::RANDOM_MAX];
    let t0 = b.now();
    let r = get_random_once(b, s, &mut x);
    let dt = b.now().wrapping_sub(t0);
    x.fill(0);
    match r {
        Ok(n) if n == w::RANDOM_MAX => Ok(dt.saturating_mul(1_000_000) / b.hz().max(1)),
        Ok(_) => Err("GetRandom gave fewer bytes than asked for"),
        Err(IoErr::Wire(WireErr::Rc(_))) => Err("GetRandom returned an error code"),
        Err(IoErr::Wire(_)) => Err("GetRandom's answer was malformed"),
        Err(_) => Err("GetRandom timed out"),
    }
}

/// One batch, with locality held by the caller: GetRandom until `buf` is full or the TPM stops answering.
/// Returns the bytes written, the GetRandom calls made, and whether it stopped answering.
pub fn fill_batch<B: Bus>(b: &mut B, s: &mut Session, buf: &mut [u8]) -> (usize, u64, bool) {
    let mut n = 0;
    let mut calls = 0u64;
    while n < buf.len() {
        calls += 1;
        match get_random_once(b, s, &mut buf[n..]) {
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

/// How slow a TPM that missed the start-up ceiling was (follow-up 4: the "too slow" line prints these).
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
