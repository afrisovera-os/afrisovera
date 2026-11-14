//! The TPM's random number generator as a counted strong source (F3 step 3, ADR 0008 Decision 1a; plan
//! PLAN_OS_MESH_F3_STEP3_TPM.md, architect's OK with conditions in ARCHITECT_REVIEWS 13 Nov evening).
//!
//! A TIS (FIFO) driver, locality 0 at 0xFED40000, for TPM 1.2 (ACPI TCPA) and TPM 2.0 (ACPI TPM2, start method 6).
//! - No TPM table: no MMIO access at all. CRB (start methods 7 and 8, firmware TPMs) is reported, not driven.
//! - Before anything is written: TPM_ACCESS must show tpmRegValidSts and DID_VID must be neither 0 nor all ones.
//! - One command at a time; every wait has a clock ceiling (the TSC), never a fixed sleep or an unbounded spin.
//! - Command bytes and response parsing only through `tpm_wire` (the closed command list, bounded reads).
//! - Startup only when the TPM answers "not started", once. Any other error leaves the TPM alone and off.
//! - TPM 1.2 is counted only when its permanent `disable` and volatile `deactivated` flags both say on.
//! - Locality 0 is requested per batch and given up afterwards, so firmware finds the TPM free.
//! - The 1024-sample start-up test is capped at 2 s in total; a slower TPM is not counted for this boot.
//! The MMIO page is mapped uncached by mmu.rs (0xFEC00000..0xFF000000, the APIC block). Single accessor: the rng
//! path on the shell's core.
#![allow(dead_code)]

use crate::tpm_tis::{self as tis, Budget, Bus, Session, Slow};
use crate::tpm_wire::Ver;

const BASE: u64 = 0xFED4_0000;

/// The real bus: locality 0's registers by MMIO, and the TSC.
struct Mmio;

impl Bus for Mmio {
    fn r8(&mut self, off: u64) -> u8 {
        // SAFETY: only called after a TPM table said a TIS is here; the page is mapped 1:1 uncached (mmu.rs).
        unsafe { core::ptr::read_volatile((BASE + off) as *const u8) }
    }
    fn r32(&mut self, off: u64) -> u32 {
        // SAFETY: as r8.
        unsafe { core::ptr::read_volatile((BASE + off) as *const u32) }
    }
    fn w8(&mut self, off: u64, v: u8) {
        // SAFETY: as r8.
        unsafe { core::ptr::write_volatile((BASE + off) as *mut u8, v) }
    }
    fn w32(&mut self, off: u64, v: u32) {
        // SAFETY: as r8.
        unsafe { core::ptr::write_volatile((BASE + off) as *mut u32, v) }
    }
    fn now(&mut self) -> u64 {
        crate::timer::rdtsc()
    }
    fn hz(&self) -> u64 {
        crate::timer::hz()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not looked at yet (before `init`).
    Unprobed,
    /// No TCPA or TPM2 table.
    None,
    /// Counted: answering GetRandom.
    On,
    /// Present but not used; the reason is in `OFF` (and `SLOW` when it missed the start-up ceiling).
    Off,
}

static mut STATE: State = State::Unprobed;
static mut SESSION: Session = Session { ver: Ver::V20, started_by_us: false };
static mut OFF: &str = "";
static mut SLOW: Option<Slow> = None;
static mut DIDVID: u32 = 0;
static mut CALL_US: u64 = 0;
/// The start-up budget: bytes, TSC ticks and calls spent in GetRandom until STARTUP_SAMPLES have gone out.
static mut BUDGET: Budget = Budget { served: 0, spent: 0, calls: 0 };

fn st() -> State {
    // SAFETY: single accessor (the rng path on the shell's core).
    unsafe { STATE }
}

fn off(why: &'static str) {
    // SAFETY: as above.
    unsafe {
        STATE = State::Off;
        OFF = why;
    }
}

/// Boot: decide whether there is a TPM to count, and print one `[tpm]` line saying what was found.
pub fn init() {
    let t = crate::acpi::tpm_tables();
    // SAFETY: single accessor, before any rng use of the TPM (STATE was Unprobed, so `fill` returned 0).
    unsafe {
        if t.tpm2.0 != 0 {
            SESSION.ver = Ver::V20;
            match t.tpm2_start {
                6 => {}
                7 | 8 => {
                    off("CRB, not supported yet: only the TIS interface is driven");
                    return line();
                }
                _ => {
                    off("TPM2 table names a start method other than TIS");
                    return line();
                }
            }
        } else if t.tcpa.0 != 0 {
            SESSION.ver = Ver::V12;
        } else {
            STATE = State::None;
            return line();
        }
    }
    let mut b = Mmio;
    // Condition 4: absent before anything is written.
    let (acc, did) = (b.r8(tis::ACCESS), b.r32(tis::DID_VID));
    if acc & tis::ACCESS_VALID == 0 || did == 0 || did == u32::MAX {
        off("table but no TIS answer");
        return line();
    }
    // SAFETY: single accessor.
    unsafe { DIDVID = did };
    if !tis::request_locality(&mut b) {
        off("locality 0 was not granted");
        return line();
    }
    // SAFETY: single accessor.
    let r = tis::probe(&mut b, unsafe { &mut *core::ptr::addr_of_mut!(SESSION) });
    tis::release_locality(&mut b);
    match r {
        // SAFETY: single accessor.
        Ok(us) => unsafe {
            CALL_US = us;
            STATE = State::On;
        },
        Err(why) => off(why),
    }
    line();
}

/// `Platform::strong` for the TPM: up to `buf.len()` bytes, 0 when it is not counted or stops answering. The first
/// STARTUP_SAMPLES bytes (the generator's start-up test) must come within STARTUP_CEILING_MS in total.
pub fn fill(buf: &mut [u8]) -> usize {
    if st() != State::On || buf.is_empty() {
        return 0;
    }
    let mut b = Mmio;
    if !tis::request_locality(&mut b) {
        off("locality 0 was not granted");
        return 0;
    }
    let t0 = b.now();
    // SAFETY: single accessor.
    let (n, calls, stopped) = tis::fill_batch(&mut b, unsafe { &mut *core::ptr::addr_of_mut!(SESSION) }, buf);
    if stopped {
        off("stopped answering GetRandom");
    }
    tis::release_locality(&mut b);
    let dt = b.now().wrapping_sub(t0);
    // SAFETY: single accessor.
    let budget = unsafe { &mut *core::ptr::addr_of_mut!(BUDGET) };
    if let Err(slow) = budget.charge(n, dt, calls, b.hz(), crate::drbg::STARTUP_SAMPLES, tis::STARTUP_CEILING_MS) {
        off("too slow");
        // SAFETY: single accessor.
        unsafe { SLOW = Some(slow) };
        buf[..n].fill(0);
        return 0;
    }
    n
}

pub fn counted() -> bool {
    st() == State::On
}

fn ver_name() -> &'static str {
    // SAFETY: single accessor.
    match unsafe { SESSION.ver } {
        Ver::V12 => "1.2",
        Ver::V20 => "2.0",
    }
}

fn summary(prefix: &str) {
    // SAFETY: single accessor; read-only here.
    let (state, did, us, why, slow, started) = unsafe { (STATE, DIDVID, CALL_US, OFF, SLOW, SESSION.started_by_us) };
    match state {
        State::Unprobed => crate::kprintln!("{}not probed", prefix),
        State::None => crate::kprintln!("{}none (no TCPA or TPM2 table)", prefix),
        State::On => crate::kprintln!(
            "{}TPM {} over TIS at 0x{:x}: vendor 0x{:04x} device 0x{:04x}, GetRandom ok (32 bytes in {} us){}",
            prefix, ver_name(), BASE, did & 0xFFFF, did >> 16, us,
            if started { ", started by us" } else { "" }
        ),
        State::Off => match slow {
            // Follow-up 4: the measured time, so a slow TPM on a real machine says how slow.
            Some(s) => crate::kprintln!(
                "{}off (too slow: the 1024-byte start-up test took {} ms over {} GetRandom calls, {} us per call; {}",
                prefix, s.total_ms, s.calls, s.us_per_call, "the ceiling is 2000 ms, so it is not counted this boot)"
            ),
            None => crate::kprintln!("{}off ({})", prefix, why),
        },
    }
}

/// The boot line.
fn line() {
    summary("[tpm] ");
}

/// For `rng sources`.
pub fn sources_line() {
    summary("rng sources: tpm: ");
}
