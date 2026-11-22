//! The TPM's random number generator as a counted strong source (F3 step 3, ADR 0008 Decision 1a; plan
//! PLAN_OS_MESH_F3_STEP3_TPM.md, architect's OK with conditions in ARCHITECT_REVIEWS 13 Nov evening).
//!
//! Locality 0 at 0xFED40000: TIS (FIFO) for TPM 1.2 (ACPI TCPA) and TPM 2.0 (ACPI TPM2, start method 6), and CRB for
//! TPM 2.0 (start method 7, firmware TPMs such as Intel PTT; tpm_crb.rs, architect's OK 14 Nov 2026).
//! - No TPM table: no MMIO access at all. When a TPM2 table is listed, a TCPA table is never used as a fallback to
//!   TIS, so TPM 1.2 commands never go to a TPM 2.0. Start methods 2 and 8 need ACPI's _DSM (AML): off, with the
//!   number.
//! - Before anything is written: TIS needs TPM_ACCESS with tpmRegValidSts and DID_VID neither 0 nor all ones; CRB
//!   needs every check in `tpm_crb::check` (control area and buffers inside the page, INTF_ID says CRB).
//! - One command at a time; every wait has a clock ceiling (the TSC), never a fixed sleep or an unbounded spin.
//! - Command bytes and response parsing only through `tpm_wire` (the closed command list, bounded reads).
//! - Startup only when the TPM answers "not started", once. Any other error leaves the TPM alone and off.
//! - TPM 1.2 is counted only when its permanent `disable` and volatile `deactivated` flags both say on.
//! - Locality 0 is requested per batch and given up afterwards, so firmware finds the TPM free.
//! - The 1024-sample start-up test is capped at 2 s in total; a slower TPM is not counted for this boot.
//! The MMIO page is mapped uncached by mmu.rs (0xFEC00000..0xFF000000, the APIC block). Single accessor: the rng
//! path on the shell's core.
#![allow(dead_code)]

use crate::tpm_crb::{self as crb, CrbLink, CrbOff, Layout};
use crate::tpm_rules::{self as rules, Budget, Bus, Session, Slow};
use crate::tpm_tis::{self as tis, TisLink};
use crate::tpm_wire::Ver;

const BASE: u64 = 0xFED4_0000;

/// The real bus: locality 0's registers by MMIO, and the TSC.
struct Mmio;

impl Bus for Mmio {
    fn r8(&mut self, off: u64) -> u8 {
        // SAFETY: only called after a TPM table said a TPM is here, with an offset inside the page (TIS: fixed
        // registers; CRB: `tpm_crb::check` kept every offset inside it); the page is mapped 1:1 uncached (mmu.rs).
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
    /// Present but not used; the reason is in `OFF`.
    Off,
}

/// Which interface drives the TPM.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Iface {
    Tis,
    Crb(Layout),
}

/// Why the TPM is off, with the numbers a field report needs.
#[derive(Clone, Copy)]
enum Why {
    Text(&'static str),
    Slow(Slow),
    Crb(CrbOff),
    StartMethod(u32),
}

static mut STATE: State = State::Unprobed;
static mut IFACE: Iface = Iface::Tis;
static mut SESSION: Session = Session { ver: Ver::V20, started_by_us: false };
static mut OFF: Why = Why::Text("");
static mut DIDVID: u32 = 0;
static mut CALL_US: u64 = 0;
/// The start-up budget: bytes, TSC ticks and calls spent in GetRandom until STARTUP_SAMPLES have gone out.
static mut BUDGET: Budget = Budget { served: 0, spent: 0, calls: 0 };

fn st() -> State {
    // SAFETY: single accessor (the rng path on the shell's core).
    unsafe { STATE }
}

fn off_why(why: Why) {
    // SAFETY: as above.
    unsafe {
        STATE = State::Off;
        OFF = why;
    }
}

fn off(why: &'static str) {
    off_why(Why::Text(why));
}

/// hwdb's hook for MSFT0101 (AcpiV::Asks; the architect's Q2 on the CRB plan, 14 Nov 2026: hwdb asks the owning
/// driver, it holds no fixed answer). Driven when the TPM answers and is counted; off when this driver found it and
/// left it off (`rng sources` gives the reason and the values); missing before the probe or with no TPM table.
pub fn claim() -> crate::hwdb::Claim {
    use crate::hwdb::Claim;
    match st() {
        State::On => Claim::Driven(match iface() {
            Iface::Crb(_) => "tpm, CRB (its random numbers are a counted source)",
            Iface::Tis => "tpm, TIS (its random numbers are a counted source)",
        }),
        State::Off => Claim::Off("tpm found it and left it off; rng sources gives the reason and the values"),
        State::Unprobed | State::None => Claim::Missing,
    }
}

fn iface() -> Iface {
    // SAFETY: single accessor.
    unsafe { IFACE }
}

fn session() -> &'static mut Session {
    // SAFETY: single accessor (the rng path on the shell's core); no other reference to SESSION is live.
    unsafe { &mut *core::ptr::addr_of_mut!(SESSION) }
}

fn request_locality(b: &mut Mmio) -> bool {
    match iface() {
        Iface::Tis => tis::request_locality(b),
        Iface::Crb(_) => crb::request_locality(b),
    }
}

fn release_locality(b: &mut Mmio) {
    match iface() {
        Iface::Tis => tis::release_locality(b),
        Iface::Crb(_) => crb::release_locality(b),
    }
}

/// The boot probe, through the interface's link (locality held by the caller).
fn probe(b: &mut Mmio) -> Result<u64, &'static str> {
    match iface() {
        Iface::Tis => rules::probe(&mut TisLink(b), session()),
        Iface::Crb(layout) => rules::probe(&mut CrbLink { bus: b, layout }, session()),
    }
}

/// One batch, through the interface's link (locality held by the caller).
fn fill_batch(b: &mut Mmio, buf: &mut [u8]) -> (usize, u64, bool) {
    match iface() {
        Iface::Tis => rules::fill_batch(&mut TisLink(b), session(), buf),
        Iface::Crb(layout) => rules::fill_batch(&mut CrbLink { bus: b, layout }, session(), buf),
    }
}

/// TIS, condition 4: absent before anything is written. True when the registers answer.
fn tis_present(b: &mut Mmio) -> bool {
    let (acc, did) = (b.r8(tis::ACCESS), b.r32(tis::DID_VID));
    if acc & tis::ACCESS_VALID == 0 || did == 0 || did == u32::MAX {
        off("table but no TIS answer");
        return false;
    }
    // SAFETY: single accessor.
    unsafe { DIDVID = did };
    true
}

/// Boot: decide whether there is a TPM to count, and print one `[tpm]` line saying what was found.
pub fn init() {
    let t = crate::acpi::tpm_tables();
    let mut b = Mmio;
    // A TPM2 table decides alone: TCPA is not a fallback when TPM2 is listed (the HP lists both for one PTT).
    if t.tpm2.0 != 0 {
        session().ver = Ver::V20;
        match t.tpm2_start {
            6 => {
                if !tis_present(&mut b) {
                    return line();
                }
            }
            7 => match crb::check(&mut b, BASE, t.tpm2_ctrl) {
                // SAFETY: single accessor.
                Ok(layout) => unsafe {
                    IFACE = Iface::Crb(layout);
                    DIDVID = layout.vid_did;
                },
                Err(e) => {
                    off_why(Why::Crb(e));
                    return line();
                }
            },
            2 | 8 => {
                off_why(Why::StartMethod(t.tpm2_start));
                return line();
            }
            _ => {
                off("TPM2 table names a start method other than TIS");
                return line();
            }
        }
    } else if t.tcpa.0 != 0 {
        session().ver = Ver::V12;
        if !tis_present(&mut b) {
            return line();
        }
    } else {
        // SAFETY: single accessor.
        unsafe { STATE = State::None };
        return line();
    }
    if !request_locality(&mut b) {
        off("locality 0 was not granted");
        return line();
    }
    let r = probe(&mut b);
    release_locality(&mut b);
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
    if !request_locality(&mut b) {
        off("locality 0 was not granted");
        return 0;
    }
    let t0 = b.now();
    let (n, calls, stopped) = fill_batch(&mut b, buf);
    if stopped {
        off("stopped answering GetRandom");
    }
    release_locality(&mut b);
    let dt = b.now().wrapping_sub(t0);
    // SAFETY: single accessor.
    let budget = unsafe { &mut *core::ptr::addr_of_mut!(BUDGET) };
    if let Err(slow) = budget.charge(n, dt, calls, b.hz(), crate::drbg::STARTUP_SAMPLES, rules::STARTUP_CEILING_MS) {
        off_why(Why::Slow(slow));
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
    let (state, did, us, why, started) = unsafe { (STATE, DIDVID, CALL_US, OFF, SESSION.started_by_us) };
    let via = match iface() {
        Iface::Tis => "TIS",
        Iface::Crb(_) => "CRB",
    };
    match state {
        State::Unprobed => crate::kprintln!("{}not probed", prefix),
        State::None => crate::kprintln!("{}none (no TCPA or TPM2 table)", prefix),
        State::On => crate::kprintln!(
            "{}TPM {} over {} at 0x{:x}: vendor 0x{:04x} device 0x{:04x}, GetRandom ok (32 bytes in {} us){}",
            prefix, ver_name(), via, BASE, did & 0xFFFF, did >> 16, us,
            if started { ", started by us" } else { "" }
        ),
        State::Off => match why {
            // Follow-up 4: the measured time, so a slow TPM on a real machine says how slow.
            Why::Slow(s) => crate::kprintln!(
                "{}off (too slow: the 1024-byte start-up test took {} ms over {} GetRandom calls, {} us per call; {}",
                prefix, s.total_ms, s.calls, s.us_per_call, "the ceiling is 2000 ms, so it is not counted this boot)"
            ),
            Why::Crb(e) => crate::kprintln!("{}off ({})", prefix, e),
            Why::StartMethod(m) => crate::kprintln!(
                "{}off (TPM2 start method {} needs the ACPI start method (AML), not supported)",
                prefix, m
            ),
            Why::Text(t) => crate::kprintln!("{}off ({})", prefix, t),
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
