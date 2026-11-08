//! ARM generic timer — the scheduler's heartbeat.
//!
//! Uses the EL1 physical timer (CNTP). On QEMU `virt` its interrupt is private
//! peripheral interrupt PPI 14 → GIC INTID 30. We arm it to fire at a fixed
//! tick rate; each fire drives one round of the scheduler.

use crate::gic;
use core::arch::asm;

/// GIC INTID of the EL1 physical timer on QEMU virt (PPI 14 = INTID 16+14).
pub const TIMER_INTID: u32 = 30;

/// Scheduler tick rate.
const TICK_HZ: u64 = 100; // 10 ms slices

#[inline]
fn read_cntfrq() -> u64 {
    let f: u64;
    // SAFETY: reading the timer frequency register is side-effect-free.
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
    f
}

#[inline]
fn read_cntpct() -> u64 {
    let c: u64;
    // SAFETY: reading the physical count register is side-effect-free. It counts
    // continuously at `cntfrq` Hz regardless of interrupt state.
    unsafe { asm!("mrs {}, cntpct_el0", out(reg) c, options(nomem, nostack)) };
    c
}

/// Program the countdown for the next tick and (re)enable the timer.
pub fn rearm() {
    let interval = read_cntfrq() / TICK_HZ;
    // SAFETY: writing the EL1 physical timer registers at EL1.
    unsafe {
        // TVAL counts down; when it reaches 0 the interrupt is raised.
        asm!("msr cntp_tval_el0, {}", in(reg) interval, options(nomem, nostack));
        // CTL: bit0 ENABLE = 1, bit1 IMASK = 0 (unmasked).
        asm!("msr cntp_ctl_el0, {}", in(reg) 1u64, options(nomem, nostack));
    }
}

/// Configure the GIC to deliver the timer interrupt and arm the first tick.
pub fn init() {
    gic::enable(TIMER_INTID);
    rearm();
}

/// Raw value of the always-running physical counter (CNTPCT_EL0) -- a monotonic,
/// high-resolution tick source for micro-benchmarks and precise timing.
pub fn cycles() -> u64 {
    read_cntpct()
}

/// Frequency of the physical counter (CNTFRQ_EL0), in Hz.
pub fn hz() -> u64 {
    read_cntfrq()
}

/// Block for approximately `ms` milliseconds using the interrupt-driven timer
/// tick (100 Hz = 10 ms/tick). Sleeps on `wfi` so the CPU idles between ticks,
/// and returns the number of ticks actually elapsed. Bounded so it can never
/// hang even if ticks somehow stop advancing (a fallback spin cap trips first).
pub fn sleep_ms(ms: u64) -> u64 {
    let start = crate::sched::ticks();
    let want = (ms + 9) / 10; // ticks, rounded up
    let mut guard = 0u64;
    loop {
        let elapsed = crate::sched::ticks().wrapping_sub(start);
        if elapsed >= want {
            return elapsed;
        }
        crate::cpu::wfi();
        // Fallback: bound the loop so a stalled timer can't wedge the caller.
        guard += 1;
        if guard > 200_000_000 {
            return crate::sched::ticks().wrapping_sub(start);
        }
    }
}

/// Block for approximately `ms` milliseconds by polling the always-running
/// physical counter (`CNTPCT_EL0`), independent of interrupts. Unlike
/// [`sleep_ms`] (which sleeps on `wfi` and needs the timer IRQ to advance the
/// scheduler tick), this is safe to call at EL1 with IRQs masked -- e.g. inside
/// an SVC syscall handler, where a tick-driven wait could never progress.
/// Returns the measured elapsed milliseconds. Bounded two ways: the counter
/// always advances (so the wait terminates), and a guard cap trips as a
/// belt-and-braces backstop.
pub fn busywait_ms(ms: u64) -> u64 {
    let freq = read_cntfrq();
    if freq == 0 {
        return 0;
    }
    let start = read_cntpct();
    let want = freq.saturating_mul(ms) / 1000; // counter ticks to wait
    let mut guard: u64 = 0;
    loop {
        let elapsed = read_cntpct().wrapping_sub(start);
        if elapsed >= want {
            return elapsed.saturating_mul(1000) / freq;
        }
        core::hint::spin_loop();
        guard += 1;
        if guard > 5_000_000_000 {
            let e = read_cntpct().wrapping_sub(start);
            return e.saturating_mul(1000) / freq;
        }
    }
}
