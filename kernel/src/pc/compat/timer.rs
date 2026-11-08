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
            if spins > 20_000_000 {
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
        if spins > 50_000_000 {
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
    for _ in 0..3 {
        if let Some(t) = pit_10ms() {
            best = best.min(t);
        }
    }
    let hz = if best != u64::MAX && best >= 10_000 {
        best * 100
    } else if let Some(h) = cpuid_tsc_hz() {
        h // no PIT (some recent PCs gate it off): the CPU's own statement of its TSC rate
    } else if let Some(h) = pm_timer_hz() {
        h // nor CPUID: measure against the ACPI power-management timer
    } else {
        1_000_000_000 // nothing answered: assume 1 GHz rather than divide by zero
    };
    HZ.store(hz, Ordering::Relaxed);
    hz
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

/// Block for ~`ms` milliseconds in 10 ms scheduler ticks (the ARM contract); returns the
/// ticks that passed.
pub fn sleep_ms(ms: u64) -> u64 {
    let start = crate::sched::ticks();
    let want = (ms + 9) / 10;
    let mut guard = 0u64;
    loop {
        let elapsed = crate::sched::ticks().wrapping_sub(start);
        if elapsed >= want {
            return elapsed;
        }
        crate::cpu::wfi();
        guard += 1;
        if guard > 200_000_000 {
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
