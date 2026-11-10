//! Mansa — AfriSovera OS microkernel.
//!
//! Phase 0.3 (the Phase 0 finale): **userspace**. On top of 0.2's MMU, frame
//! allocator, heap, and interrupts, this drops to **EL0** with per-process
//! address spaces, an **SVC syscall** interface, a **capability table**, and
//! **IPC endpoints** — then runs two userspace processes that exchange a
//! **capability-secured message**. That is the security spine the whole OS
//! (and the no-gatekeeper app store) rests on.
#![no_std]
#![no_main]

extern crate alloc;

mod asili;
mod board;
mod config;
mod cpu;
mod daraja;
mod drbg;
mod drbg_kat;
mod ed25519;
mod exceptions;
mod fdt;
mod fmtbuf;
mod font;
mod integrity;
mod mesh;
mod sha256;
mod sha512;
mod framebuffer;
mod frame;
mod fwcfg;
mod gic;
mod gpio;
mod heap;
mod nia;
mod paging;
mod psci;
#[cfg(feature = "board-raspi")]
mod pi_fb;
mod process;
mod rng;
mod sched;
mod shell;
mod store;
mod timer;
mod tonga;
mod desk;
#[path = "pc/font_lower.rs"]
mod font_lower;
mod uart;
mod uart_ns16550;
mod virtio;
mod voice;
mod web;
mod vfs;

use core::arch::global_asm;
use core::panic::PanicInfo;
use core::ptr::addr_of;

// The DRAM range is no longer a constant here (2.731): it comes from
// `board::resolved()`, which is the compile-time map on a machine with no live tree
// and the tree's own /memory node on one that passes a trustworthy blob. The
// fallback that used to live here is `board::ACTIVE.ram_base/ram_size`.

extern "C" {
    static __kernel_end: u8;
}

// ---------------------------------------------------------------------------
// Boot: park secondary cores, install a stack, drop EL2→EL1 if needed, clear
// BSS, enter Rust. Placed in `.text.boot` (kept first by the linker script).
// ---------------------------------------------------------------------------
global_asm!(
    r#"
.section ".text.boot"
.global _start
_start:
    // --- arm64 Linux Image header (Documentation/arm64/booting.rst) ---
    // Lets the kernel boot as a raw `Image` (QEMU raw -kernel, U-Boot, real HW),
    // where firmware passes a live DTB pointer in x0. Harmless to the ELF path:
    // the first instruction just branches over the 64-byte header to the boot code.
    b       header_end              // code0: branch to entry
    .long   0                       // code1
    .quad   0                       // text_offset: load at 2 MiB-aligned RAM base
    .quad   __image_size            // image_size: total footprint
    .quad   2                       // flags: LE, 4 KiB pages
    .quad   0                       // res2
    .quad   0                       // res3
    .quad   0                       // res4
    .long   0x644d5241              // magic: "ARM\x64"
    .long   0                       // res5 (PE header offset, unused)
header_end:
    mov     x20, x0                 // preserve the DTB pointer QEMU passes in x0
    mrs     x1, mpidr_el1
    and     x1, x1, #0xff
    cbnz    x1, park_secondary

    ldr     x0, =__stack_top
    mov     sp, x0

    // EL2: configure and drop to EL1h. EL1 already: continue directly.
    // Anything else (EL3, EL0) is a boot state no supported board's firmware
    // hands off at (docs/15: RPi4 armstub and PINE64 BL31 both land at EL2 or
    // already-EL1) -- park rather than silently running the EL1-assuming
    // kernel un-dropped. The old `b.ne el1_entry` conflated "not EL2" with
    // "already EL1", which finding-3.712 caught: QEMU's raspi4b
    // `-device loader,cpu-num=N` bypass enters at EL3 and fell through
    // untouched, at EL1-scoped register writes actually executing at EL3.
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.eq    el2_to_el1
    cmp     x0, #1
    b.eq    el1_entry
    b       park_secondary          // unsupported EL (EL3/EL0): park

el2_to_el1:
    // --- EL2 -> EL1 ---
    mov     x0, #(1 << 31)          // HCR_EL2.RW = 1 : EL1 executes AArch64
    msr     hcr_el2, x0
    mov     x0, #0x0800             // SCTLR_EL1 = 0x30d00800 (RES1, MMU/caches off)
    movk    x0, #0x30d0, lsl #16
    msr     sctlr_el1, x0
    mrs     x0, cnthctl_el2
    orr     x0, x0, #3              // EL1PCTEN | EL1PCEN : EL1 timer access
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr
    mov     x0, #0x3c5              // SPSR_EL2 : EL1h, DAIF masked
    msr     spsr_el2, x0
    adr     x0, el1_entry
    msr     elr_el2, x0
    ldr     x0, =__stack_top
    msr     sp_el1, x0
    eret

park_secondary:
1:  wfe
    b       1b

// Entry point a secondary core lands on after PSCI CPU_ON (MMU off, its own stack).
.global _start_secondary
_start_secondary:
    ldr     x0, =__stack_top_secondary
    mov     sp, x0
    // Enable FP/SIMD (CPACR_EL1.FPEN=0b11) — the compiler emits NEON for the Rust.
    mov     x0, #(3 << 20)
    msr     cpacr_el1, x0
    isb
    bl      kernel_secondary
2:  wfe
    b       2b

el1_entry:
    ldr     x0, =__bss_start
    ldr     x1, =__bss_end
2:  cmp     x0, x1
    b.ge    3f
    str     xzr, [x0], #8
    b       2b
3:  mov     x0, x20                 // pass the DTB pointer as kernel_main's arg
    bl      kernel_main
4:  wfe
    b       4b
"#
);

// Minimal Raspberry Pi (BCM2837) bring-up: the memory map, interrupt controller
// and firmware all differ from QEMU virt, so this path keeps the MMU off and does
// UART-only output — proof the kernel can target a second, real-hardware-shaped
// board. Full Pi bring-up (paging, IRQ, mailbox framebuffer) is future work; the
// virt build remains the full-featured default. See docs/15.
#[cfg(feature = "board-raspi")]
#[no_mangle]
pub extern "C" fn kernel_main(_dtb: u64) -> ! {
    cpu::enable_fp_simd(); // the compiler emits NEON for memcpy/memset/fmt
    uart::console_init(); // no-op on PL011 parts; FIFO setup on an 8250 board
    kprintln!();
    kprintln!("========================================");
    kprintln!("  Mansa microkernel — AfriSovera OS");
    kprintln!("  Raspberry Pi (BCM2837) UART bring-up");
    kprintln!("========================================");
    kprintln!("[mansa] board: {}", board::NAME);
    kprintln!("[mansa] PL011 UART @ {:#x}", board::UART_BASE);
    kprintln!("[mansa] hello from AfriSovera on a second board (QEMU raspi3b)");

    // Enable the MMU with the BCM2837 map (RAM Normal, peripherals Device) so the
    // kernel runs cached and atomics work — then the heap comes alive.
    paging::init();
    kprintln!("[mansa] Pi MMU on (BCM2837: RAM Normal @ 0x0, peripherals Device @ 0x3F00_0000)");

    // Prove the kernel heap works on the Pi (atomics require Normal memory).
    {
        use alloc::vec::Vec;
        let mut v: Vec<u32> = Vec::new();
        let mut sum = 0u32;
        for i in 1..=8u32 {
            v.push(i * i);
            sum += i * i;
        }
        kprintln!("[mansa] heap OK: sum of squares 1..8 = {} ({} elems)", sum, v.len());
    }

    // Ask the VideoCore GPU for a framebuffer and draw the splash.
    if pi_fb::init() {
        pi_fb::splash();
        kprintln!("[mansa] Pi framebuffer splash drawn ({}x{})", pi_fb::WIDTH, pi_fb::HEIGHT);
    } else {
        kprintln!("[mansa] Pi framebuffer unavailable (mailbox) — UART only");
    }
    // BCM2837 system timer: a free-running 1 MHz counter (CLO @ 0x3F00_3004).
    // Read it twice across a delay to prove a real hardware timebase advances.
    {
        let clo = 0x3F00_3004 as *const u32;
        // SAFETY: 0x3F00_3000 is in the Pi's Device-mapped peripheral window.
        let t0 = unsafe { core::ptr::read_volatile(clo) };
        let mut spin = 0u64;
        while spin < 2_000_000 {
            spin += 1;
            core::hint::spin_loop();
        }
        let t1 = unsafe { core::ptr::read_volatile(clo) };
        kprintln!(
            "[mansa] Pi system timer: {} -> {} us (advanced {} us) — hardware timebase OK",
            t0,
            t1,
            t1.wrapping_sub(t0)
        );
    }
    // Interrupt-driven ticks: route the ARM generic timer through the BCM2836/
    // 2837 LOCAL interrupt controller (no GIC on Pi3) and take periodic IRQs.
    {
        use core::ptr::write_volatile;
        exceptions::init(); // VBAR_EL1 -> shared vector table (PC-relative, works on Pi)

        // Core0 Timers interrupt control @ 0x4000_0040: bit 1 routes the
        // non-secure physical timer (CNTPNSIRQ) to this core's IRQ line.
        const CORE0_TIMER_IRQCNTL: *mut u32 = 0x4000_0040 as *mut u32;
        // SAFETY: Device-mapped local-peripheral register (L1[1] block).
        unsafe { write_volatile(CORE0_TIMER_IRQCNTL, 1 << 1) };

        // Register this boot context as the idle thread and spawn two kernel
        // threads, so the timer IRQ can round-robin them (real preemption on the
        // Pi, the same scheduler the virt build uses).
        sched::init_idle();
        // A refused thread is reported and survived, not asserted on (2.735): the idle
        // loop below is a working kernel with zero, one, or two of these running.
        for (name, entry) in [("A", pi_thread_a as extern "C" fn(usize) -> !), ("B", pi_thread_b)] {
            if let Err(why) = sched::spawn(entry, 0) {
                kprintln!("[mansa] Pi thread {} NOT spawned — {}", name, why);
            }
        }

        timer::rearm(); // arm CNTP_TVAL/CTL for the first countdown
        cpu::irq_enable(); // unmask IRQs so the timer fires + preempts
        kprintln!("[mansa] Pi timer IRQ armed + scheduler: 2 kernel threads preempting");
    }

    kprintln!("[mansa] Raspberry Pi bring-up: MMU + heap + framebuffer + preemptive scheduler");
    loop {
        // The idle thread: wait for the next timer interrupt, which preempts us
        // into thread A / B and rotates back here.
        cpu::wfi();
    }
}

/// A kernel thread on the Pi. Prints its first few heartbeats then keeps looping;
/// the timer IRQ preempts it regardless, which is what proves Pi scheduling.
#[cfg(feature = "board-raspi")]
extern "C" fn pi_thread_a(_arg: usize) -> ! {
    let mut n = 0u32;
    loop {
        n += 1;
        if n <= 6 {
            kprintln!("[pi] thread A tick {}", n);
        }
        for _ in 0..3_000_000 {
            core::hint::spin_loop();
        }
    }
}

/// A second Pi kernel thread (see [`pi_thread_a`]).
#[cfg(feature = "board-raspi")]
extern "C" fn pi_thread_b(_arg: usize) -> ! {
    let mut n = 0u32;
    loop {
        n += 1;
        if n <= 6 {
            kprintln!("[pi] thread B tick {}", n);
        }
        for _ in 0..3_000_000 {
            core::hint::spin_loop();
        }
    }
}

/// Count and report a Raspberry Pi timer interrupt (called from the Pi IRQ
/// handler). Prints the first few so the serial log stays readable.
#[cfg(feature = "board-raspi")]
static PI_TICKS: AtomicU32 = AtomicU32::new(0);

#[cfg(feature = "board-raspi")]
pub fn pi_timer_tick() {
    let n = PI_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 6 {
        kprintln!("[pi] timer tick {}", n);
    }
}

// --- SMP: a secondary CPU core, plus a shared work queue it drains ---
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

#[cfg(not(feature = "board-raspi"))]
extern "C" {
    fn _start_secondary();
}
static SECONDARY_UP: AtomicU32 = AtomicU32::new(0);
// The secondary core records its own MPIDR Aff0 here once it is running, proving
// it actually executed code. Sentinel 0xffff_ffff = not yet recorded.
static SECONDARY_AFF0: AtomicU32 = AtomicU32::new(0xffff_ffff);

/// Number of CPU cores currently online: 1 (primary only) or 2 (a secondary
/// checked in via PSCI on QEMU virt). Honest — reflects the real boot topology.
pub fn cores_online() -> u32 {
    if SECONDARY_UP.load(Ordering::Acquire) == 1 {
        2
    } else {
        1
    }
}

/// The machine's RAM in MiB (the `crate::ram_mib` the shared desktop and `mem` print). On
/// the ARM kernel the frame allocator covers the real RAM, so the frame total is it.
pub fn ram_mib() -> u64 {
    (crate::frame::total_frames() * 4 / 1024) as u64
}

/// The secondary core's MPIDR Aff0 (its affinity/core id), or None if no
/// secondary is online. The value is written by the secondary itself once it is
/// running, so a Some(_) proves the second core executed our code.
pub fn secondary_aff0() -> Option<u32> {
    if SECONDARY_UP.load(Ordering::Acquire) == 1 {
        Some(SECONDARY_AFF0.load(Ordering::Acquire))
    } else {
        None
    }
}

// Shared job queue: the primary enqueues work items and the secondary core
// drains them in parallel. A single spinlock guards the ring indices. This
// state is UNCONDITIONAL because `kernel_secondary` (reached from shared boot
// asm on both boards) touches it; only the virt path ever enqueues.
//
// HONEST HW NOTE: the secondary runs MMU-off here, so on real silicon the
// exclusive monitor (used by compare_exchange / fetch_add) and cache coherency
// with the MMU-on primary are not guaranteed. It works under QEMU (coherent
// memory model, lenient monitor). A real per-core scheduler needs the secondary
// to enable the MMU with matching cacheable attributes first -- a later step.
const JOB_CAP: usize = 64;
const JOB_SLOT_INIT: AtomicU32 = AtomicU32::new(0);
static JOB_RING: [AtomicU32; JOB_CAP] = [JOB_SLOT_INIT; JOB_CAP];
static JOB_HEAD: AtomicUsize = AtomicUsize::new(0); // producer index (primary)
static JOB_TAIL: AtomicUsize = AtomicUsize::new(0); // consumer index (secondary)
static JOB_LOCK: AtomicU32 = AtomicU32::new(0); // 0 = free, 1 = held
static JOBS_DONE_BY_CORE1: AtomicU32 = AtomicU32::new(0);
static JOB_RESULT_SUM: AtomicU32 = AtomicU32::new(0);
static SMP_SHUTDOWN: AtomicU32 = AtomicU32::new(0);
/// Set by the primary once it has printed its SMP report, so core 1 holds off
/// starting its (chatty) per-core scheduler until the report prints uncontended.
static SMP_REPORTED: AtomicU32 = AtomicU32::new(0);
/// Set by core 1 once its scheduler is running and IRQs are unmasked, so the
/// primary knows it can deliver cross-core SGIs that will actually be handled.
static CORE1_READY: AtomicU32 = AtomicU32::new(0);
/// Count of cross-core SGIs core 1 has received from the primary.
static SGI_COUNT: AtomicU32 = AtomicU32::new(0);

// --- Cross-core IPC mailbox (primary -> core 1, with a round-trip ack) --------
// The primary writes a payload under IPC_LOCK, bumps IPC_SEQ, then fires an SGI
// (id IPC_SGI). Core 1's SGI handler reads the payload, computes a result, and
// publishes it via IPC_RESULT + IPC_ACK_SEQ. The primary waits for the matching
// ack sequence and reads the result back — real data across cores, both ways.
const IPC_SGI: u32 = 1; // SGI id 0 = wakeup (2.56); id 1 = IPC message (2.57)

// 2.991 falsify-first harness (debug-only, reached only by the `sgi-el0` shell
// command). A self-targeted SGI id that carries no mailbox semantics, so the
// probe grades exactly one thing: WHERE the SGI was taken. See
// docs/design-2.991-sgi-el0-probe.md.
pub const PROBE_SGI: u32 = 2;
pub static SGI_PROBE_SEEN: AtomicU32 = AtomicU32::new(0);
static IPC_LOCK: AtomicU32 = AtomicU32::new(0);
static IPC_PAYLOAD: AtomicU32 = AtomicU32::new(0);
static IPC_SEQ: AtomicU32 = AtomicU32::new(0);
static IPC_ACK_SEQ: AtomicU32 = AtomicU32::new(0);
static IPC_RESULT: AtomicU32 = AtomicU32::new(0);

#[cfg(not(feature = "board-raspi"))]
fn ipc_lock() {
    while IPC_LOCK
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[cfg(not(feature = "board-raspi"))]
fn ipc_unlock() {
    IPC_LOCK.store(0, Ordering::Release);
}

/// Acquire the job-queue spinlock (test-and-set; Acquire orders the critical
/// section after the lock is taken).
fn job_lock() {
    while JOB_LOCK
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Release the job-queue spinlock (Release publishes the critical section).
fn job_unlock() {
    JOB_LOCK.store(0, Ordering::Release);
}

/// Pop the next job id under the lock, or None if the ring is empty.
fn job_pop() -> Option<u32> {
    job_lock();
    let tail = JOB_TAIL.load(Ordering::Relaxed);
    let head = JOB_HEAD.load(Ordering::Relaxed);
    let out = if tail < head {
        let id = JOB_RING[tail % JOB_CAP].load(Ordering::Relaxed);
        JOB_TAIL.store(tail + 1, Ordering::Relaxed);
        Some(id)
    } else {
        None
    };
    job_unlock();
    out
}

/// Push a job id onto the ring under the lock (primary side, virt only).
#[cfg(not(feature = "board-raspi"))]
fn job_push(id: u32) {
    job_lock();
    let head = JOB_HEAD.load(Ordering::Relaxed);
    JOB_RING[head % JOB_CAP].store(id, Ordering::Relaxed);
    JOB_HEAD.store(head + 1, Ordering::Relaxed);
    job_unlock();
}

/// A secondary core lands here (MMU off, its own stack). It announces itself on
/// the UART (kprintln's lock/framebuffer tee aren't safe from a second, MMU-off
/// core), signals the primary, then drains the shared job queue until the
/// primary raises SMP_SHUTDOWN. A bounded idle spin guarantees it never hangs.
#[no_mangle]
pub extern "C" fn kernel_secondary() {
    let uart = board::UART_BASE as *mut u32;
    // SAFETY: direct PL011 data-register writes; MMIO works with the MMU off.
    let emit = |s: &[u8]| unsafe {
        for &b in s {
            core::ptr::write_volatile(uart, b as u32);
        }
    };
    emit(b"[core 1] online (secondary CPU via PSCI)\n");

    // Turn on translation using the primary's page tables BEFORE touching shared
    // memory: the spinlock's exclusives and the cacheable atomics below are only
    // well-defined with the MMU on (with it off, DRAM is Device memory and the
    // exclusive monitor / coherency are not guaranteed on real silicon).
    #[cfg(not(feature = "board-raspi"))]
    {
        paging::enable_mmu_secondary();
        emit(b"[core 1] MMU on (secondary tables active)\n");
    }

    // Record our own MPIDR Aff0 (core id) so the primary can prove we ran.
    {
        let mpidr: u64;
        // SAFETY: MPIDR_EL1 is readable at EL1; the MMU is on by this point.
        unsafe {
            core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr);
        }
        SECONDARY_AFF0.store((mpidr & 0xff) as u32, Ordering::Release);
    }
    SECONDARY_UP.store(1, Ordering::Release);

    let mut idle: u64 = 0;
    loop {
        if let Some(id) = job_pop() {
            // "Execute" the job: a little real arithmetic, folded into a shared
            // checksum so the primary can verify core 1 actually ran the work.
            let work = id.wrapping_mul(id).wrapping_add(id);
            JOB_RESULT_SUM.fetch_add(work, Ordering::Relaxed);
            JOBS_DONE_BY_CORE1.fetch_add(1, Ordering::Release);
            idle = 0;
        } else if SMP_SHUTDOWN.load(Ordering::Acquire) == 1 {
            break;
        } else {
            idle += 1;
            if idle > 200_000_000 {
                break; // safety bound: never spin forever waiting on work
            }
            core::hint::spin_loop();
        }
    }
    // From here core 1 is MMU-on, so it prints through the cross-core console
    // lock (serialized against the primary's kprintln), not raw interleaved UART.
    #[cfg(not(feature = "board-raspi"))]
    crate::uart::locked_bytes(b"[core 1] queue drained\n");
    #[cfg(feature = "board-raspi")]
    emit(b"[core 1] queue drained\n");

    // Wait (bounded) for the primary to finish its SMP report so the scheduler
    // banner lands after it in the log.
    #[cfg(not(feature = "board-raspi"))]
    {
        let mut w = 0u64;
        while SMP_REPORTED.load(Ordering::Acquire) == 0 && w < 200_000_000 {
            w += 1;
            core::hint::spin_loop();
        }
    }

    // With the MMU on and the queue drained, core 1 starts its OWN preemptive
    // scheduler: it configures its exception vector, its GIC CPU interface, and
    // its generic timer, spawns two kernel threads, then becomes the idle thread.
    // From here core 1's timer IRQ round-robins A -> B -> idle on core 1 alone,
    // concurrently with the primary's own work. (Never returns.)
    #[cfg(not(feature = "board-raspi"))]
    {
        exceptions::init(); // VBAR_EL1 on this core (shared vector table)
        gic::init_secondary(); // this core's GICC (distributor already up)
        sched::register_current_secondary(); // running context = core 1 idle thread
        sched::spawn_secondary(core1_thread_a, 0);
        sched::spawn_secondary(core1_thread_b, 0);
        crate::uart::locked_bytes(b"[core 1] scheduler up: 2 threads under per-core timer IRQ\n");
        timer::init(); // this core's timer PPI + first countdown
        cpu::irq_enable(); // unmask IRQs so the timer preempts core 1
        CORE1_READY.store(1, Ordering::Release); // primary may now send SGIs
        loop {
            cpu::wfi(); // idle thread; the timer switches us into A / B
        }
    }
    #[cfg(feature = "board-raspi")]
    {
        emit(b"[core 1] parking\n");
        // The asm entry parks the core (wfe) after this returns.
    }
}

/// Append bytes to `buf` at `i`, returning the new index.
#[cfg(not(feature = "board-raspi"))]
fn put_bytes(buf: &mut [u8], mut i: usize, s: &[u8]) -> usize {
    for &b in s {
        buf[i] = b;
        i += 1;
    }
    i
}

/// Append the decimal digits of `n` to `buf` at `i`, returning the new index.
#[cfg(not(feature = "board-raspi"))]
fn put_num(buf: &mut [u8], mut i: usize, n: u64) -> usize {
    if n == 0 {
        buf[i] = b'0';
        return i + 1;
    }
    let mut tmp = [0u8; 20];
    let mut j = 0;
    let mut m = n;
    while m > 0 {
        tmp[j] = b'0' + (m % 10) as u8;
        m /= 10;
        j += 1;
    }
    while j > 0 {
        j -= 1;
        buf[i] = tmp[j];
        i += 1;
    }
    i
}

/// Print "<prefix><n><suffix>" from core 1 as a single locked message, so it
/// never interleaves with the primary's console output.
#[cfg(not(feature = "board-raspi"))]
fn c1_msg(prefix: &[u8], n: u64, suffix: &[u8]) {
    let mut buf = [0u8; 64];
    let mut i = 0;
    i = put_bytes(&mut buf, i, prefix);
    i = put_num(&mut buf, i, n);
    i = put_bytes(&mut buf, i, suffix);
    crate::uart::locked_bytes(&buf[..i]);
}

/// One core-1 thread heartbeat: "<label><n>\n".
#[cfg(not(feature = "board-raspi"))]
fn c1_tick(label: &[u8], n: u64) {
    c1_msg(label, n, b"\n");
}

/// Core 1's handler for a cross-core SGI from the primary, dispatched by id:
/// id 0 is a bare wakeup (2.56), id IPC_SGI carries a mailbox message (2.57).
/// Called from the IRQ path (`exceptions::rust_irq_handler`) on core 1.
#[cfg(not(feature = "board-raspi"))]
pub fn c1_on_sgi(intid: u32) {
    if intid == PROBE_SGI {
        // Probe instrument: count the delivery and return. Deliberately does no
        // mailbox work and prints nothing, so it is neutral to both entry points.
        SGI_PROBE_SEEN.fetch_add(1, Ordering::AcqRel);
        return;
    }
    if intid == IPC_SGI {
        c1_handle_ipc();
    } else {
        let n = SGI_COUNT.fetch_add(1, Ordering::AcqRel) + 1;
        c1_msg(b"[core 1] SGI from core 0 (", n as u64, b")\n");
    }
}

/// board-raspi sibling: this build has no core-1 bring-up, so an SGI taken here
/// carries no mailbox work. Defined as a no-op ONLY so the name is TOTAL across
/// both cfg arms - `process.rs:696` (`rust_irq_from_el0`) calls it un-gated, and
/// without this arm the board-raspi build dies at name resolution (E0425), which
/// is the whole of the `pi` FAIL in walk 2995 (findings 3.121 / 3.122 / 3.123).
/// Same idiom the tree already uses for `rust_irq_handler` and `kernel_main`.
#[cfg(feature = "board-raspi")]
pub fn c1_on_sgi(_intid: u32) {}

/// Core 1 consumes one IPC message: read the payload under the lock, compute a
/// result, publish it + the ack sequence for the primary, and report the round
/// trip. The "work" here is a stand-in (double the payload) — the point is that
/// a value went core 0 -> core 1 and a derived value came back.
#[cfg(not(feature = "board-raspi"))]
fn c1_handle_ipc() {
    ipc_lock();
    let payload = IPC_PAYLOAD.load(Ordering::Relaxed);
    let seq = IPC_SEQ.load(Ordering::Relaxed);
    ipc_unlock();

    let result = payload.wrapping_mul(2);
    IPC_RESULT.store(result, Ordering::Relaxed);
    IPC_ACK_SEQ.store(seq, Ordering::Release); // publish result before ack

    let mut buf = [0u8; 80];
    let mut i = 0;
    i = put_bytes(&mut buf, i, b"[core 1] IPC msg ");
    i = put_num(&mut buf, i, payload as u64);
    i = put_bytes(&mut buf, i, b" from core 0 -> result ");
    i = put_num(&mut buf, i, result as u64);
    i = put_bytes(&mut buf, i, b"\n");
    crate::uart::locked_bytes(&buf[..i]);
}

/// A kernel thread pinned to core 1. Prints its first few heartbeats (so the
/// serial log stays readable) then keeps looping — the timer preempts it either
/// way, which is what proves core 1 is scheduling.
#[cfg(not(feature = "board-raspi"))]
extern "C" fn core1_thread_a(_arg: usize) -> ! {
    let mut n: u64 = 0;
    loop {
        n += 1;
        if n <= 6 {
            c1_tick(b"[core 1] thread A tick ", n);
        }
        for _ in 0..3_000_000 {
            core::hint::spin_loop();
        }
    }
}

/// A second kernel thread pinned to core 1 (see [`core1_thread_a`]).
#[cfg(not(feature = "board-raspi"))]
extern "C" fn core1_thread_b(_arg: usize) -> ! {
    let mut n: u64 = 0;
    loop {
        n += 1;
        if n <= 6 {
            c1_tick(b"[core 1] thread B tick ", n);
        }
        for _ in 0..3_000_000 {
            core::hint::spin_loop();
        }
    }
}

/// PSCI CPU_ON (SMC64 0xC400_0003): power on `target` at `entry`. Returns 0 on
/// success, a negative PSCI error otherwise.
#[cfg(not(feature = "board-raspi"))]
fn psci_cpu_on(target: u64, entry: u64) -> i64 {
    let mut x0: u64 = 0xC400_0003;
    // SAFETY: QEMU virt (no EL3) exposes PSCI via the HVC conduit; QEMU intercepts
    // the call and handles it directly.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            inout("x0") x0,
            inout("x1") target => _,
            inout("x2") entry => _,
            inout("x3") 0u64 => _,
            options(nostack),
        );
    }
    x0 as i64
}

/// Bring up secondary core 1 (best-effort; single-core boots fine if absent),
/// then hand it a batch of jobs to drain in parallel and confirm it finished.
#[cfg(not(feature = "board-raspi"))]
fn bring_up_secondary() {
    let entry = _start_secondary as *const () as usize as u64;
    let ret = psci_cpu_on(1, entry);
    if ret != 0 {
        kprintln!("[mansa] SMP: no secondary core (PSCI CPU_ON -> {}); running single-core", ret);
        return;
    }
    let mut spins = 0u64;
    while SECONDARY_UP.load(Ordering::Acquire) == 0 && spins < 100_000_000 {
        spins += 1;
        core::hint::spin_loop();
    }
    if SECONDARY_UP.load(Ordering::Acquire) != 1 {
        // Powered on but never checked in: do NOT wait on it for work.
        kprintln!("[mansa] SMP: secondary powered on but did not check in");
        return;
    }
    kprintln!("[mansa] SMP: 2 cores online (primary + secondary, PSCI)");
    // The secondary recorded its own MPIDR Aff0 before checking in; echo it so the
    // topology (and the fact core 1 executed our code) is provable from the log.
    kprintln!(
        "[mansa] SMP: secondary core Aff0 = {} (recorded by core 1)",
        SECONDARY_AFF0.load(Ordering::Acquire)
    );

    // Real parallel work: enqueue N jobs; core 1 drains them while we wait.
    const N: u32 = 20;
    for id in 1..=N {
        job_push(id);
    }
    let mut waited = 0u64;
    while JOBS_DONE_BY_CORE1.load(Ordering::Acquire) < N && waited < 200_000_000 {
        waited += 1;
        core::hint::spin_loop();
    }
    SMP_SHUTDOWN.store(1, Ordering::Release);
    let done = JOBS_DONE_BY_CORE1.load(Ordering::Acquire);
    let sum = JOB_RESULT_SUM.load(Ordering::Acquire);
    if done == N {
        // Expected checksum = sum_{id=1..20}(id*id + id) = 2870 + 210 = 3080.
        kprintln!("[mansa] SMP: core 1 completed {} jobs in parallel (checksum {})", done, sum);
    } else {
        kprintln!("[mansa] SMP: core 1 completed {} of {} jobs (timed out)", done, N);
    }
    // Release core 1 to start its per-core scheduler now that the report printed.
    SMP_REPORTED.store(1, Ordering::Release);

    // Cross-core signalling: once core 1's scheduler is up and taking IRQs, send
    // it a few SGIs (software-generated interrupts) and let it report each. This
    // is the primitive behind cross-core IPC, scheduler kicks, and TLB shootdown.
    let mut w = 0u64;
    while CORE1_READY.load(Ordering::Acquire) == 0 && w < 200_000_000 {
        w += 1;
        core::hint::spin_loop();
    }
    if CORE1_READY.load(Ordering::Acquire) == 1 {
        for _ in 0..3 {
            gic::send_sgi(1, 0); // SGI 0 to core 1
            kprintln!("[mansa] SMP: sent SGI to core 1");
            // Let core 1 take and handle this SGI before sending the next (SGIs
            // of the same id/source coalesce while one is still pending).
            let mut d = 0u64;
            while d < 20_000_000 {
                d += 1;
                core::hint::spin_loop();
            }
        }

        // Cross-core IPC: send payloads through the shared mailbox; core 1
        // processes each and acks a result the primary reads back (round trip).
        for &payload in &[100u32, 200, 300] {
            let result = send_ipc_to_core1(payload);
            kprintln!("[mansa] SMP: core 1 acked IPC msg {} -> result {}", payload, result);
        }
    }
}

/// Send one IPC message to core 1 and wait (bounded) for its ack, returning the
/// result core 1 computed. Publishes the payload under the mailbox lock, bumps
/// the sequence, then signals core 1 with an SGI. Returns 0 if no ack arrives.
#[cfg(not(feature = "board-raspi"))]
fn send_ipc_to_core1(payload: u32) -> u32 {
    ipc_lock();
    IPC_PAYLOAD.store(payload, Ordering::Relaxed);
    let seq = IPC_SEQ.load(Ordering::Relaxed) + 1;
    IPC_SEQ.store(seq, Ordering::Relaxed);
    ipc_unlock();

    gic::send_sgi(1, IPC_SGI);
    kprintln!("[mansa] SMP: sent IPC msg {} to core 1", payload);

    let mut w = 0u64;
    while IPC_ACK_SEQ.load(Ordering::Acquire) < seq && w < 200_000_000 {
        w += 1;
        core::hint::spin_loop();
    }
    // A short settle so core 1's SGI is fully EOI'd before the next send.
    let mut d = 0u64;
    while d < 10_000_000 {
        d += 1;
        core::hint::spin_loop();
    }
    IPC_RESULT.load(Ordering::Acquire)
}

#[cfg(not(feature = "board-raspi"))]
#[no_mangle]
pub extern "C" fn kernel_main(dtb: u64) -> ! {
    cpu::enable_fp_simd();
    // Console first: on a PL011 board this is a no-op (the part is live at reset),
    // but the `board-pine64` build compiles this same entry path and its 8250 wants
    // its FIFOs enabled before the first character of the banner.
    uart::console_init();

    kprintln!();
    kprintln!("========================================");
    kprintln!("  Mansa microkernel — AfriSovera OS");
    kprintln!("  Phase 0.3: userspace + caps + IPC");
    kprintln!("========================================");
    kprintln!();
    kprintln!("[mansa] boot core online at EL{}", cpu::current_el());

    // 1) Address space: identity map + MMU on. After this, memory is cached and
    //    atomics work.
    paging::init();
    kprintln!("[mansa] MMU enabled — identity map live (device + DRAM)");

    // Parse the device tree the firmware handed us (x0) to discover the machine
    // instead of trusting only the compile-time board map. QEMU's ELF boot passes
    // x0=0 (no live DTB); fall back to the bundled reference blob so the parser is
    // still exercised (its RAM figure is a fixture and never sizes the allocator).
    fdt::parse(dtb);
    if !fdt::info().valid {
        fdt::parse_reference();
    }
    let dt = fdt::info();
    if dt.valid && !dt.reference {
        kprintln!(
            "[mansa] device tree (live): model '{}', RAM {:#x}..{:#x} ({} MiB)",
            fdt::model(),
            dt.ram_base,
            dt.ram_base + dt.ram_size,
            dt.ram_size / (1024 * 1024)
        );
        kprintln!(
            "[mansa] device tree (live): UART @ {:#x}, GIC @ {:#x} (board map: {:#x}, {:#x})",
            dt.uart_base,
            dt.gic_base,
            board::UART_BASE,
            board::GIC_DIST_BASE
        );
    } else if dt.valid {
        kprintln!(
            "[mansa] firmware passed no DTB (x0={:#x}); parsed bundled reference: model '{}', {} MiB",
            dtb,
            fdt::model(),
            dt.ram_size / (1024 * 1024)
        );
    } else {
        kprintln!("[mansa] no device tree available — using board defaults");
    }

    // Fold the tree into the peripheral map: a *live* blob overrides the compiled-in
    // constants for peripheral consumers (never the bundled fixture, never a row the
    // walker did not find, never the console — see board.rs "letting the machine
    // override the map"). On QEMU's -kernel path this is a no-op by design.
    board::resolve();
    {
        let r = board::resolved();
        if r.live {
            kprintln!(
                "[mansa] peripheral map resolved from the live tree: GIC @ {:#x} ({}), RTC @ {:#x} ({})",
                r.gic_dist,
                r.gic_dist_src.as_str(),
                r.rtc,
                r.rtc_src.as_str()
            );
        } else {
            kprintln!("[mansa] peripheral map: compile-time (no live tree to override it)");
        }
    }

    // Register the always-present platform devices with the Device Manager.
    asili::register("pl011-uart", asili::Class::Console, board::UART_BASE as u64, true);
    asili::register("fw-cfg", asili::Class::Firmware, board::FWCFG_BASE as u64, true);

    // Bring up the framebuffer console early so the rest of boot mirrors on screen.
    let display_ok = framebuffer::init();
    asili::register("ramfb", asili::Class::Display, 0, display_ok);
    if display_ok {
        kprintln!("[mansa] display online — framebuffer console ({}x{})", framebuffer::WIDTH, framebuffer::HEIGHT);
    }

    // 2) Physical frame allocator over RAM above the kernel image.
    //
    // The top of RAM comes from `board::resolved()`, not from a raw `fdt` read
    // (2.731). The two differ on exactly the machines the last five milestones were
    // about: a bundled fixture tree, another board's tree and an impossible /memory
    // claim are all refused there, and the allocator is the largest consumer on the
    // board — it was the last one still walking around those rules. The identity-map
    // clamp moved into `frame::plan`, which now *names* the bound instead of
    // silently applying `.min()` here.
    let kend = addr_of!(__kernel_end) as usize;
    let ram_end = board::resolved().ram_end() as usize;
    frame::init(kend, ram_end);
    kprintln!(
        "[mansa] frame allocator: {} frames ({} MiB) from {:#x} (RAM top {:#x}, {})",
        frame::total_frames(),
        frame::total_frames() * frame::FRAME_SIZE / (1024 * 1024),
        kend,
        ram_end,
        board::resolved().ram_src.as_str()
    );
    if let Some(why) = frame::bound() {
        kprintln!(
            "[mansa]   ! bounded: {} frames ({} MiB) of the resolved range are unmanaged — {}",
            frame::unmanaged_frames(),
            frame::unmanaged_frames() * frame::FRAME_SIZE / (1024 * 1024),
            why
        );
    }
    if let (Some(a), Some(b)) = (frame::alloc(), frame::alloc()) {
        kprintln!("[mansa]   demo frames: {:#x}, {:#x}", a, b);
        frame::free(a);
        frame::free(b);
    }

    // 3) Kernel heap is live (global allocator). Prove alloc works.
    {
        use alloc::vec::Vec;
        let mut v: Vec<u32> = Vec::new();
        for i in 0..8 {
            v.push(i * i);
        }
        kprintln!(
            "[mansa] heap live ({} KiB) — Vec demo {:?}, used {} B",
            heap::size() / 1024,
            v.as_slice(),
            heap::used()
        );
    }

    // 4) Interrupts: vector table (incl. the SVC syscall path), GIC, timer.
    exceptions::init();
    gic::init();
    timer::init();
    // Interrupt-driven console input: route the PL011 RX interrupt to this core
    // and unmask it at the UART. Virt-only (INTID 33 / GICv2 / PL011); the Pi
    // uses a different UART + interrupt controller and stays on polling.
    #[cfg(not(feature = "board-raspi"))]
    {
        gic::enable(uart::UART_RX_INTID);
        uart::init_irq();
    }
    // The Device Manager reports the address actually in use, which after a live
    // tree is not necessarily the compiled-in one.
    asili::register("gicv2", asili::Class::Interrupt, board::resolved().gic_dist as u64, true);
    asili::register("arm-generic-timer", asili::Class::Timer, 0, true);
    asili::register("pl031-rtc", asili::Class::Timer, board::resolved().rtc as u64, true);
    #[cfg(not(feature = "board-raspi"))]
    asili::register("pl061-gpio", asili::Class::Gpio, board::GPIO_BASE as u64, gpio::probe());

    // Enumerate the virtio-mmio bus and negotiate with the audio device (if any).
    virtio::init();
    kprintln!("[mansa] exceptions + GICv2 + generic timer online");

    // Auto-configure networking via DHCP when a net device is present. On a
    // netless boot (the default smoke-test path) net_present() is false and this
    // is skipped entirely, so nothing waits on a network that isn't there.
    if virtio::net_present() {
        virtio::net_dhcp();
    }

    // SMP: try to bring up a second CPU core (harmless if only one is present).
    bring_up_secondary();

    // 5) Userspace: register idle, then run the capability-secured IPC demo.
    //    Two EL0 processes, private address spaces, syscalls via SVC, and an
    //    IPC message that only flows because the sender holds a SEND capability.
    process::init();
    kprintln!("[mansa] process subsystem up — entering userspace demo");
    kprintln!();
    process::run_demo(); // returns when both user processes have exited
    kprintln!();
    kprintln!("[mansa] ===== capability-secured IPC demo complete =====");
    kprintln!("[mansa]   P1 sent 0x42; P2 received it; P2's illegal SEND was refused.");
    kprintln!("[mansa]   EL0 isolation + SVC syscalls + capabilities: PROVEN.");
    kprintln!("[mansa] Phase 0 deliverable met. Starting interactive shell.");

    // 6) Mount the in-RAM filesystem and register it, then drop into the shell.
    vfs::seed();
    // Seed two real .wasm apps as files so the user can `nia <file>` them.
    vfs::write("hello.wasm", nia::hello_wasm());
    vfs::write("sum.wasm", nia::sum_wasm());
    vfs::write("draw.wasm", nia::draw_wasm());
    vfs::write("fileapp.wasm", nia::file_wasm());
    vfs::write("clock.wasm", nia::clock_wasm());
    vfs::write("type.wasm", nia::type_wasm());
    vfs::write("ai.wasm", nia::ai_wasm());
    vfs::write("chat.wasm", nia::chat_wasm());
    vfs::write("notify.wasm", nia::notify_wasm());
    vfs::write("notify2.wasm", nia::notify2_wasm());
    vfs::write("net.wasm", nia::net_wasm());
    vfs::write("rand.wasm", nia::rand_wasm());
    vfs::write("sample.webapp", web::sample());
    vfs::write("sample.apkmeta", daraja::sample());
    asili::register("ramfs", asili::Class::Storage, 0, true);

    // If a virtio-blk disk holds a previously-synced filesystem, restore it
    // (replacing the seeded defaults) so files persist across reboots.
    if virtio::blk_present() {
        vfs::load_from_disk();
    }

    // Load/refresh persistent system config (hostname, DHCP lease, boot counter)
    // and bump the boot count. No-op when there is no disk to persist to.
    config::boot_apply();

    // Record each installed app's expected SHA-256 for AfriStore integrity checks,
    // after validating the hash against the FIPS test vector.
    kprintln!("[mansa] sha256 self-test (FIPS \"abc\"): {}", if sha256::self_test() { "PASS" } else { "FAIL" });
    kprintln!("[mansa] sha512 self-test (FIPS \"abc\"): {}", if sha512::self_test() { "PASS" } else { "FAIL" });
    kprintln!("[mansa] ed25519 self-test (RFC 8032 #1): {}", if ed25519::self_test() { "PASS" } else { "FAIL" });
    for name in [
        "hello.wasm", "sum.wasm", "draw.wasm", "fileapp.wasm", "clock.wasm", "type.wasm", "ai.wasm",
        "chat.wasm", "notify.wasm",
    ] {
        if let Some(d) = vfs::read_raw(name) {
            integrity::record(name, d);
        }
    }

    cpu::irq_enable(); // timer keeps ticking
    shell::run();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // 2.736: the message is rendered into a fixed buffer instead of straight to the
    // UART, because it has to be READ before it is printed. The kernel's small
    // allocations (`Vec`, `String`, `format!`) go through the infallible `alloc` API,
    // which cannot hand a caller a bound the way `heap::alloc_stack` now does — a full
    // arena surfaces here, as `memory allocation of N bytes failed`, a line that names a
    // byte count and nothing else. `#[alloc_error_handler]` would catch it earlier, but
    // it is unstable and this toolchain is pinned to stable, so this is the last place
    // the failure can be attributed to the arena that caused it.
    //
    // Nothing on this path allocates: the buffer is on the stack and `kprintln!` formats
    // straight to the UART. A panic handler that needed the heap would be a second panic
    // on a full one. 256 bytes comfortably holds the allocation message, which is what
    // the classification below reads; a longer panic is printed truncated and says so.
    let mut buf = [0u8; 256];
    let (text, dropped) = fmtbuf::render_lossy(&mut buf, format_args!("{}", info));
    if dropped > 0 {
        kprintln!("[mansa] KERNEL PANIC: {}... (+{} B truncated)", text, dropped);
    } else {
        kprintln!("[mansa] KERNEL PANIC: {}", text);
    }
    if let Some(n) = heap::alloc_failure_size(text) {
        heap::report_exhaustion(n);
    }
    loop {
        cpu::wfi();
    }
}
