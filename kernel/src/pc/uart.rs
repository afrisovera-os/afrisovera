//! The PC console: the 16550 UART at COM1 (I/O port 0x3F8).
//!
//! Named `uart` and exporting `kprint!`/`kprintln!` with the same expansion as the ARM
//! kernel's `uart.rs`, so the shared modules (`heap.rs` and the rest) print through
//! this file unchanged. Every PC chipset and QEMU's `q35` decode COM1 at 0x3F8; on a
//! laptop with no serial header the bytes simply go nowhere and the screen is the
//! console.

use crate::cpu::{inb, outb};
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const COM1: u16 = 0x3F8;

pub fn init() {
    // SAFETY: COM1's registers; the standard 16550 programming sequence.
    unsafe {
        outb(COM1 + 1, 0x00); // no UART interrupts: the console is polled
        outb(COM1 + 3, 0x80); // DLAB on, to set the divisor
        outb(COM1, 0x01); //     divisor 1 = 115200 baud
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit; DLAB off
        outb(COM1 + 2, 0xC7); // FIFOs on and cleared
        outb(COM1 + 4, 0x0B); // DTR + RTS + OUT2
    }
}

fn write_byte(b: u8) {
    // SAFETY: polling the line-status register, then writing the data register.
    unsafe {
        let mut spins = 0u32;
        // Bounded: a port with nothing behind it must not hang the kernel.
        while inb(COM1 + 5) & 0x20 == 0 && spins < 100_000 {
            spins += 1;
        }
        outb(COM1, b);
    }
}

// ---- Console input (the AfriSh shell, PC track step 2) --------------------------------
// The same interface as the ARM `uart.rs` (getc, driver_name and the counters the shell
// prints). Once `enable_rx_irq` has run, input is INTERRUPT-DRIVEN like the ARM PL011:
// COM1's receive interrupt (ISA IRQ 4 -> IOAPIC -> `cpu::COM1_VECTOR`) drains the 16550
// FIFO into a software ring and `getc` takes bytes from the ring. Before that (or on a PC
// it) `getc` polls the port instead. Interrupts are held off while `getc` looks, so a
// byte the handler takes can never overtake one already waiting.
static RX_BYTES: AtomicU64 = AtomicU64::new(0);
static RX_OVERRUNS: AtomicU64 = AtomicU64::new(0);
static RX_IRQS: AtomicU64 = AtomicU64::new(0);
static RX_DROPS: AtomicU64 = AtomicU64::new(0);
static IDLE_WFI: AtomicU64 = AtomicU64::new(0);
static IRQ_ON: AtomicBool = AtomicBool::new(false);

const RING: usize = 256;
static mut RX_RING: [u8; RING] = [0; RING];
static mut RX_HEAD: usize = 0; // next byte to hand out
static mut RX_TAIL: usize = 0; // next free slot

/// A byte typed on another keyboard (the USB one, xhci.rs) joins the console input ring as
/// if COM1 had received it. Called with interrupts off (from the timer interrupt).
pub fn inject(b: u8) {
    // SAFETY: interrupts are off in the caller; `getc` takes the ring with them off.
    unsafe {
        let next = (RX_TAIL + 1) % RING;
        if next == RX_HEAD {
            RX_DROPS.fetch_add(1, Ordering::Relaxed);
        } else {
            RX_RING[RX_TAIL] = b;
            RX_TAIL = next;
        }
    }
}

/// One byte from the hardware, if the 16550 holds one (counts overruns).
fn hw_byte() -> Option<u8> {
    // SAFETY: COM1's line-status and receive-buffer registers.
    unsafe {
        let lsr = inb(COM1 + 5);
        if lsr == 0xFF {
            return None; // no UART decodes the port (a laptop without COM1): nothing to read
        }
        if lsr & 0x02 != 0 {
            RX_OVERRUNS.fetch_add(1, Ordering::Relaxed);
        }
        if lsr & 0x01 == 0 {
            return None;
        }
        Some(inb(COM1))
    }
}

/// Turn on COM1's receive interrupt and route IRQ 4 to it (after `cpu::lapic_init`).
pub fn enable_rx_irq() {
    crate::cpu::ioapic_route(4, crate::cpu::COM1_VECTOR);
    // SAFETY: COM1's interrupt-enable register: "received data available" only.
    unsafe { outb(COM1 + 1, 0x01) };
    IRQ_ON.store(true, Ordering::Relaxed);
    // bytes already waiting raised no edge: take them now, so the line can rise again
    crate::cpu::cli();
    rx_irq();
    RX_IRQS.fetch_sub(1, Ordering::Relaxed); // that was a drain, not an interrupt
    crate::cpu::sti();
}

/// The COM1 interrupt: move every byte the FIFO holds into the ring.
pub fn rx_irq() {
    RX_IRQS.fetch_add(1, Ordering::Relaxed);
    while let Some(b) = hw_byte() {
        // SAFETY: interrupts are off inside the handler; `getc` takes the ring with them off.
        unsafe {
            let next = (RX_TAIL + 1) % RING;
            if next == RX_HEAD {
                RX_DROPS.fetch_add(1, Ordering::Relaxed);
            } else {
                RX_RING[RX_TAIL] = b;
                RX_TAIL = next;
            }
        }
    }
}

/// One received byte, if one is waiting (from the ring, else from the port).
pub fn getc() -> Option<u8> {
    let flags: u64;
    // SAFETY: save RFLAGS and mask interrupts while the ring and the port are read together.
    unsafe { core::arch::asm!("pushfq; pop {}; cli", out(reg) flags) };
    // SAFETY: interrupts are masked (above), so the handler cannot run meanwhile.
    let b = unsafe {
        if RX_HEAD != RX_TAIL {
            let b = RX_RING[RX_HEAD];
            RX_HEAD = (RX_HEAD + 1) % RING;
            Some(b)
        } else if IRQ_ON.load(Ordering::Relaxed) {
            None // the interrupt fills the ring; nothing waiting yet
        } else {
            hw_byte()
        }
    };
    if flags & 0x200 != 0 {
        crate::cpu::sti();
    }
    if b.is_some() {
        RX_BYTES.fetch_add(1, Ordering::Relaxed);
    }
    b
}

pub fn driver_name() -> &'static str {
    if IRQ_ON.load(Ordering::Relaxed) {
        "16550 (COM1 @ port 0x3f8, IRQ 4 via IOAPIC, interrupt-driven)"
    } else {
        "16550 (COM1 @ port 0x3f8, polled)"
    }
}

/// COM1 receive interrupts serviced so far.
pub fn rx_irq_count() -> u64 {
    RX_IRQS.load(Ordering::Relaxed)
}

/// Bytes dropped because the software ring was full.
pub fn rx_drop_count() -> u64 {
    RX_DROPS.load(Ordering::Relaxed)
}

/// Times the 16550 reported a receive overrun (LSR bit 1).
pub fn rx_overrun_count() -> u64 {
    RX_OVERRUNS.load(Ordering::Relaxed)
}

pub fn note_idle_wfi() {
    IDLE_WFI.fetch_add(1, Ordering::Relaxed);
}

pub fn idle_wfi_count() -> u64 {
    IDLE_WFI.load(Ordering::Relaxed)
}

/// COM1 and the framebuffer console (shown on the GOP screen by screen.rs), as the ARM
/// kernel's `uart::Tee` sends every print to the PL011 and its ramfb console.
struct Tee;
impl Write for Tee {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = Com1.write_str(s);
        crate::xhci::serial_write(s); // and a USB serial cable, when one is plugged in
        crate::framebuffer::console_write(s);
        crate::screen::mark();
        Ok(())
    }
}

struct Com1;
impl Write for Com1 {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for b in s.bytes() {
            if b == b'\n' {
                write_byte(b'\r');
            }
            write_byte(b);
        }
        Ok(())
    }
}

/// One line at a time from either core: a spin lock, held with interrupts off on this core
/// (so a handler on the same core can never wait for a lock its own core holds).
static PRINT_LOCK: AtomicBool = AtomicBool::new(false);

pub fn _print(args: core::fmt::Arguments) {
    let flags: u64;
    // SAFETY: save RFLAGS and mask interrupts on this core for the length of the print.
    unsafe { core::arch::asm!("pushfq; pop {}; cli", out(reg) flags) };
    while PRINT_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    let _ = Tee.write_fmt(args);
    PRINT_LOCK.store(false, Ordering::Release);
    if flags & 0x200 != 0 {
        crate::cpu::sti();
    }
}

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
