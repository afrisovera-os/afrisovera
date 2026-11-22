//! The TPM's TIS (FIFO) protocol, generic over the register bus (architect's TPM merge follow-up 2, 13 Nov 2026).
//! Pure: no MMIO, no clock and no statics of its own. tpm.rs supplies the real bus (MMIO at 0xFED40000 and the TSC);
//! drbg-host/tests/tpm_tis.rs supplies a scripted TPM, so this is tested on the host:
//! - one command at a time, and every wait has a clock ceiling, never a fixed sleep or an unbounded spin;
//! - a response is read whole and size-checked (`tpm_wire`), and bytes left over after it are refused.
//! The rules above the interface (Startup once, the 1.2 flags, the start-up ceiling) are in tpm_rules.rs, over
//! `TisLink`; the functions at the end of this file call them through it, for tpm.rs and the TIS host tests.
#![allow(dead_code)]

use crate::tpm_rules::{self as rules, Link};
pub use crate::tpm_rules::{
    not_started, Budget, Bus, IoErr, Session, Slow, STARTUP_CEILING_MS,
};
pub use crate::tpm_rules::{ticks, wait};
use crate::tpm_wire::{self as w, Cmd};

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

/// The TIS path as a `Link` for the shared rules.
pub struct TisLink<'a, B: Bus>(pub &'a mut B);

impl<B: Bus> Link for TisLink<'_, B> {
    fn transact(&mut self, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
        transact(self.0, cmd, rsp)
    }
    fn now(&mut self) -> u64 {
        self.0.now()
    }
    fn hz(&self) -> u64 {
        self.0.hz()
    }
}

pub fn call<B: Bus>(b: &mut B, s: &Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    rules::call(&mut TisLink(b), s, c, body)
}

pub fn call_starting<B: Bus>(b: &mut B, s: &mut Session, c: Cmd, body: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    rules::call_starting(&mut TisLink(b), s, c, body)
}

pub fn get_random_once<B: Bus>(b: &mut B, s: &mut Session, dst: &mut [u8]) -> Result<usize, IoErr> {
    rules::get_random_once(&mut TisLink(b), s, dst)
}

pub fn tpm12_switched_on<B: Bus>(b: &mut B, s: &mut Session) -> Result<bool, IoErr> {
    rules::tpm12_switched_on(&mut TisLink(b), s)
}

pub fn probe<B: Bus>(b: &mut B, s: &mut Session) -> Result<u64, &'static str> {
    rules::probe(&mut TisLink(b), s)
}

pub fn fill_batch<B: Bus>(b: &mut B, s: &mut Session, buf: &mut [u8]) -> (usize, u64, bool) {
    rules::fill_batch(&mut TisLink(b), s, buf)
}
