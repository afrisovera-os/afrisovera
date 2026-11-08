//! Exception handling: the AArch64 vector table and the IRQ entry path.
//!
//! Threads run at EL1h (using SP_EL1), so all their exceptions arrive through
//! the "current EL, SPx" quadrant of the table. Only the IRQ vector does real
//! work in Phase 0.2 (timer → scheduler); synchronous faults print diagnostics
//! and halt. The IRQ path saves the full integer register frame on the current
//! thread's kernel stack, so a context switch *inside* the handler cleanly parks
//! the interrupted thread and resumes another.

use core::arch::{asm, global_asm};

/// Point VBAR_EL1 at our vector table.
pub fn init() {
    // SAFETY: `vector_table` is a valid, 2 KiB-aligned code symbol in .text.
    unsafe {
        asm!(
            "adr {t}, vector_table",
            "msr vbar_el1, {t}",
            "isb",
            t = out(reg) _,
            options(nostack),
        );
    }
}

// The Rust-side IRQ handler, called from `irq_el1h` with a full register frame
// already saved on the stack. It may call `sched::tick()`, which can context-
// switch to another thread and only return much later.
//
// The interrupt controller differs by board: QEMU virt has a GICv2, while the
// Raspberry Pi 3 (BCM2837) has no GIC — its per-core generic-timer interrupt
// routes through the BCM2836/2837 LOCAL interrupt controller at 0x4000_0000.
#[cfg(not(feature = "board-raspi"))]
#[no_mangle]
extern "C" fn rust_irq_handler() {
    use crate::{gic, sched, timer};
    let iar = gic::acknowledge();
    let intid = iar & 0x3FF;

    if intid == timer::TIMER_INTID {
        timer::rearm(); // schedule the next tick (per-core CNTP register)
        gic::end_of_interrupt(iar); // EOI *before* any context switch (banked GICC)
        // Each core drives its own scheduler: the primary rotates SCHED, the
        // secondary (core 1) rotates its isolated SCHED1. Neither touches the
        // other's state, so no cross-core lock is needed.
        if crate::cpu::core_id() == 0 {
            sched::tick(); // may switch away and resume a different thread
        } else {
            sched::tick_secondary();
        }
    } else if intid == crate::uart::UART_RX_INTID {
        // Console input: drain the PL011 RX FIFO into the input ring.
        crate::uart::rx_isr();
        gic::end_of_interrupt(iar);
    } else if intid < 16 {
        // Software-generated interrupt: a cross-core signal. Only core 1 is
        // targeted in this build; the SGI id selects wakeup vs IPC.
        crate::c1_on_sgi(intid);
        gic::end_of_interrupt(iar); // EOI the raw IAR (keeps SGI source bits)
    } else if intid < 1020 {
        // Spurious (1023) interrupts are ignored; anything else is EOI'd.
        gic::end_of_interrupt(iar);
    }
}

/// Raspberry Pi IRQ handler: read the BCM2836/2837 local interrupt controller's
/// Core0 IRQ-source register (0x4000_0060) instead of a GIC. Bit 1 is the
/// non-secure physical timer (CNTPNSIRQ); re-arming CNTP clears the timer's IRQ
/// line, which the level-sensitive local controller follows (no separate EOI).
#[cfg(feature = "board-raspi")]
#[no_mangle]
extern "C" fn rust_irq_handler() {
    use core::ptr::read_volatile;
    const CORE0_IRQ_SOURCE: *const u32 = 0x4000_0060 as *const u32;
    const CNTPNSIRQ: u32 = 1 << 1;
    // SAFETY: 0x4000_0060 is in the Device-mapped local-peripheral window.
    let src = unsafe { read_volatile(CORE0_IRQ_SOURCE) };
    if src & CNTPNSIRQ != 0 {
        crate::timer::rearm(); // next deadline; clears CNTP.ISTATUS → drops the IRQ
        crate::pi_timer_tick();
        // Preempt: round-robin the Pi kernel threads. Single core, so the global
        // SCHED has one accessor (this in-handler path, IRQs masked) — the same
        // invariant the virt build relies on. May context-switch and return much
        // later, exactly as on virt.
        crate::sched::tick();
    }
}

// Synchronous-exception reporter (unrecoverable in Phase 0.2 — halt loudly).
#[no_mangle]
extern "C" fn rust_sync_handler() -> ! {
    let (esr, elr, far): (u64, u64, u64);
    // SAFETY: reading fault syndrome registers.
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack));
        asm!("mrs {}, elr_el1", out(reg) elr, options(nomem, nostack));
        asm!("mrs {}, far_el1", out(reg) far, options(nomem, nostack));
    }
    crate::kprintln!(
        "[mansa] SYNC EXCEPTION  ESR={:#018x}  ELR={:#018x}  FAR={:#018x}",
        esr,
        elr,
        far
    );
    crate::kprintln!("[mansa] halting.");
    loop {
        crate::cpu::wfi();
    }
}

#[no_mangle]
extern "C" fn rust_unhandled() -> ! {
    crate::kprintln!("[mansa] unhandled exception; halting.");
    loop {
        crate::cpu::wfi();
    }
}

// ---------------------------------------------------------------------------
// The vector table. 16 entries × 0x80 bytes, the whole table 2 KiB-aligned.
// The IRQ-from-current-EL-SPx slot (offset 0x280) runs the timer path; the two
// synchronous slots report and halt; everything else funnels to `rust_unhandled`.
// ---------------------------------------------------------------------------
global_asm!(
    r#"
.section .text
.balign 0x800
.global vector_table
vector_table:
    // ---- Current EL with SP_EL0 (SP0) ----
    b   sync_el1t          // 0x000 Synchronous
    .balign 0x80
    b   unhandled_entry    // 0x080 IRQ
    .balign 0x80
    b   unhandled_entry    // 0x100 FIQ
    .balign 0x80
    b   unhandled_entry    // 0x180 SError
    .balign 0x80
    // ---- Current EL with SP_ELx (SPx) — where threads live ----
    b   sync_el1h          // 0x200 Synchronous
    .balign 0x80
    b   irq_el1h           // 0x280 IRQ  <-- timer
    .balign 0x80
    b   unhandled_entry    // 0x300 FIQ
    .balign 0x80
    b   unhandled_entry    // 0x380 SError
    .balign 0x80
    // ---- Lower EL, AArch64 (userspace) ----
    b   svc_entry          // 0x400 Synchronous  <-- SVC syscalls from EL0
    .balign 0x80
    b   irq_lower_entry    // 0x480 IRQ  <-- timer preemption of EL0 processes
    .balign 0x80
    b   unhandled_entry    // 0x500 FIQ
    .balign 0x80
    b   unhandled_entry    // 0x580 SError
    .balign 0x80
    // ---- Lower EL, AArch32 (unused) ----
    b   unhandled_entry    // 0x600
    .balign 0x80
    b   unhandled_entry    // 0x680
    .balign 0x80
    b   unhandled_entry    // 0x700
    .balign 0x80
    b   unhandled_entry    // 0x780

sync_el1t:
sync_el1h:
    bl  rust_sync_handler

unhandled_entry:
    bl  rust_unhandled

// IRQ from current EL (SPx): save the integer frame, dispatch, restore, return.
irq_el1h:
    sub sp, sp, #272
    stp x0,  x1,  [sp, #16*0]
    stp x2,  x3,  [sp, #16*1]
    stp x4,  x5,  [sp, #16*2]
    stp x6,  x7,  [sp, #16*3]
    stp x8,  x9,  [sp, #16*4]
    stp x10, x11, [sp, #16*5]
    stp x12, x13, [sp, #16*6]
    stp x14, x15, [sp, #16*7]
    stp x16, x17, [sp, #16*8]
    stp x18, x19, [sp, #16*9]
    stp x20, x21, [sp, #16*10]
    stp x22, x23, [sp, #16*11]
    stp x24, x25, [sp, #16*12]
    stp x26, x27, [sp, #16*13]
    stp x28, x29, [sp, #16*14]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x30, x0,  [sp, #16*15]   // x30 + ELR_EL1
    str x1,       [sp, #256]     // SPSR_EL1

    bl  rust_irq_handler

    ldr x1,       [sp, #256]
    ldp x30, x0,  [sp, #16*15]
    msr spsr_el1, x1
    msr elr_el1,  x0
    ldp x28, x29, [sp, #16*14]
    ldp x26, x27, [sp, #16*13]
    ldp x24, x25, [sp, #16*12]
    ldp x22, x23, [sp, #16*11]
    ldp x20, x21, [sp, #16*10]
    ldp x18, x19, [sp, #16*9]
    ldp x16, x17, [sp, #16*8]
    ldp x14, x15, [sp, #16*7]
    ldp x12, x13, [sp, #16*6]
    ldp x10, x11, [sp, #16*5]
    ldp x8,  x9,  [sp, #16*4]
    ldp x6,  x7,  [sp, #16*3]
    ldp x4,  x5,  [sp, #16*2]
    ldp x2,  x3,  [sp, #16*1]
    ldp x0,  x1,  [sp, #16*0]
    add sp, sp, #272
    eret
"#
);
