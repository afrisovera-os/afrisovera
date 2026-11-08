//! Threads and a preemptive round-robin scheduler.
//!
//! Each thread has its own kernel stack and a saved stack pointer. Switching is
//! the classic callee-saved swap: `cpu_switch` pushes x19–x30 onto the outgoing
//! thread's stack, stores its SP, loads the incoming thread's SP, and pops. A
//! brand-new thread's stack is pre-forged so the first switch "returns" into a
//! trampoline that enables IRQs and calls the thread entry.
//!
//! Preemption: the timer IRQ calls [`tick`] → [`schedule`], which runs entirely
//! with IRQs masked (we are inside the IRQ handler) on this single core, so the
//! `static mut` scheduler state has exactly one accessor at a time.

use crate::heap;
use core::arch::global_asm;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicU64, Ordering};

const MAX_THREADS: usize = 8;
/// 16 KiB per thread — declared once, in the allocator that pays for it, so `heap` can
/// state its remaining room in threads rather than in bytes (2.735).
const STACK_SIZE: usize = crate::heap::STACK_SIZE;
/// Why a spawn was refused. The thread table is this module's own bound; a heap refusal
/// arrives already named by `heap::alloc_stack`.
pub const BOUND_TABLE: &str = "the thread table is full (sched::MAX_THREADS)";
/// 12 callee-saved slots (x19–x30) the initial fake frame must supply.
const FRAME_SLOTS: usize = 12;

#[derive(Clone, Copy)]
struct Thread {
    /// Saved stack pointer of a parked thread (0 = "currently running").
    sp: u64,
}

struct Scheduler {
    threads: [Thread; MAX_THREADS],
    count: usize,
    current: usize,
}

static mut SCHED: Scheduler = Scheduler {
    threads: [Thread { sp: 0 }; MAX_THREADS],
    count: 0,
    current: 0,
};

/// Ticks elapsed since scheduling began (also drives per-thread heartbeats).
static TICKS: AtomicU64 = AtomicU64::new(0);

// --- Per-core scheduler for the secondary CPU (core 1, MMU-on via PSCI) -------
//
// Core 1 gets its OWN scheduler instance and its OWN static thread stacks. It is
// touched only by core 1 (during setup, then only from core 1's IRQ handler with
// IRQs masked), so — exactly like the single-core invariant on `SCHED` — there is
// one accessor at a time and no cross-core lock is needed. Static stacks avoid
// any concurrent use of the heap allocator with the primary.
#[cfg(not(feature = "board-raspi"))]
static mut SCHED1: Scheduler = Scheduler {
    threads: [Thread { sp: 0 }; MAX_THREADS],
    count: 0,
    current: 0,
};

#[cfg(not(feature = "board-raspi"))]
const C1_MAX_THREADS: usize = 4;
#[cfg(not(feature = "board-raspi"))]
static mut C1_STACKS: [[u8; STACK_SIZE]; C1_MAX_THREADS] =
    [[0u8; STACK_SIZE]; C1_MAX_THREADS];
#[cfg(not(feature = "board-raspi"))]
static mut C1_NEXT_STACK: usize = 0;

extern "C" {
    /// Swap callee-saved state: save the current context to `*prev_sp`, then
    /// resume the context whose saved SP is `next_sp`.
    fn cpu_switch(prev_sp: *mut u64, next_sp: u64);
    /// New-thread landing pad (see the asm below). Never called from Rust.
    fn thread_trampoline();
}

/// Register the *currently executing* boot context as thread 0 (the idle
/// thread). Its SP is captured lazily on the first switch away from it.
///
/// Retained from the Milestone 0.2 EL1-thread demo; Phase 0.3 drives scheduling
/// cooperatively from `process.rs` instead, so this is currently unused.
#[allow(dead_code)]
pub fn init_idle() {
    // SAFETY: called once at boot before IRQs are enabled — no concurrency.
    unsafe {
        let s = &mut *addr_of_mut!(SCHED);
        s.threads[0] = Thread { sp: 0 };
        s.count = 1;
        s.current = 0;
    }
}

/// Spawn a kernel (EL1) thread that runs `entry(arg)`, or refuse **by name**.
///
/// Retained from Milestone 0.2; unused in Phase 0.3 (see `init_idle`).
///
/// 2.735: this used to answer a null stack with `assert!(!stack.is_null(), "out of heap
/// for thread stack")` — the arena filling up killed the machine rather than the one
/// spawn that asked for the stack that was not there, and the panic message named the
/// caller rather than the bound that fired. Both of this function's ceilings are now
/// returned to the caller, which is the only place that knows whether one fewer thread
/// is survivable.
#[allow(dead_code)]
#[must_use = "a refused spawn produced no thread — report the bound"]
pub fn spawn(entry: extern "C" fn(usize) -> !, arg: usize) -> Result<(), &'static str> {
    // Checked BEFORE the stack is taken: a spawn that cannot be recorded must not first
    // consume 16 KiB of an arena that never gives anything back.
    // SAFETY: read-only peek at SCHED on the primary core (single accessor invariant).
    if unsafe { (*addr_of_mut!(SCHED)).count } >= MAX_THREADS {
        return Err(BOUND_TABLE);
    }
    // An aligned stack from the kernel heap, leaked (it lives as long as the thread).
    let stack = heap::alloc_stack()?;

    // Forge the initial callee-saved frame `cpu_switch` will pop on first run.
    // Layout (low→high) matches cpu_switch's push order:
    //   [0]=x29 [1]=x30 [2]=x27 [3]=x28 [4]=x25 [5]=x26
    //   [6]=x23 [7]=x24 [8]=x21 [9]=x22 [10]=x19 [11]=x20
    let top = stack as usize + STACK_SIZE;
    let frame = (top - FRAME_SLOTS * 8) as *mut u64;
    // SAFETY: `frame` points inside the freshly allocated, aligned stack.
    unsafe {
        for i in 0..FRAME_SLOTS {
            frame.add(i).write(0);
        }
        frame.add(1).write(thread_trampoline as *const () as u64); // x30 → trampoline
        frame.add(10).write(entry as usize as u64); // x19 → entry fn
        frame.add(11).write(arg as u64); // x20 → arg

        let s = &mut *addr_of_mut!(SCHED);
        // Re-checked under the same single-accessor invariant the peek above used; the
        // bound cannot have moved in between (no preemption spawns threads).
        if s.count >= MAX_THREADS {
            return Err(BOUND_TABLE);
        }
        s.threads[s.count] = Thread { sp: frame as u64 };
        s.count += 1;
    }
    Ok(())
}

/// Timer-driven scheduler entry: count the tick and rotate threads.
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: single accessor of SCHED (primary core, IRQs masked in-handler).
    unsafe { schedule_on(addr_of_mut!(SCHED)) };
}

/// Timer-driven scheduler entry for the **secondary** core (core 1). Rotates
/// core 1's own thread set on `SCHED1`; never touches the primary's `SCHED`. On
/// boards with no secondary (Pi) this is an inert stub the IRQ path never calls.
pub fn tick_secondary() {
    #[cfg(not(feature = "board-raspi"))]
    // SAFETY: single accessor of SCHED1 (core 1, IRQs masked in-handler).
    unsafe {
        schedule_on(addr_of_mut!(SCHED1))
    };
}

/// Round-robin over one scheduler instance: switch from current to next.
///
/// SAFETY: the caller guarantees exactly one accessor of `*s` at a time (the
/// per-core, IRQs-masked invariant). `s` must point at a valid `Scheduler`.
unsafe fn schedule_on(s: *mut Scheduler) {
    let s = &mut *s;
    if s.count < 2 {
        return;
    }
    let prev = s.current;
    let next = (s.current + 1) % s.count;
    if prev == next {
        return;
    }
    s.current = next;
    let prev_sp = addr_of_mut!(s.threads[prev].sp);
    let next_sp = s.threads[next].sp;
    cpu_switch(prev_sp, next_sp);
}

/// Register core 1's currently-executing context as its idle thread (thread 0).
/// Its SP is captured on the first switch away, exactly like `init_idle`.
#[cfg(not(feature = "board-raspi"))]
pub fn register_current_secondary() {
    // SAFETY: called once on core 1 before it unmasks IRQs — no concurrency.
    unsafe {
        let s = &mut *addr_of_mut!(SCHED1);
        s.threads[0] = Thread { sp: 0 };
        s.count = 1;
        s.current = 0;
    }
}

/// Spawn a kernel thread on **core 1** using a static (non-heap) stack, so the
/// secondary never races the primary on the allocator.
#[cfg(not(feature = "board-raspi"))]
pub fn spawn_secondary(entry: extern "C" fn(usize) -> !, arg: usize) {
    // SAFETY: called only on core 1 during setup (before IRQs unmasked). The
    // static stacks are used by exactly one thread each.
    unsafe {
        let idx = C1_NEXT_STACK;
        assert!(idx < C1_MAX_THREADS, "core 1 stack pool exhausted");
        C1_NEXT_STACK += 1;

        let stack = addr_of_mut!(C1_STACKS[idx]) as *mut u8;
        let top = stack as usize + STACK_SIZE;
        let frame = (top - FRAME_SLOTS * 8) as *mut u64;
        for i in 0..FRAME_SLOTS {
            frame.add(i).write(0);
        }
        frame.add(1).write(thread_trampoline as *const () as u64); // x30 → trampoline
        frame.add(10).write(entry as usize as u64); // x19 → entry fn
        frame.add(11).write(arg as u64); // x20 → arg

        let s = &mut *addr_of_mut!(SCHED1);
        assert!(s.count < MAX_THREADS, "core 1 thread table full");
        s.threads[s.count] = Thread { sp: frame as u64 };
        s.count += 1;
    }
}

/// Current tick count (per-thread heartbeat source). Retained from 0.2.
#[allow(dead_code)]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Context switch + new-thread trampoline.
//
// cpu_switch(x0 = &mut prev.sp, x1 = next.sp):
//   push callee-saved onto current stack, store SP to *x0, load SP from x1,
//   pop callee-saved, ret — resuming wherever the incoming thread last stopped.
//
// thread_trampoline: reached only the *first* time a new thread runs. It unmasks
// IRQs (so the thread is itself preemptible) then calls entry(arg) via the
// x19/x20 the forged frame supplied.
// ---------------------------------------------------------------------------
global_asm!(
    r#"
.section .text
.global cpu_switch
cpu_switch:
    stp x19, x20, [sp, #-16]!
    stp x21, x22, [sp, #-16]!
    stp x23, x24, [sp, #-16]!
    stp x25, x26, [sp, #-16]!
    stp x27, x28, [sp, #-16]!
    stp x29, x30, [sp, #-16]!
    mov x2, sp
    str x2, [x0]          // *prev_sp = current SP
    mov sp, x1            // SP = next thread's saved SP
    ldp x29, x30, [sp], #16
    ldp x27, x28, [sp], #16
    ldp x25, x26, [sp], #16
    ldp x23, x24, [sp], #16
    ldp x21, x22, [sp], #16
    ldp x19, x20, [sp], #16
    ret

.global thread_trampoline
thread_trampoline:
    msr daifclr, #2      // enable IRQs so this thread can be preempted
    mov x0, x20          // arg
    blr x19              // entry(arg)
1:  wfe                  // entry is `-> !`; if it ever returns, park
    b   1b
"#
);
