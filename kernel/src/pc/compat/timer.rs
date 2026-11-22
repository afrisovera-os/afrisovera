//! PC stand-in for the ARM `timer.rs` (the ARM generic timer). The PC's counter is the
//! CPU's time-stamp counter (TSC); its frequency is measured once against the PC's
//! fixed-rate interval timer (the 8254 PIT, 1.193182 MHz, channel 2 gated through port
//! 0x61), the classic calibration every PC kernel does. `hz()` / `cycles()` therefore mean
//! "TSC ticks per second" / "TSC now": the same contract as CNTFRQ / CNTPCT on ARM.
#![allow(dead_code)]

use crate::cpu::{inb, outb};
use core::sync::atomic::{AtomicU64, Ordering};

static HZ: AtomicU64 = AtomicU64::new(0);
static T0: AtomicU64 = AtomicU64::new(0);
/// Where `hz()` came from: 1 the PIT, 2 CPUID 0x15 or 0x16, 3 the ACPI PM timer, 4 assumed (0: not set up yet).
static SOURCE: AtomicU64 = AtomicU64::new(0);
/// Call 79's floor: the largest value any process has been given, so the clock never goes backwards (CLOCK_PLAN 1).
static LAST_NS: AtomicU64 = AtomicU64::new(0);

/// The calibration loops run before the TSC rate is known, so their bound is raw TSC ticks: 1,000,000,000 is at
/// least 100 ms on any TSC up to 10 GHz (each window wants 10 ms). Their spin counts stay as the last fallback.
const CAL_TSC_MAX: u64 = 1_000_000_000;
const PIT_SPIN_CAP: u64 = 20_000_000;
const PMT_SPIN_CAP: u64 = 50_000_000;
/// Calibration windows; the shortest is taken (the one the host disturbed least).
const CAL_WINDOWS: u32 = 3;
/// sleep_ms gives up after twice its time plus this, should the scheduler's tick stop; the wfi count is the
/// fallback when there is no rate.
const SLEEP_SLACK_MS: u64 = 20;
const SLEEP_WFI_CAP: u64 = 200_000_000;

#[inline]
pub fn rdtsc() -> u64 {
    // SAFETY: RDTSC is unprivileged and side-effect-free.
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// 10 ms of PIT channel 2, in TSC ticks; None if the PIT never answered.
fn pit_10ms() -> Option<u64> {
    const LATCH: u16 = 11_932; // 1_193_182 Hz / 100
    // SAFETY: the 8254 PIT and the port-0x61 gate, the standard calibration sequence;
    // the speaker stays off (bit 1 cleared) and port 0x61 is restored afterwards.
    unsafe {
        let p61 = inb(0x61);
        outb(0x61, (p61 & !0x02) | 0x01); // gate channel 2 on, speaker off
        outb(0x43, 0xB0); //                 channel 2, lo/hi byte, mode 0 (count down once)
        outb(0x42, LATCH as u8);
        outb(0x42, (LATCH >> 8) as u8);
        let t0 = rdtsc();
        let mut spins = 0u64;
        while inb(0x61) & 0x20 == 0 {
            spins += 1;
            if spins > PIT_SPIN_CAP || rdtsc().wrapping_sub(t0) > CAL_TSC_MAX {
                outb(0x61, p61);
                return None;
            }
        }
        let t1 = rdtsc();
        outb(0x61, p61);
        Some(t1.wrapping_sub(t0))
    }
}

/// The TSC rate the CPU itself reports: CPUID 0x15 (TSC / crystal ratio and the crystal's
/// frequency, Intel Skylake and later), else CPUID 0x16 (base clock in MHz). None if neither.
fn cpuid_tsc_hz() -> Option<u64> {
    #[allow(unused_unsafe)]
    // SAFETY: CPUID is available on every x86-64 CPU; leaves past the maximum read as 0.
    let (max, l15, l16) = unsafe {
        let max = core::arch::x86_64::__cpuid(0).eax;
        (max, core::arch::x86_64::__cpuid(0x15), core::arch::x86_64::__cpuid(0x16))
    };
    if max >= 0x15 && l15.eax != 0 && l15.ebx != 0 && l15.ecx != 0 {
        return Some(l15.ecx as u64 * l15.ebx as u64 / l15.eax as u64);
    }
    if max >= 0x16 && l16.eax & 0xFFFF != 0 {
        return Some((l16.eax & 0xFFFF) as u64 * 1_000_000);
    }
    None
}

/// 10 ms of the ACPI PM timer (3.579545 MHz, at least 24 bits) in TSC ticks, scaled to Hz.
pub fn pm_timer_hz() -> Option<u64> {
    let port = crate::acpi::pm_timer_port();
    if port == 0 {
        return None;
    }
    let read = || -> u32 {
        let v: u32;
        // SAFETY: a read of the ACPI PM timer's I/O port; no side effects.
        unsafe { core::arch::asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags)) };
        v & 0xFF_FFFF
    };
    const TICKS: u32 = 35_795; // 10 ms at 3.579545 MHz
    let p0 = read();
    let t0 = rdtsc();
    let mut spins = 0u64;
    while read().wrapping_sub(p0) & 0xFF_FFFF < TICKS {
        spins += 1;
        if spins > PMT_SPIN_CAP || rdtsc().wrapping_sub(t0) > CAL_TSC_MAX {
            return None; // the timer does not count
        }
    }
    Some(rdtsc().wrapping_sub(t0) * 100)
}

/// Measure the TSC frequency (the shortest of three 10 ms windows: the one least disturbed
/// by the host). Called once at boot, before the shell; also on first use.
pub fn init() -> u64 {
    let _ = T0.compare_exchange(0, rdtsc(), Ordering::Relaxed, Ordering::Relaxed);
    let mut best = u64::MAX;
    for _ in 0..CAL_WINDOWS {
        if let Some(t) = pit_10ms() {
            best = best.min(t);
        }
    }
    let (hz, src) = if best != u64::MAX && best >= 10_000 {
        (best * 100, 1)
    } else if let Some(h) = cpuid_tsc_hz() {
        (h, 2) // no PIT (some recent PCs gate it off): the CPU's own statement of its TSC rate
    } else if let Some(h) = pm_timer_hz() {
        (h, 3) // nor CPUID: measure against the ACPI power-management timer
    } else {
        (1_000_000_000, 4) // nothing answered: assume 1 GHz rather than divide by zero
    };
    HZ.store(hz, Ordering::Relaxed);
    SOURCE.store(src, Ordering::Relaxed);
    hz
}

/// The name of the source `hz()` was measured against, for the boot's `[timer]` line.
pub fn source_name() -> &'static str {
    match SOURCE.load(Ordering::Relaxed) {
        1 => "the 8254 PIT",
        2 => "CPUID (the TSC rate the CPU states)",
        3 => "the ACPI PM timer",
        4 => "nothing (assumed 1 GHz)",
        _ => "not set up",
    }
}

/// True when no source answered and `hz()` is the assumed 1 GHz.
pub fn rate_assumed() -> bool {
    SOURCE.load(Ordering::Relaxed) == 4
}

/// The boot's `[timer]` lines (CLOCK_PLAN 2 and the architect's verdict, point 4): the rate and its source, whether
/// the TSC is invariant, and a named line when the rate was assumed. The field report carries them (the boot log).
pub fn report() {
    crate::kprintln!(
        "[timer] TSC {} MHz, measured against {}; invariant TSC: {}",
        hz() / 1_000_000,
        source_name(),
        if crate::rng::has_invariant_tsc() { "yes" } else { "no (the clock never goes backwards, but its rate may drift with CPU power states)" }
    );
    if rate_assumed() {
        crate::kprintln!("[timer] time source rate assumed, not measured");
    }
}

/// Call 79 `clock_monotonic` (AFR-0001 7.6; CLOCK_PLAN): nanoseconds since the timer was set up at boot, rounded
/// down to a whole microsecond for every process (the coarse-timer rule), never below a value already given out.
pub fn monotonic_ns() -> u64 {
    let ticks = rdtsc().saturating_sub(t0()); // a core whose TSC reads behind T0 counts as 0, not as 2^64
    let ns = (ticks as u128 * 1_000_000_000 / hz().max(1) as u128).min(u64::MAX as u128) as u64;
    let us_ns = ns - ns % 1_000;
    let last = LAST_NS.fetch_max(us_ns, Ordering::Relaxed);
    last.max(us_ns)
}

/// TSC value when the timer was set up (the PC's "boot" for tick counting).
pub fn t0() -> u64 {
    T0.load(Ordering::Relaxed)
}

pub fn cycles() -> u64 {
    rdtsc()
}

pub fn hz() -> u64 {
    match HZ.load(Ordering::Relaxed) {
        0 => init(),
        h => h,
    }
}

/// A deadline `us` microseconds from now in `cycles()` units, or None while the counter's rate is not known yet
/// (the time-waits rule, the architect's sweep of 15 Nov 2026: a wait that stands for a time is bound by the
/// counter, and its spin count is only the fallback for when there is no rate).
pub fn deadline_us(us: u64) -> Option<u64> {
    let hz = HZ.load(Ordering::Relaxed);
    (hz != 0).then(|| cycles().wrapping_add(hz.saturating_mul(us) / 1_000_000))
}

/// True once `deadline` (from `deadline_us`) has passed; never true without one.
pub fn past(deadline: Option<u64>) -> bool {
    deadline.is_some_and(|d| cycles().wrapping_sub(d) < 1 << 62)
}

/// Block for ~`ms` milliseconds in 10 ms scheduler ticks (the ARM contract); returns the
/// ticks that passed.
pub fn sleep_ms(ms: u64) -> u64 {
    let start = crate::sched::ticks();
    let want = (ms + 9) / 10;
    let until = deadline_us((ms.saturating_mul(2) + SLEEP_SLACK_MS).saturating_mul(1000));
    let mut guard = 0u64;
    loop {
        let elapsed = crate::sched::ticks().wrapping_sub(start);
        if elapsed >= want {
            return elapsed;
        }
        crate::cpu::wfi();
        guard += 1;
        if past(until) || guard > SLEEP_WFI_CAP {
            return crate::sched::ticks().wrapping_sub(start);
        }
    }
}

/// Spin (no sleeping) for ~`ms` milliseconds on the TSC; returns the milliseconds spun.
pub fn busywait_ms(ms: u64) -> u64 {
    let freq = hz();
    let start = rdtsc();
    let want = freq.saturating_mul(ms) / 1000;
    loop {
        let elapsed = rdtsc().wrapping_sub(start);
        if elapsed >= want {
            return elapsed.saturating_mul(1000) / freq;
        }
        core::hint::spin_loop();
    }
}
