//! A second CPU core on the PC (PC track step 2: the ARM kernel's SMP demo, `main.rs`
//! `bring_up_secondary`, on x86-64).
//!
//! The ARM kernel wakes core 1 with one firmware call (PSCI CPU_ON). A PC core wakes the
//! x86 way: the boot core sends it INIT and two STARTUP inter-processor interrupts through
//! the local APIC, and the woken core starts in 16-bit real mode at a page below 1 MiB.
//! `ap_tramp` (below) is copied to such a page; it switches to protected mode, turns on PAE
//! and long mode on a copy of the kernel's top-level page table in the page below it (CR3 is
//! loaded in 32-bit mode, so it must be below 4 GiB, and the kernel's own table may not be:
//! on the HP ENVY the image, and so the frame pool, was loaded above 4 GiB, field item F3),
//! and jumps to `ap_main` on a stack of its own, which moves to the kernel's own table
//! (CR3 = `mmu::root()`). From there the demo is the ARM one, step for step:
//!   1. core 1 checks in ("[core 1] online", "SMP: 2 cores online");
//!   2. core 0 queues 20 jobs, core 1 drains them in parallel (checksum 3080);
//!   3. core 1 starts its own scheduler: two kernel threads switched by ITS local-APIC timer;
//!   4. core 0 sends it inter-processor interrupts (ARM: SGIs), which core 1 reports;
//!   5. cross-core IPC: core 0 posts a message, interrupts core 1, core 1 doubles it and
//!      answers, core 0 reads the answer.
//! Under the regression rig QEMU usually has one core: no core answers the STARTUP and the
//! boot says so and carries on single-core, as the ARM kernel does.

use crate::cpu::{self, TrapFrame};
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// The interrupt core 0 sends core 1 (the ARM demo's SGI 0).
pub const IPI_VECTOR: u64 = 0x40;

/// A free 4 KiB page below 1 MiB for the start-up code (from the UEFI memory map), or 0.
/// The page below it is free too: core 1's first top-level page table goes there.
pub static LOW_PAGE: AtomicU64 = AtomicU64::new(0);
/// The same from boot-services memory (free after exit), used only when LOW_PAGE found none.
pub static LOW_PAGE_BS: AtomicU64 = AtomicU64::new(0);

static UP: AtomicBool = AtomicBool::new(false);
static AP_ID: AtomicU32 = AtomicU32::new(u32::MAX);

const N_JOBS: u32 = 20;
static JOB_PUSHED: AtomicU32 = AtomicU32::new(0);
static JOB_TAKEN: AtomicU32 = AtomicU32::new(0);
static JOBS_DONE: AtomicU32 = AtomicU32::new(0);
static JOB_SUM: AtomicU64 = AtomicU64::new(0);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static REPORTED: AtomicBool = AtomicBool::new(false);
static CORE1_READY: AtomicBool = AtomicBool::new(false);
static IPIS: AtomicU32 = AtomicU32::new(0);
/// Core 0 -> core 1 message (0 = empty) and core 1's answer, (msg << 32) | result (0 = none).
static MAIL: AtomicU64 = AtomicU64::new(0);
static REPLY: AtomicU64 = AtomicU64::new(0);

#[repr(align(16))]
#[allow(dead_code)] // only their addresses are used
struct Stack([u8; 16 * 1024]);
static mut AP_STACK: Stack = Stack([0; 16 * 1024]);
static mut THREAD_B_STACK: Stack = Stack([0; 16 * 1024]);

// The start-up code. Copied to LOW_PAGE; a STARTUP IPI with vector LOW_PAGE >> 12 starts the
// core at LOW_PAGE:0 in real mode. Everything it needs is in the page: its own small GDT,
// and four fields `bring_up` fills in (the linear addresses of the GDT and of the two far-
// jump targets, which depend on where the page is, plus CR3, the stack and the entry point).
global_asm!(
    r#"
.global ap_tramp
.global ap_tramp_end
.global ap_gdt
.global ap_gdtptr
.global ap_pm32
.global ap_lm64
.global ap_far32
.global ap_far64
.global ap_cr3
.global ap_stack
.global ap_entry
.code16
ap_tramp:
    cli
    mov %cs, %ax
    mov %ax, %ds
    xor %ebx, %ebx
    mov %cs, %bx
    shl $4, %ebx
    lgdtl (ap_gdtptr - ap_tramp)
    mov %cr0, %eax
    or $1, %eax
    mov %eax, %cr0
    ljmpl *(ap_far32 - ap_tramp)
.code32
ap_pm32:
    mov $0x10, %ax
    mov %ax, %ds
    mov %ax, %es
    mov %ax, %ss
    mov %cr4, %eax
    or $0x20, %eax
    mov %eax, %cr4
    mov (ap_cr3 - ap_tramp)(%ebx), %eax
    mov %eax, %cr3
    mov $0xC0000080, %ecx
    rdmsr
    or $0x100, %eax
    wrmsr
    mov %cr0, %eax
    or $0x80000000, %eax
    mov %eax, %cr0
    ljmpl *(ap_far64 - ap_tramp)(%ebx)
.code64
ap_lm64:
    mov %ebx, %ebx
    mov (ap_stack - ap_tramp)(%rbx), %rsp
    mov (ap_entry - ap_tramp)(%rbx), %rax
    jmp *%rax
.balign 8
ap_gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF
    .quad 0x00CF92000000FFFF
    .quad 0x00AF9A000000FFFF
ap_gdtptr:
    .word 31
    .long 0
ap_far32:
    .long 0
    .word 0x08
ap_far64:
    .long 0
    .word 0x18
ap_cr3:
    .long 0
.balign 8
ap_stack:
    .quad 0
ap_entry:
    .quad 0
ap_tramp_end:
.text
"#,
    options(att_syntax)
);

extern "C" {
    static ap_tramp: u8;
    static ap_tramp_end: u8;
    static ap_gdt: u8;
    static ap_gdtptr: u8;
    static ap_pm32: u8;
    static ap_lm64: u8;
    static ap_far32: u8;
    static ap_far64: u8;
    static ap_cr3: u8;
    static ap_stack: u8;
    static ap_entry: u8;
}

/// The offset of a start-up-code label from the start of the code.
fn off(label: &u8) -> u64 {
    // SAFETY: only addresses are taken.
    label as *const u8 as u64 - unsafe { &ap_tramp } as *const u8 as u64
}

/// Is the running core the second one (once it has checked in)?
pub fn on_core1() -> bool {
    UP.load(Ordering::Relaxed) && cpu::lapic_id() == AP_ID.load(Ordering::Relaxed)
}

/// Core 0: wake core 1, run the demo, report. Returns with core 1 idle in its own scheduler.
pub fn bring_up() {
    let page = match LOW_PAGE.load(Ordering::Relaxed) {
        0 => LOW_PAGE_BS.load(Ordering::Relaxed),
        p => p,
    };
    if page == 0 {
        crate::kprintln!("[pc] SMP: no free page below 1 MiB for the start-up code; running single-core");
        return;
    }
    // SAFETY: `page` is free conventional RAM below 1 MiB (UEFI memory map), mapped 1:1;
    // the start-up code is plain bytes in .text. Single core here.
    unsafe {
        let src = &ap_tramp as *const u8;
        let len = &ap_tramp_end as *const u8 as usize - src as usize;
        core::ptr::copy_nonoverlapping(src, page as *mut u8, len);
        let put32 = |at: u64, v: u64| core::ptr::write_unaligned((page + at) as *mut u32, v as u32);
        let put64 = |at: u64, v: u64| core::ptr::write_unaligned((page + at) as *mut u64, v);
        put32(off(&ap_gdtptr) + 2, page + off(&ap_gdt));
        put32(off(&ap_far32), page + off(&ap_pm32));
        put32(off(&ap_far64), page + off(&ap_lm64));
        // the copy's entries are 64-bit: the tables they lead to may be anywhere
        let low = page - 0x1000;
        core::ptr::copy_nonoverlapping(crate::mmu::root() as *const u8, low as *mut u8, 4096);
        put32(off(&ap_cr3), low);
        let top = core::ptr::addr_of!(AP_STACK) as u64 + core::mem::size_of::<Stack>() as u64;
        put64(off(&ap_stack), top - 8); // as if `call` had pushed a return address
        put64(off(&ap_entry), ap_main as usize as u64);
    }
    let Some(dest) = crate::acpi::other_core(cpu::lapic_id()) else {
        crate::kprintln!("[pc] SMP: the ACPI MADT lists one core; running single-core");
        return;
    };
    cpu::send_init_sipi(page, dest);
    let stuck = cpu::ICR_STUCK.load(Ordering::Relaxed);
    if stuck != 0 {
        crate::kprintln!("[pc] SMP: {} start-up IPI(s) were still pending after 10 ms", stuck);
    }
    let t0 = crate::timer::cycles();
    let hz = crate::timer::hz().max(1);
    while !UP.load(Ordering::Acquire) && (crate::timer::cycles() - t0) < hz / 5 {
        core::hint::spin_loop();
    }
    if !UP.load(Ordering::Acquire) {
        crate::kprintln!("[pc] SMP: no second core answered the STARTUP IPI (a 1-core machine); running single-core");
        return;
    }
    crate::kprintln!("[pc] SMP: 2 cores online (boot core + core 1, woken by INIT/STARTUP IPIs)");
    crate::kprintln!("[pc] SMP: core 1 has local-APIC id {} (recorded by core 1)", AP_ID.load(Ordering::Relaxed));

    // 2. parallel work: queue the jobs; core 1 drains them while core 0 waits
    for _ in 0..N_JOBS {
        JOB_PUSHED.fetch_add(1, Ordering::Release);
    }
    let t0 = crate::timer::cycles();
    while JOBS_DONE.load(Ordering::Acquire) < N_JOBS && crate::timer::cycles() - t0 < hz * 2 {
        core::hint::spin_loop();
    }
    SHUTDOWN.store(true, Ordering::Release);
    let (done, sum) = (JOBS_DONE.load(Ordering::Acquire), JOB_SUM.load(Ordering::Acquire));
    if done == N_JOBS {
        // sum over id = 1..20 of id*id + id = 2870 + 210 = 3080
        crate::kprintln!("[pc] SMP: core 1 completed {} jobs in parallel (checksum {})", done, sum);
    } else {
        crate::kprintln!("[pc] SMP: core 1 completed {} of {} jobs (timed out)", done, N_JOBS);
    }
    REPORTED.store(true, Ordering::Release);

    // 3-4. once core 1's scheduler runs, interrupt it three times
    let t0 = crate::timer::cycles();
    while !CORE1_READY.load(Ordering::Acquire) && crate::timer::cycles() - t0 < hz * 2 {
        core::hint::spin_loop();
    }
    if !CORE1_READY.load(Ordering::Acquire) {
        crate::kprintln!("[pc] SMP: core 1's scheduler never reported ready");
        return;
    }
    let ap = AP_ID.load(Ordering::Relaxed);
    for _ in 0..3 {
        let before = IPIS.load(Ordering::Acquire);
        cpu::send_ipi(ap, IPI_VECTOR);
        crate::kprintln!("[pc] SMP: sent an IPI to core 1");
        let t0 = crate::timer::cycles();
        while IPIS.load(Ordering::Acquire) == before && crate::timer::cycles() - t0 < hz / 10 {
            core::hint::spin_loop();
        }
    }
    // 5. cross-core IPC: post, interrupt, wait for the answer
    for msg in [100u64, 300] {
        REPLY.store(0, Ordering::Release);
        MAIL.store(msg, Ordering::Release);
        cpu::send_ipi(ap, IPI_VECTOR);
        crate::kprintln!("[pc] SMP: sent IPC msg {} to core 1", msg);
        let t0 = crate::timer::cycles();
        while REPLY.load(Ordering::Acquire) == 0 && crate::timer::cycles() - t0 < hz / 5 {
            core::hint::spin_loop();
        }
        let r = REPLY.load(Ordering::Acquire);
        if r >> 32 == msg {
            crate::kprintln!("[pc] SMP: core 1 acked IPC msg {} -> result {}", msg, r & 0xFFFF_FFFF);
        } else {
            crate::kprintln!("[pc] SMP: core 1 did not answer IPC msg {}", msg);
        }
    }
}

/// Core 1 starts here, in 64-bit mode on its own stack, interrupts off.
extern "sysv64" fn ap_main() -> ! {
    cpu::load_gdt_ap();
    if crate::mmu::nx() {
        crate::mmu::nx_here(); // the kernel's tables hold NX bits: reserved on this core until NXE
    }
    // from the start-up copy to the kernel's own top-level table (same entries, but later
    // changes to the kernel's table are seen here too)
    // SAFETY: both tables map the kernel 1:1 the same way, so execution continues unchanged.
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) crate::mmu::root(), options(nostack)) };
    crate::mmu::pat_init(); // the same PAT as core 0 (its screen pages are WC: every core must agree)
    cpu::lapic_enable_here();
    AP_ID.store(cpu::lapic_id(), Ordering::Relaxed);
    UP.store(true, Ordering::Release);
    crate::kprintln!("[core 1] online (local APIC id {}, own stack, the kernel's page tables)", cpu::lapic_id());

    // 1. drain the job queue
    loop {
        let taken = JOB_TAKEN.load(Ordering::Acquire);
        if taken < JOB_PUSHED.load(Ordering::Acquire) {
            if JOB_TAKEN.compare_exchange(taken, taken + 1, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                let id = taken as u64 + 1;
                JOB_SUM.fetch_add(id * id + id, Ordering::AcqRel);
                JOBS_DONE.fetch_add(1, Ordering::Release);
            }
        } else if SHUTDOWN.load(Ordering::Acquire) {
            break;
        } else {
            core::hint::spin_loop();
        }
    }
    while !REPORTED.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    // 2. this core's own scheduler: thread A runs here, thread B waits in a prepared frame,
    // and core 1's local-APIC timer switches between them (`core1_tick`)
    // SAFETY: core 1 only; core 0 never touches THREADS.
    unsafe {
        let top = core::ptr::addr_of!(THREAD_B_STACK) as u64 + core::mem::size_of::<Stack>() as u64;
        let mut f: TrapFrame = core::mem::zeroed();
        f.rip = thread_b as usize as u64;
        f.cs = cpu::KERNEL_CS as u64;
        f.rflags = 0x202;
        f.rsp = top - 8;
        f.ss = 0x10;
        THREADS[1] = f;
        CUR = 0;
    }
    cpu::lapic_timer_start(0x2_0000);
    crate::kprintln!("[core 1] scheduler up: 2 threads under per-core timer IRQ");
    CORE1_READY.store(true, Ordering::Release);
    cpu::sti();
    thread_a()
}

static mut THREADS: [TrapFrame; 2] = [unsafe { core::mem::zeroed() }; 2];
static mut CUR: usize = 0;

fn thread_a() -> ! {
    for n in 1..=3 {
        crate::kprintln!("[core 1] thread A tick {}", n);
        cpu::hlt(); // the next timer interrupt may hand the core to thread B
    }
    loop {
        cpu::hlt();
    }
}

extern "sysv64" fn thread_b() -> ! {
    for n in 1..=3 {
        crate::kprintln!("[core 1] thread B tick {}", n);
        cpu::hlt();
    }
    loop {
        cpu::hlt();
    }
}

/// Core 1's timer interrupt: save the running thread's frame, resume the other.
pub fn core1_tick(f: &mut TrapFrame) {
    // SAFETY: core 1 only, in its interrupt handler.
    unsafe {
        THREADS[CUR] = *f;
        CUR ^= 1;
        *f = THREADS[CUR];
    }
}

/// TLB shootdown for a memory-type change (D1, architect's condition 3): asked / done counts.
static FLUSH_ASKED: AtomicU32 = AtomicU32::new(0);
static FLUSH_DONE: AtomicU32 = AtomicU32::new(0);

/// True when every other core can be asked to flush now: no other core runs, or core 1's
/// scheduler is up (it takes interrupts). False while core 1 is still starting.
pub fn can_flush_others() -> bool {
    !UP.load(Ordering::Acquire) || (CORE1_READY.load(Ordering::Acquire) && !on_core1())
}

/// After a change of memory type in the kernel's tables: ask core 1 by IPI to drain its WC
/// buffers, write back its caches and reload CR3, and wait (up to 0.2 s) until it has. True
/// when no other core runs or it answered.
pub fn flush_others() -> bool {
    if !UP.load(Ordering::Acquire) {
        return true;
    }
    if !can_flush_others() {
        return false;
    }
    let want = FLUSH_ASKED.fetch_add(1, Ordering::AcqRel) + 1;
    cpu::send_ipi(AP_ID.load(Ordering::Relaxed), IPI_VECTOR);
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while FLUSH_DONE.load(Ordering::Acquire) < want && crate::timer::cycles() - t0 < hz / 5 {
        core::hint::spin_loop();
    }
    FLUSH_DONE.load(Ordering::Acquire) >= want
}

/// Cores running now: the boot core, plus core 1 once it is online (after its `pat_init`).
pub fn cores_running() -> u64 {
    1 + UP.load(Ordering::Acquire) as u64
}

/// Shootdowns core 1 has done (`screen wc on|off` reports it).
pub fn flushes_done() -> u32 {
    FLUSH_DONE.load(Ordering::Acquire)
}

/// An inter-processor interrupt on core 1: a TLB shootdown, a message to answer, or a plain
/// signal to report. IPIs merge: one interrupt can carry a shootdown and a signal, and after a
/// shootdown with no mail this returns without counting the signal, so IPIS is not a count of
/// IPIs sent; never build a counter on it (architect's D1d).
pub fn ipi() {
    let asked = FLUSH_ASKED.load(Ordering::Acquire);
    if FLUSH_DONE.load(Ordering::Relaxed) < asked {
        // SAFETY: drain WC buffers, write back caches, drop every cached translation.
        unsafe { core::arch::asm!("sfence", "wbinvd", "mov {t}, cr3", "mov cr3, {t}", t = out(reg) _, options(nostack)) };
        FLUSH_DONE.store(asked, Ordering::Release);
        if MAIL.load(Ordering::Acquire) == 0 {
            return;
        }
    }
    let n = IPIS.load(Ordering::Relaxed) + 1;
    let msg = MAIL.swap(0, Ordering::AcqRel);
    if msg != 0 {
        let result = msg * 2;
        crate::kprintln!("[core 1] IPC msg {} from core 0 -> result {}", msg, result);
        REPLY.store(msg << 32 | result, Ordering::Release);
    } else {
        crate::kprintln!("[core 1] SGI from core 0 ({}): an x86 IPI, vector {:#x}", n, IPI_VECTOR);
    }
    IPIS.store(n, Ordering::Release);
}
