//! Minimal PL011 UART driver — Mansa's first "voice" — and the console facade
//! every other module prints through.
//!
//! On QEMU `-M virt` the primary PL011 lives at MMIO base 0x0900_0000 and is
//! already usable at reset, so Phase 0 only needs to write bytes to the data
//! register. A real driver (baud/FIFO/IRQ setup) becomes an *Asili userspace
//! driver* later — the kernel keeps only this bring-up console.
//!
//! Since milestone 2.724 the PL011 is no longer the *only* console part. [`Uart`]
//! is the interface (`write_byte`/`read_byte`, and everything layered on top:
//! `kprintln!`, the framebuffer tee, the RX ring, the cross-core console lock); the
//! PL011 code below is one implementation of it and `uart_ns16550.rs` is the other,
//! for boards whose UART is a DesignWare 8250. The choice is the compile-time
//! constant [`crate::board::UART_IS_8250`], derived from the active board's
//! `uart_kind`, so exactly one implementation survives optimization and no caller
//! has to know which board it is on.

use core::fmt::{self, Write};
use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// PL011 MMIO base on the QEMU `virt` board.
const PL011_BASE: usize = crate::board::UART_BASE;
/// Data register offset (write a byte here to transmit; read to receive).
const UART_DR: usize = 0x00;
/// Flag register offset; bit 5 (TXFF) set = TX FIFO full, bit 4 (RXFE) = RX empty.
const UART_ECR: usize = 0x04;  // error-clear (write) / RSR (read) — same offset
const UART_FR: usize = 0x18;
/// Line-control register; bit 4 (FEN) enables the 16-deep RX/TX FIFOs.
const UART_LCRH: usize = 0x2C;
/// Interrupt mask set/clear register; bit 4 (RXIM) + bit 6 (RTIM) enable RX.
const UART_IMSC: usize = 0x38;
/// Interrupt clear register; write-1-to-clear per source.
const UART_ICR: usize = 0x44;
const FR_TXFF: u32 = 1 << 5;
const FR_RXFE: u32 = 1 << 4;
/// Overrun flag as it arrives in DR[11] alongside the character it applies to.
const DR_OE: u32 = 1 << 11;
const LCRH_FEN: u32 = 1 << 4; // FIFO enable
const INT_RX: u32 = 1 << 4; // RX interrupt (FIFO passed its trigger level)
const INT_RT: u32 = 1 << 6; // RX timeout (FIFO non-empty but idle — drains the tail)
const INT_RXALL: u32 = INT_RX | INT_RT;

/// GIC INTID of the PL011 UART on the QEMU `virt` board (SPI 1 = 32 + 1).
pub const UART_RX_INTID: u32 = 33;

pub struct Uart;

impl Uart {
    #[inline]
    fn reg(off: usize) -> *mut u32 {
        (PL011_BASE + off) as *mut u32
    }

    /// Transmit one byte on whichever console part this board has.
    pub fn write_byte(&self, b: u8) {
        if crate::board::UART_IS_8250 {
            // A dropped byte (the 8250 driver's bounded TX spin gave up) is not
            // reportable here — the facade has no error channel and every caller
            // up to `kprintln!` is infallible. Losing output beats hanging.
            let _ = crate::uart_ns16550::CONSOLE.write_byte(b);
            return;
        }
        // SAFETY: fixed, valid MMIO addresses on the QEMU virt board; Phase 0
        // is single-core with the boot thread as the only writer.
        unsafe {
            while core::ptr::read_volatile(Self::reg(UART_FR)) & FR_TXFF != 0 {
                core::hint::spin_loop();
            }
            core::ptr::write_volatile(Self::reg(UART_DR), b as u32);
        }
    }

    /// Non-blocking receive: returns a byte if one is waiting in the RX FIFO.
    pub fn read_byte(&self) -> Option<u8> {
        if crate::board::UART_IS_8250 {
            return crate::uart_ns16550::CONSOLE.read_byte();
        }
        // SAFETY: fixed, valid MMIO addresses on the QEMU virt board.
        unsafe {
            if core::ptr::read_volatile(Self::reg(UART_FR)) & FR_RXFE != 0 {
                None
            } else {
                Some(core::ptr::read_volatile(Self::reg(UART_DR)) as u8)
            }
        }
    }

    pub fn write_str_bytes(&self, s: &str) {
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.write_byte(b'\r'); // terminals want CRLF
            }
            self.write_byte(b);
        }
    }
}

impl Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_str_bytes(s);
        Ok(())
    }
}

/// Bring the console part up far enough to print. Call it first thing in
/// `kernel_main`, before the banner.
///
/// A no-op on PL011 boards — QEMU virt and both Pis hand us a UART that is enabled
/// and running at reset, which is why the kernel got away with having no console
/// init at all until now. An 8250 board needs its FIFOs enabled and flushed first
/// (`uart_ns16550::Ns16550::init`); the baud divisor is firmware's job either way.
pub fn console_init() {
    if crate::board::UART_IS_8250 {
        crate::uart_ns16550::CONSOLE.init();
    }
}

/// Short name of the console driver this build compiled in (`pl011` / `ns16550`).
pub fn driver_name() -> &'static str {
    crate::board::uart_driver_for(crate::board::ACTIVE.uart_kind)
}

/// A sink that tees output to the serial UART **and** the framebuffer console,
/// so every `kprintln!` shows both on the wire and on screen (the latter is a
/// no-op until the framebuffer console is initialized).
struct Tee;
impl Write for Tee {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        Uart.write_str_bytes(s);
        crate::framebuffer::console_write(s);
        Ok(())
    }
}

// --- Interrupt-driven RX (2.87) ---------------------------------------------
// A single-producer (the RX ISR) / single-consumer (the shell/getc) ring. The
// producer only advances HEAD, the consumer only advances TAIL, so on this
// single core no lock is needed — the atomics just provide the happens-before
// between the ISR store and the consumer's read.
const RX_CAP: usize = 256;
static mut RX_RING: [u8; RX_CAP] = [0; RX_CAP];
static RX_HEAD: AtomicUsize = AtomicUsize::new(0);
static RX_TAIL: AtomicUsize = AtomicUsize::new(0);
static RX_IRQS: AtomicU64 = AtomicU64::new(0);
/// Bytes the ISR could not store because the 256-byte ring was FULL — the shell
/// was not draining fast enough. Candidate site (a) for the uart-rx-stress loss.
static RX_DROPS: AtomicU64 = AtomicU64::new(0);
/// Characters whose DR read carried the PL011 overrun flag — the 16-deep hardware
/// FIFO filled before the ISR reached it, so the byte died inside the UART and the
/// ring never saw it. Candidate site (b). Only ONE of these two can move.
static RX_OVERRUNS: AtomicU64 = AtomicU64::new(0);
/// True once RX interrupts are enabled. While true, `getc` reads ONLY from the
/// ring (the ISR is the sole reader of the hardware FIFO) — a direct hardware
/// poll would race the ISR and reorder/duplicate bytes. While false (pre-init,
/// or the Pi which never enables the RX IRQ) `getc` polls the hardware.
static RX_IRQ_ON: AtomicBool = AtomicBool::new(false);

fn ring_push(b: u8) {
    let head = RX_HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % RX_CAP;
    if next == RX_TAIL.load(Ordering::Acquire) {
        // Ring full: drop the byte rather than overwrite unread input. Silent
        // for six milestones — this is the site the 2.892 triage named (a).
        RX_DROPS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // SAFETY: `head` indexes within RX_RING; the ISR is the only writer of cells.
    unsafe { addr_of_mut!(RX_RING).cast::<u8>().add(head).write(b) };
    RX_HEAD.store(next, Ordering::Release);
}

fn ring_pop() -> Option<u8> {
    let tail = RX_TAIL.load(Ordering::Relaxed);
    if tail == RX_HEAD.load(Ordering::Acquire) {
        return None; // empty
    }
    // SAFETY: `tail` indexes a cell the ISR already published (Acquire above).
    let b = unsafe { addr_of_mut!(RX_RING).cast::<u8>().add(tail).read() };
    RX_TAIL.store((tail + 1) % RX_CAP, Ordering::Release);
    Some(b)
}

/// PL011 RX interrupt service routine: drain the hardware RX FIFO into the ring,
/// clear the interrupt, and bump the counter. Called from the IRQ handler.
pub fn rx_isr() {
    if crate::board::UART_IS_8250 {
        // Nothing registers this ISR on an 8250 board (its console is polled, see
        // `init_irq`), but reading PL011 flag/data offsets out of an 8250's window
        // would land on LCR/MCR — so refuse rather than trust the caller.
        return;
    }
    // SAFETY: fixed, valid MMIO on the virt board; runs with IRQs masked.
    unsafe {
        // ACKNOWLEDGE FIRST, THEN DRAIN — never the other way round (2.749).
        // Draining first and clearing afterwards acknowledges any byte that lands
        // in the window between the last FIFO read and the clear WITHOUT taking
        // it, and nothing ever announces it again: RXI re-asserts only as the FIFO
        // count CROSSES the trigger level, and the receive-timeout interrupt that
        // would have covered the straggler is exactly what the clear just wiped.
        // The console then goes permanently deaf mid-command. That is not a theory:
        // scripted commands wedged 1 time in 93 at 17 bytes received — the 16-deep
        // FIFO plus the one byte already read out — and stayed deaf to every later
        // command, which is the signature five suites showed for six milestones.
        // Clearing first means such a byte is either drained by the loop below or
        // leaves its own interrupt pending for the next entry.
        loop {
            write_volatile((PL011_BASE + UART_ICR) as *mut u32, INT_RXALL); // clear RXI + RTI
            let mut drained = false;
            while read_volatile(Uart::reg(UART_FR)) & FR_RXFE == 0 {
                // The PL011 puts this character's error flags in DR[11:8] — OE at
                // bit 11 — so the overrun is ALREADY in this read; the old `as u8`
                // threw it away. Counting it here costs no extra MMIO read, where
                // polling RSR would add one that races the drain loop. Clear via
                // ECR so a later overrun counts as a NEW event, not this one again.
                let dr = read_volatile(Uart::reg(UART_DR));
                if dr & DR_OE != 0 {
                    RX_OVERRUNS.fetch_add(1, Ordering::Relaxed);
                    write_volatile((PL011_BASE + UART_ECR) as *mut u32, 0);
                }
                let b = dr as u8;
                ring_push(b);
                drained = true;
            }
            // Empty at a point AFTER a clear: nothing is stranded behind an
            // interrupt we already acknowledged, so this entry is done.
            if !drained {
                break;
            }
        }
    }
    RX_IRQS.fetch_add(1, Ordering::Relaxed);
}

/// Enable the PL011 RX interrupt (RXIM in IMSC). Call once at boot after the GIC
/// has the UART INTID enabled. Virt-only (the Pi uses a different UART/IRQ path).
pub fn init_irq() {
    if crate::board::UART_IS_8250 {
        // The 8250 console stays polled: its RX interrupt is a different register
        // (IER) *and* a different INTID, and on the A64 nothing has brought up an
        // interrupt controller anyway. Leaving RX_IRQ_ON false routes `getc`
        // through the hardware poll, which the 8250 driver implements.
        return;
    }
    // SAFETY: fixed, valid MMIO on the virt board.
    unsafe {
        // Enable the 16-deep RX FIFO so a burst of typed bytes (e.g. a pasted or
        // fast-sent command) is buffered in hardware instead of overrunning the
        // 1-byte holding register between interrupts.
        let lcrh = (PL011_BASE + UART_LCRH) as *mut u32;
        write_volatile(lcrh, read_volatile(lcrh) | LCRH_FEN);
        // Unmask RX (FIFO trigger level) and RX-timeout (drains the last few
        // bytes that never reach the trigger level) interrupts.
        let imsc = (PL011_BASE + UART_IMSC) as *mut u32;
        write_volatile(imsc, read_volatile(imsc) | INT_RXALL);
    }
    RX_IRQ_ON.store(true, Ordering::Release);
}

/// Number of PL011 RX interrupts serviced since boot (0 if still polling only).
pub fn rx_irq_count() -> u64 {
    RX_IRQS.load(Ordering::Relaxed)
}

/// Bytes dropped on a FULL ring (consumer too slow) — site (a).
pub fn rx_drop_count() -> u64 {
    RX_DROPS.load(Ordering::Relaxed)
}

/// Characters that arrived flagged OVERRUN by the PL011 (ISR too slow) — site (b).
pub fn rx_overrun_count() -> u64 {
    RX_OVERRUNS.load(Ordering::Relaxed)
}

/// Times the console loop slept on `wfi` waiting for input (idle-power proof).
static IDLE_WFI: AtomicU64 = AtomicU64::new(0);

/// Record one idle `wfi` — the console had no key ready and is about to sleep.
pub fn note_idle_wfi() {
    IDLE_WFI.fetch_add(1, Ordering::Relaxed);
}

/// Number of times the console idled on `wfi` since boot.
pub fn idle_wfi_count() -> u64 {
    IDLE_WFI.load(Ordering::Relaxed)
}

/// Non-blocking read of one byte from the console UART. Once RX interrupts are
/// enabled, prefers the interrupt-filled ring (the ISR owns the hardware FIFO; a
/// concurrent poll would race it and reorder bytes). If the ring is empty it masks
/// IRQs and polls the hardware anyway: with the ISR unable to run on this core the
/// race the ring exists to avoid cannot happen, and a byte no interrupt ever
/// announced — inside an SVC handler where the RX ISR can't run at all (the `edit`
/// app's blocking getkey), or stranded in the FIFO by a lost edge — is still read.
/// Before the IRQ is enabled (early boot, or the Pi which stays on polling) always
/// polls.
pub fn getc() -> Option<u8> {
    if RX_IRQ_ON.load(Ordering::Acquire) {
        if let Some(b) = ring_pop() {
            return Some(b);
        }
        // Ring empty. Look at the hardware too, with IRQs masked so the ISR cannot
        // be draining it underneath us on this core — the reason the poll was
        // avoided in the first place. This is the same direct read the SVC path
        // already relied on, now on every empty-ring poll, and it is what makes a
        // LOST interrupt survivable: a byte the ISR was never told about would
        // otherwise sit in the FIFO forever, because a single byte in an empty FIFO
        // does not reach the trigger level. Costs one MMIO read on an idle poll,
        // and the timer tick guarantees the poll happens even from `wfi` (2.749).
        let prev = crate::cpu::irq_save();
        let b = ring_pop().or_else(|| Uart.read_byte());
        crate::cpu::irq_restore(prev);
        b
    } else {
        Uart.read_byte()
    }
}

/// Write formatted output atomically w.r.t. timer preemption. IRQs are masked
/// for the duration so a line can't be interleaved by another thread mid-print
/// (the IRQ path itself never prints, so there is no re-entrancy/deadlock).
pub fn _print(args: core::fmt::Arguments) {
    let prev = console_lock();
    let _ = Tee.write_fmt(args);
    console_unlock(prev);
}

/// Cross-core console spinlock. On a multi-core boot (SMP) the second core also
/// prints; without a real lock the two cores' bytes interleave and corrupt the
/// output. This serializes whole messages. It is taken with IRQs masked on the
/// calling core (so no timer preemption inside the critical section) and is only
/// ever held for the duration of one message, so it can never deadlock across
/// cores. Callers using exclusives here must be MMU-on (true for the primary
/// always, and for the secondary after it enables translation).
static CONSOLE_LOCK: AtomicU32 = AtomicU32::new(0);

/// Acquire the console lock; returns the saved DAIF to restore on unlock.
pub fn console_lock() -> u64 {
    let prev = crate::cpu::irq_save();
    while CONSOLE_LOCK
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    prev
}

/// Release the console lock and restore interrupts.
pub fn console_unlock(prev: u64) {
    CONSOLE_LOCK.store(0, Ordering::Release);
    crate::cpu::irq_restore(prev);
}

/// Write a byte string straight to the UART under the console lock (no
/// framebuffer tee). Used by the secondary core once it is MMU-on, so its
/// messages serialize against the primary's `kprintln!` without routing through
/// the shared framebuffer console state.
pub fn locked_bytes(s: &[u8]) {
    let prev = console_lock();
    let u = Uart;
    for &b in s {
        u.write_byte(b);
    }
    console_unlock(prev);
}

/// `kprintln!`-style kernel console output.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        $crate::uart::_print(format_args!($($arg)*));
    }};
}

#[macro_export]
macro_rules! kprintln {
    () => {{ $crate::uart::_print(format_args!("\n")); }};
    ($($arg:tt)*) => {{
        $crate::uart::_print(format_args!("{}\n", format_args!($($arg)*)));
    }};
}
