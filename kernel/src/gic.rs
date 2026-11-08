//! GICv2 interrupt controller (QEMU `virt`: distributor @ 0x0800_0000, CPU interface
//! @ 0x0801_0000 — but those numbers are the *board map's* claim, not this file's).
//!
//! Phase 0.2 only needs to route one private peripheral interrupt (the EL1 physical
//! timer, INTID 30) to this core, so the setup is minimal.
//!
//! **Milestone 2.727: the bases are resolved, not compiled in.** Until now this file
//! derived `GICD_BASE`/`GICC_BASE` as `const`s from [`crate::board::ACTIVE`], so
//! milestone 2.726's device-tree override reached the RTC readers and the device
//! listing but stopped at the interrupt controller — the one peripheral whose wrong
//! address costs you every interrupt on the machine. Both windows now come from
//! [`crate::board::resolved`] **at use time**, which is what makes a live tree able
//! to move them, and which imposes an ordering: `board::resolve()` must run before
//! any GIC access, on *every* core.
//!
//! That ordering is proved rather than asserted. Every access goes through [`gicd`]
//! or [`gicc`], which count themselves and count separately any access made while
//! [`crate::board::is_resolved`] is still false; [`describe_bases`] (`gicbase`)
//! prints the tally with a verdict. A regression that moved `gic::init()` above
//! `board::resolve()` — or brought a secondary core up on a path that skipped
//! resolution — would show as a non-zero "before resolve" count instead of as
//! silence on a machine whose GIC is not where the map says.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};

// --- the ordering audit ------------------------------------------------------
// Counters, not asserts: this file cannot panic usefully on a mis-ordered boot (an
// interrupt controller at the wrong address has no way to report anything), so it
// records what happened and lets `gicbase` state the verdict.
static USES: AtomicU32 = AtomicU32::new(0);
static USES_EARLY: AtomicU32 = AtomicU32::new(0);
static SECONDARY_INITS: AtomicU32 = AtomicU32::new(0);
static SECONDARY_EARLY: AtomicU32 = AtomicU32::new(0);

/// Count one GIC MMIO access, and whether it happened before the map was resolved.
#[inline]
fn note_use() {
    USES.fetch_add(1, Ordering::Relaxed);
    if !crate::board::is_resolved() {
        USES_EARLY.fetch_add(1, Ordering::Relaxed);
    }
}

/// Base of the distributor **in use** — the map, or whatever a live tree corrected.
#[inline]
pub fn dist_base() -> usize {
    note_use();
    crate::board::resolved().gic_dist
}

/// Base of this core's CPU interface, from the same resolution as [`dist_base`].
/// The two are always taken from one source (board.rs rule 5), so a caller can never
/// end up talking to one machine's distributor through another machine's CPU
/// interface.
#[inline]
pub fn cpu_base() -> usize {
    note_use();
    crate::board::resolved().gic_cpu
}

// Distributor register offsets (from `dist_base()`).
const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100; // one bit per INTID
const GICD_IPRIORITYR: usize = 0x400; // one byte per INTID
const GICD_SGIR: usize = 0xF00; // software-generated interrupt trigger

// CPU interface register offsets (from `cpu_base()`).
const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_IAR: usize = 0x00C;
const GICC_EOIR: usize = 0x010;

#[inline]
fn mmio_w(addr: usize, val: u32) {
    // SAFETY: a GIC MMIO address resolved from the board map or the machine's own
    // device tree; both are device memory in the identity map.
    unsafe { write_volatile(addr as *mut u32, val) };
}

#[inline]
fn mmio_r(addr: usize) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

/// Initialize the distributor and this CPU's interface.
pub fn init() {
    // Enable the distributor.
    mmio_w(dist_base() + GICD_CTLR, 1);
    // Allow all interrupt priorities through the CPU interface (0xFF = lowest
    // threshold → nothing masked by priority).
    let gicc = cpu_base();
    mmio_w(gicc + GICC_PMR, 0xFF);
    // Enable signalling of interrupts from the CPU interface to this core.
    mmio_w(gicc + GICC_CTLR, 1);
}

/// Initialize a **secondary** core's CPU interface. The distributor (GICD) is
/// shared and already enabled by the primary, so we only bring up this core's
/// own banked CPU interface (GICC). Private peripheral interrupts (INTID < 32,
/// e.g. the timer's 30) are banked per-core, so enabling them still happens per
/// core via [`enable`].
pub fn init_secondary() {
    // Counted on its own: a secondary that came up before `board::resolve()` would be
    // running its interrupts through an unresolved CPU-interface base while the
    // primary used the resolved one — the exact split-brain rule 5 forbids *within* a
    // core, happening *between* cores instead.
    SECONDARY_INITS.fetch_add(1, Ordering::Relaxed);
    if !crate::board::is_resolved() {
        SECONDARY_EARLY.fetch_add(1, Ordering::Relaxed);
    }
    let gicc = cpu_base();
    mmio_w(gicc + GICC_PMR, 0xFF); // this core's priority mask
    mmio_w(gicc + GICC_CTLR, 1); // signal interrupts to this core
}

/// Enable a specific interrupt ID and give it a priority.
pub fn enable(intid: u32) {
    let gicd = dist_base();
    let reg = gicd + GICD_ISENABLER + (intid as usize / 32) * 4;
    mmio_w(reg, 1 << (intid % 32));
    // Priority 0 = highest. Byte-addressed priority register.
    let prio = gicd + GICD_IPRIORITYR + intid as usize;
    // SAFETY: byte MMIO write within the priority register file.
    unsafe { write_volatile(prio as *mut u8, 0x00) };
}

/// Send a software-generated interrupt (SGI, INTID 0-15) to another core via the
/// distributor. `target_core` is a physical core id; the distributor takes a CPU
/// bitmask, so bit `target_core` is set. SGIs are the GIC's inter-processor
/// signalling mechanism (cross-core wakeups, IPIs, TLB-shootdown kicks).
pub fn send_sgi(target_core: u32, intid: u32) {
    let target_list = 1u32 << target_core;
    mmio_w(dist_base() + GICD_SGIR, (target_list << 16) | (intid & 0xF));
}

/// Acknowledge the pending interrupt; returns the raw IAR (INTID in low bits).
pub fn acknowledge() -> u32 {
    mmio_r(cpu_base() + GICC_IAR)
}

/// Signal end-of-interrupt for a value previously returned by [`acknowledge`].
pub fn end_of_interrupt(iar: u32) {
    mmio_w(cpu_base() + GICC_EOIR, iar);
}

/// Print the bases this driver is actually using, their provenance, and the ordering
/// audit (the `gicbase` shell command).
///
/// The interesting line is the last one. Every other milestone in this series could
/// be checked by reading a printed address; "the driver read the resolved value
/// *late enough*" cannot — it is a fact about boot order, and the only honest way to
/// report it is to have counted.
pub fn describe_bases() {
    let r = crate::board::resolved();
    let (uses, early) = (USES.load(Ordering::Relaxed), USES_EARLY.load(Ordering::Relaxed));
    let (sec, sec_early) =
        (SECONDARY_INITS.load(Ordering::Relaxed), SECONDARY_EARLY.load(Ordering::Relaxed));
    crate::kprintln!("gicbase: GICv2 bases in use by gic.rs (resolved, not compiled in)");
    crate::kprintln!(
        "  distributor      {:#010x}   map {:#010x}   {}",
        r.gic_dist,
        crate::board::GIC_DIST_BASE,
        r.gic_dist_src.as_str()
    );
    crate::kprintln!(
        "  cpu interface    {:#010x}   map {:#010x}   {}",
        r.gic_cpu,
        crate::board::GIC_CPU_BASE,
        r.gic_cpu_src.as_str()
    );
    if r.gic_partial {
        crate::kprintln!("  ! partial tree discovery — the pair stayed on the map (rule 5)");
    }
    crate::kprintln!("  board::resolve() ran = {}", if crate::board::is_resolved() { "yes" } else { "no" });
    crate::kprintln!("  accesses {} ({} before resolve)", uses, early);
    crate::kprintln!("  secondary cpu-iface inits {} ({} before resolve)", sec, sec_early);
    if early == 0 && sec_early == 0 && crate::board::is_resolved() {
        crate::kprintln!("  => ORDER OK: every GIC access, on every core, used a resolved base");
    } else {
        crate::kprintln!(
            "  => ORDER VIOLATION: {} access(es) and {} secondary init(s) ran on an",
            early,
            sec_early
        );
        crate::kprintln!("     unresolved base — board::resolve() must precede gic::init()");
    }
}
