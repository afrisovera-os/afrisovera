//! The TPM 2.0 CRB interface (Command Response Buffer, TPM2 start method 7: firmware TPMs such as Intel PTT), generic
//! over the register bus (plan PLAN_OS_MESH_F3_TPM_CRB.md, architect's OK 14 Nov 2026). Pure: no MMIO, no clock and no
//! statics of its own; tpm.rs supplies the real bus, drbg-host/tests/tpm_crb.rs a scripted CRB TPM.
//! - No write before every check passes (`check`): the control area and both buffers, with their sizes, lie inside
//!   the locality-0 page (checked arithmetic, high address halves zero) and clear of the control registers;
//!   INTF_ID says CRB; LOC_STATE is valid. Anything else is off with the reason and the values.
//! - Reads stay inside the response buffer: never past min(RSP_SIZE, 128), the size checked before the body.
//! - Every wait is bounded on the bus clock; a command that does not finish is cancelled, then the TPM is off.
//! - The locality is given up and the TPM sent to idle after each command and batch: the PTT is shared with the
//!   Management Engine.
#![allow(dead_code)]

use crate::tpm_rules::{wait, Bus, IoErr, Link};
use crate::tpm_wire as w;

/// The locality-0 page (TPM_LOC_STATE_0 at its start).
pub const PAGE: u64 = 0x1000;

pub const LOC_STATE: u64 = 0x00;
pub const LOC_CTRL: u64 = 0x08;
pub const INTF_ID: u64 = 0x30;
pub const INTF_ID_HI: u64 = 0x34;

pub const LOC_STATE_ASSIGNED: u32 = 1 << 1;
pub const LOC_STATE_VALID: u32 = 1 << 7;
pub const LOC_CTRL_REQUEST: u32 = 1 << 0;
pub const LOC_CTRL_RELINQUISH: u32 = 1 << 1;
/// INTF_ID bits 0..3: 0 FIFO (TIS), 1 CRB, 0xF FIFO with a TIS-compatible id.
pub const INTF_TYPE_CRB: u32 = 1;

/// The control area's registers, relative to its start (TPM_CRB_CTRL_REQ_0; 0x40 in the page on Intel and QEMU).
pub const REQ: u64 = 0x00;
pub const STS: u64 = 0x04;
pub const CANCEL: u64 = 0x08;
pub const START: u64 = 0x0C;
pub const CMD_SIZE: u64 = 0x18;
pub const CMD_LADDR: u64 = 0x1C;
pub const CMD_HADDR: u64 = 0x20;
pub const RSP_SIZE: u64 = 0x24;
pub const RSP_ADDR: u64 = 0x28; // 64 bits: low word here, high word at +4
/// The control area's length: up to the end of RSP_ADDR.
pub const CTRL_LEN: u64 = 0x30;

pub const REQ_CMD_READY: u32 = 1 << 0;
pub const REQ_GO_IDLE: u32 = 1 << 1;
pub const STS_FATAL: u32 = 1 << 0;
pub const START_INVOKE: u32 = 1;
pub const CANCEL_INVOKE: u32 = 1;

/// TPM timeout C (locality, cmdReady, goIdle) and the ceiling for a command, as on the TIS path.
pub const TIMEOUT_C_MS: u64 = 750;
pub const TIMEOUT_CMD_MS: u64 = 2000;

/// Where the control area and the two buffers are, as offsets in the page, once `check` has passed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Layout {
    pub ctrl: u64,
    pub cmd: u64,
    pub cmd_size: u64,
    pub rsp: u64,
    pub rsp_size: u64,
    /// INTF_ID's high word: vendor id (low 16 bits) and device id (high 16).
    pub vid_did: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Buf {
    Command,
    Response,
}

/// Why a CRB TPM is not used. Each says the values it saw, for a field report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrbOff {
    /// The TPM2 table's control area address is not in the page with room for the control registers.
    Ctrl(u64),
    /// LOC_STATE without tpmRegValidSts, or all ones: nothing answers.
    NoAnswer(u32),
    /// The table says CRB (start method 7), the chip's interface id says another type.
    NotCrb(u32),
    /// A buffer's 64-bit address and size are not inside the page, clear of the control area.
    Outside(Buf, u64, u64),
    /// A buffer smaller than the largest command or response this driver uses.
    Small(Buf, u64),
}

fn buf_name(b: Buf) -> &'static str {
    match b {
        Buf::Command => "command",
        Buf::Response => "response",
    }
}

impl core::fmt::Display for CrbOff {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            CrbOff::Ctrl(a) => write!(f, "CRB control area 0x{:x} is not inside the TPM's page", a),
            CrbOff::NoAnswer(v) => write!(f, "table but no CRB answer (loc_state 0x{:x})", v),
            CrbOff::NotCrb(id) => write!(
                f,
                "the TPM2 table says CRB (start method 7) but the chip's interface id 0x{:x} says type {}",
                id,
                id & 0xF
            ),
            CrbOff::Outside(b, a, n) => write!(
                f,
                "CRB {} buffer 0x{:x} ({} bytes) is not inside the TPM's page, clear of the control area",
                buf_name(b),
                a,
                n
            ),
            CrbOff::Small(b, n) => write!(f, "CRB {} buffer of {} bytes is too small", buf_name(b), n),
        }
    }
}

/// `[a, a + n)` as an offset range inside the page, or None (checked arithmetic, never wraps).
fn in_page(base: u64, a: u64, n: u64) -> Option<u64> {
    let off = a.checked_sub(base)?;
    let end = off.checked_add(n)?;
    if end <= PAGE {
        Some(off)
    } else {
        None
    }
}

/// Every check before the first write (architect's first CRB condition). Only reads, and only of registers inside the
/// page. `base` is the page's physical address, `ctrl_pa` the TPM2 table's control area address.
pub fn check<B: Bus>(b: &mut B, base: u64, ctrl_pa: u64) -> Result<Layout, CrbOff> {
    let ctrl = in_page(base, ctrl_pa, CTRL_LEN).ok_or(CrbOff::Ctrl(ctrl_pa))?;
    if ctrl < INTF_ID_HI + 4 {
        return Err(CrbOff::Ctrl(ctrl_pa)); // the control area would overlap the locality registers
    }
    let st = b.r32(LOC_STATE);
    if st == u32::MAX || st & LOC_STATE_VALID == 0 {
        return Err(CrbOff::NoAnswer(st));
    }
    let id = b.r32(INTF_ID);
    if id == u32::MAX || id & 0xF != INTF_TYPE_CRB {
        return Err(CrbOff::NotCrb(id));
    }
    let vid_did = b.r32(INTF_ID_HI);
    let cmd_size = b.r32(ctrl + CMD_SIZE) as u64;
    let cmd_pa = (b.r32(ctrl + CMD_HADDR) as u64) << 32 | b.r32(ctrl + CMD_LADDR) as u64;
    let rsp_size = b.r32(ctrl + RSP_SIZE) as u64;
    let rsp_pa = (b.r32(ctrl + RSP_ADDR + 4) as u64) << 32 | b.r32(ctrl + RSP_ADDR) as u64;
    let clear = |off: u64, n: u64| off.checked_add(n).map_or(false, |end| end <= ctrl || off >= ctrl + CTRL_LEN);
    // neither buffer may start on the locality registers either (the floor the control area has): a command written
    // there would land on LOC_CTRL and the registers after it (the architect's merge review, 21 Nov 2026)
    let floor = INTF_ID_HI + 4;
    let cmd = match in_page(base, cmd_pa, cmd_size) {
        Some(o) if o >= floor && clear(o, cmd_size) => o,
        _ => return Err(CrbOff::Outside(Buf::Command, cmd_pa, cmd_size)),
    };
    let rsp = match in_page(base, rsp_pa, rsp_size) {
        Some(o) if o >= floor && clear(o, rsp_size) => o,
        _ => return Err(CrbOff::Outside(Buf::Response, rsp_pa, rsp_size)),
    };
    if cmd_size < w::CMD_MAX as u64 {
        return Err(CrbOff::Small(Buf::Command, cmd_size));
    }
    if rsp_size < w::RSP_MAX as u64 {
        return Err(CrbOff::Small(Buf::Response, rsp_size));
    }
    Ok(Layout { ctrl, cmd, cmd_size, rsp, rsp_size, vid_did })
}

fn assigned<B: Bus>(b: &mut B) -> bool {
    b.r32(LOC_STATE) & (LOC_STATE_VALID | LOC_STATE_ASSIGNED) == LOC_STATE_VALID | LOC_STATE_ASSIGNED
}

pub fn request_locality<B: Bus>(b: &mut B) -> bool {
    if assigned(b) {
        return true;
    }
    b.w32(LOC_CTRL, LOC_CTRL_REQUEST);
    wait(b, TIMEOUT_C_MS, |b| assigned(b))
}

/// Give locality 0 up, so the Management Engine and firmware find the TPM free.
pub fn release_locality<B: Bus>(b: &mut B) {
    b.w32(LOC_CTRL, LOC_CTRL_RELINQUISH);
    let _ = wait(b, TIMEOUT_C_MS, |b| {
        b.r32(LOC_STATE) & (LOC_STATE_VALID | LOC_STATE_ASSIGNED) == LOC_STATE_VALID
    });
}

/// Ask the TPM to go idle (it may power down) and wait for it, bounded.
pub fn go_idle<B: Bus>(b: &mut B, l: &Layout) {
    b.w32(l.ctrl + REQ, REQ_GO_IDLE);
    let _ = wait(b, TIMEOUT_C_MS, |b| b.r32(l.ctrl + REQ) & REQ_GO_IDLE == 0);
}

fn fatal<B: Bus>(b: &mut B, l: &Layout) -> bool {
    b.r32(l.ctrl + STS) & STS_FATAL != 0
}

/// One command, one response (whole, size-checked) into `rsp`; the TPM goes idle afterwards, whatever happened.
pub fn transact<B: Bus>(b: &mut B, l: &Layout, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    let r = transact_inner(b, l, cmd, rsp);
    go_idle(b, l);
    r
}

fn transact_inner<B: Bus>(b: &mut B, l: &Layout, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
    if cmd.len() as u64 > l.cmd_size {
        return Err(IoErr::TooBig);
    }
    if fatal(b, l) {
        return Err(IoErr::Fatal);
    }
    b.w32(l.ctrl + REQ, REQ_CMD_READY);
    if !wait(b, TIMEOUT_C_MS, |b| b.r32(l.ctrl + REQ) & REQ_CMD_READY == 0) {
        return Err(IoErr::NotReady);
    }
    for (i, &x) in cmd.iter().enumerate() {
        b.w8(l.cmd + i as u64, x);
    }
    b.w32(l.ctrl + CANCEL, 0); // a cancel left over from before must not cancel this command
    b.w32(l.ctrl + START, START_INVOKE);
    if !wait(b, TIMEOUT_CMD_MS, |b| b.r32(l.ctrl + START) & START_INVOKE == 0) {
        b.w32(l.ctrl + CANCEL, CANCEL_INVOKE);
        let _ = wait(b, TIMEOUT_C_MS, |b| b.r32(l.ctrl + START) & START_INVOKE == 0);
        b.w32(l.ctrl + CANCEL, 0);
        return Err(IoErr::Timeout);
    }
    if fatal(b, l) {
        return Err(IoErr::Fatal);
    }
    let lim = (l.rsp_size as usize).min(w::RSP_MAX);
    for (i, x) in rsp[..w::HDR].iter_mut().enumerate() {
        *x = b.r8(l.rsp + i as u64);
    }
    let n = w::rsp_size(&rsp[..w::HDR]).map_err(IoErr::Wire)?;
    if n > lim {
        return Err(IoErr::Wire(w::WireErr::BadSize));
    }
    for i in w::HDR..n {
        rsp[i] = b.r8(l.rsp + i as u64);
    }
    Ok(n)
}

/// The CRB path as a `Link` for the shared rules.
pub struct CrbLink<'a, B: Bus> {
    pub bus: &'a mut B,
    pub layout: Layout,
}

impl<B: Bus> Link for CrbLink<'_, B> {
    fn transact(&mut self, cmd: &[u8], rsp: &mut [u8; w::RSP_MAX]) -> Result<usize, IoErr> {
        transact(self.bus, &self.layout, cmd, rsp)
    }
    fn now(&mut self) -> u64 {
        self.bus.now()
    }
    fn hz(&self) -> u64 {
        self.bus.hz()
    }
}
