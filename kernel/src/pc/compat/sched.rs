//! PC stand-in for the ARM `sched.rs`: only its heartbeat, `ticks()`, in 10 ms ticks since
//! the timer was set up at boot. ARM counts 100 Hz timer interrupts; the PC derives the
//! same unit from the calibrated TSC, so `uptime` and `sleep` mean the same thing.
#![allow(dead_code)]

pub fn ticks() -> u64 {
    let per_tick = (crate::timer::hz() / 100).max(1);
    crate::timer::rdtsc().wrapping_sub(crate::timer::t0()) / per_tick
}
