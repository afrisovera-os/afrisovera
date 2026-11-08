//! Low-level AArch64 CPU primitives: system-register access, barriers, and
//! interrupt masking. Everything the rest of the kernel needs to talk to the
//! bare metal, kept in one small, auditable place.

use core::arch::asm;

/// Current Exception Level (1, 2, or 3). Read from `CurrentEL[3:2]`.
#[inline]
pub fn current_el() -> u64 {
    let el: u64;
    // SAFETY: `mrs CurrentEL` is always readable and has no side effects.
    unsafe { asm!("mrs {}, CurrentEL", out(reg) el, options(nomem, nostack)) };
    el >> 2
}

/// This core's id — affinity level 0 of `MPIDR_EL1` (0 = primary, 1 = the
/// secondary brought up via PSCI on QEMU virt).
#[inline]
pub fn core_id() -> u64 {
    let mpidr: u64;
    // SAFETY: `mrs MPIDR_EL1` is always readable and side-effect-free.
    unsafe { asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)) };
    mpidr & 0xFF
}

/// Data Synchronization Barrier (full system).
#[inline]
pub fn dsb() {
    unsafe { asm!("dsb sy", options(nomem, nostack)) };
}

/// Instruction Synchronization Barrier.
#[inline]
pub fn isb() {
    unsafe { asm!("isb", options(nomem, nostack)) };
}

/// Enable EL1 access to the FP/SIMD unit (`CPACR_EL1.FPEN = 0b11`). Without this,
/// any floating-point/NEON instruction traps — and the compiler may emit NEON.
#[inline]
pub fn enable_fp_simd() {
    // SAFETY: writing CPACR_EL1 at EL1 is permitted; value only widens access.
    unsafe {
        asm!(
            "mrs {t}, cpacr_el1",
            "orr {t}, {t}, #(0b11 << 20)",
            "msr cpacr_el1, {t}",
            "isb",
            t = out(reg) _,
            options(nomem, nostack),
        );
    }
}

/// Disable IRQs and return the previous `DAIF` so it can be restored. Used to
/// make short critical sections atomic with respect to timer preemption.
#[inline]
pub fn irq_save() -> u64 {
    let daif: u64;
    // SAFETY: reading DAIF and masking IRQs is always safe.
    unsafe {
        asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        asm!("msr daifset, #2", options(nomem, nostack)); // set I bit
    }
    daif
}

/// Restore a `DAIF` value previously captured by [`irq_save`].
#[inline]
pub fn irq_restore(daif: u64) {
    // SAFETY: writing back a previously-read DAIF value.
    unsafe { asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
}

/// Unmask IRQs (clear the `DAIF.I` bit).
#[inline]
pub fn irq_enable() {
    // SAFETY: enabling interrupts once the vector table + GIC are configured.
    unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) };
}

/// Low-power wait until an interrupt (or event) arrives.
#[inline]
pub fn wfi() {
    unsafe { asm!("wfi", options(nomem, nostack)) };
}

/// Switch the low (user) translation base to `base` and flush the EL1 TLB, so
/// the next EL0 execution sees the new address space. Kernel + vectors stay
/// mapped because every process table also carries the EL1 device/DRAM blocks.
#[inline]
pub fn set_ttbr0(base: u64) {
    // SAFETY: `base` points at a valid, kernel-built L1 translation table.
    unsafe {
        asm!(
            "msr ttbr0_el1, {b}",
            "isb",
            "tlbi vmalle1",
            "dsb sy",
            "isb",
            b = in(reg) base,
            options(nostack),
        );
    }
}

/// Make freshly-written code at `[addr, addr+len)` visible to the instruction
/// stream: clean D-cache to PoU, then invalidate I-cache to PoU. Required after
/// copying a user program into memory before executing it.
pub fn sync_icache_range(addr: usize, len: usize) {
    let line = 64usize; // conservative cache-line size
    let end = addr + len;
    let mut p = addr & !(line - 1);
    while p < end {
        // SAFETY: cache-maintenance by VA over a mapped, identity region.
        unsafe { asm!("dc cvau, {}", in(reg) p, options(nostack)) };
        p += line;
    }
    unsafe { asm!("dsb ish", options(nostack)) };
    let mut p = addr & !(line - 1);
    while p < end {
        unsafe { asm!("ic ivau, {}", in(reg) p, options(nostack)) };
        p += line;
    }
    unsafe {
        asm!("dsb ish", options(nostack));
        asm!("isb", options(nostack));
    }
}
