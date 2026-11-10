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

/// COM1 answers: its scratch register keeps what is written to it. A laptop without one
/// (owner's HP ENVY, 8 Nov 2026: blank screen) must not spend a bounded wait on every byte.
static PRESENT: AtomicBool = AtomicBool::new(true);

pub fn present() -> bool {
    PRESENT.load(Ordering::Relaxed)
}

pub fn init() {
    // SAFETY: COM1's scratch register (offset 7): a 16550 holds the byte; nothing behind the
    // port reads back 0xFF (or anything else), whatever was written.
    let here = unsafe {
        outb(COM1 + 7, 0x5A);
        let a = inb(COM1 + 7);
        outb(COM1 + 7, 0xA5);
        let b = inb(COM1 + 7);
        a == 0x5A && b == 0xA5
    };
    PRESENT.store(here, Ordering::Relaxed);
    if !here {
        return;
    }
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
    if !PRESENT.load(Ordering::Relaxed) {
        return;
    }
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
        put(b);
    }
}

/// One byte into the ring (false if full: counted as a drop), and into the line tracker.
/// Caller has interrupts off.
unsafe fn put(b: u8) -> bool {
    let next = (RX_TAIL + 1) % RING;
    if next == RX_HEAD {
        RX_DROPS.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    RX_RING[RX_TAIL] = b;
    RX_TAIL = next;
    track(b);
    true
}

// Shell history (9 Nov 2026, HP ENVY field item F5: the arrow keys did nothing at the
// shell). The shared line editor (shell.rs read_line) is left as it is, so the ARM image
// does not change: this file follows every byte that enters the ring the way read_line
// will take it (printable bytes add, 0x08/0x7f remove, CR/LF ends the line), keeps the
// last HIST lines, and Up/Down (from ps2.rs and xhci.rs) put DEL bytes for the line being
// typed and then the recalled line into the ring, as if they had been typed.
const HIST: usize = 16;
const LINE: usize = 127; // shell.rs MAX_LINE - 1: the longest line read_line keeps
static mut CUR: [u8; LINE] = [0; LINE];
static mut CUR_LEN: usize = 0;
static mut HBUF: [[u8; LINE]; HIST] = [[0; LINE]; HIST];
static mut HLEN: [usize; HIST] = [0; HIST];
static mut HCOUNT: usize = 0; // lines ever kept; the newest is in slot (HCOUNT - 1) % HIST
static mut BROWSE: usize = 0; // 0: the line being typed; k: the k-th newest kept line
static RECALLS: AtomicU64 = AtomicU64::new(0);
static RECALL_DROPS: AtomicU64 = AtomicU64::new(0);

/// Follow one byte as read_line will take it; a finished line joins the history (not an
/// empty one, nor the same line twice in a row). Caller has interrupts off.
unsafe fn track(b: u8) {
    let cur = &mut *core::ptr::addr_of_mut!(CUR);
    match b {
        b'\r' | b'\n' => {
            let n = CUR_LEN;
            if n > 0 {
                let (hb, hl) = (&mut *core::ptr::addr_of_mut!(HBUF), &mut *core::ptr::addr_of_mut!(HLEN));
                let last = (HCOUNT + HIST - 1) % HIST;
                if HCOUNT == 0 || hb[last][..hl[last]] != cur[..n] {
                    let slot = HCOUNT % HIST;
                    hb[slot][..n].copy_from_slice(&cur[..n]);
                    hl[slot] = n;
                    HCOUNT += 1;
                }
            }
            CUR_LEN = 0;
            BROWSE = 0;
        }
        0x08 | 0x7f => CUR_LEN = CUR_LEN.saturating_sub(1),
        0x20..=0x7e if CUR_LEN < LINE => {
            cur[CUR_LEN] = b;
            CUR_LEN += 1;
        }
        _ => {}
    }
}

/// Up (older) or Down (newer) at the shell: the line being typed is replaced by a kept
/// one (Down past the newest gives an empty line). Called from the keyboards' timer-
/// interrupt polls, so interrupts are off. Nothing happens if the ring has no room.
pub fn history_key(up: bool) {
    // SAFETY: interrupts are off in the caller; `getc` takes the ring with them off.
    unsafe {
        let have = HCOUNT.min(HIST);
        let to = match up {
            true if BROWSE < have => BROWSE + 1,
            false if BROWSE > 0 => BROWSE - 1,
            _ => return,
        };
        let mut line = [0u8; LINE];
        let n = if to == 0 {
            0
        } else {
            let slot = (HCOUNT - to) % HIST;
            let n = (*core::ptr::addr_of!(HLEN))[slot];
            line[..n].copy_from_slice(&(&*core::ptr::addr_of!(HBUF))[slot][..n]);
            n
        };
        let free = (RX_HEAD + RING - RX_TAIL - 1) % RING;
        if free < CUR_LEN + n {
            RECALL_DROPS.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for _ in 0..CUR_LEN {
            put(0x7f);
        }
        for &b in &line[..n] {
            put(b);
        }
        BROWSE = to;
        RECALLS.fetch_add(1, Ordering::Relaxed);
    }
}

/// `history`: the kept lines, oldest first, numbered from the first line typed.
pub fn history_list() {
    crate::cpu::cli();
    let (count, mut lines) = (unsafe { HCOUNT }, [([0u8; LINE], 0usize); HIST]);
    // SAFETY: interrupts are off, so no keyboard poll changes the history meanwhile.
    unsafe {
        for (i, l) in lines.iter_mut().enumerate().take(count.min(HIST)) {
            let slot = (count.saturating_sub(HIST) + i) % HIST;
            let n = (*core::ptr::addr_of!(HLEN))[slot];
            l.0[..n].copy_from_slice(&(&*core::ptr::addr_of!(HBUF))[slot][..n]);
            l.1 = n;
        }
    }
    crate::cpu::sti();
    let first = count.saturating_sub(HIST);
    for (i, (b, n)) in lines.iter().take(count.min(HIST)).enumerate() {
        crate::kprintln!("{:5}  {}", first + i + 1, core::str::from_utf8(&b[..*n]).unwrap_or("?"));
    }
    let (rc, rd) = (RECALLS.load(Ordering::Relaxed), RECALL_DROPS.load(Ordering::Relaxed));
    crate::kprintln!("history: {} line(s) typed, the last {} kept; {} recalled with Up/Down", count, count.min(HIST), rc);
    if rd > 0 {
        crate::kprintln!("history: {} Up/Down press(es) not recalled (the input ring was full)", rd);
    }
}

/// One byte from the hardware, if the 16550 holds one (counts overruns).
fn hw_byte() -> Option<u8> {
    if !PRESENT.load(Ordering::Relaxed) {
        return None;
    }
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
            put(b);
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
            hw_byte().map(|b| {
                track(b); // straight from the port (before the interrupt is on)
                b
            })
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
        crate::conback::live(); // a scrolled-back console comes back first (conback.rs)
        crate::conback::feed(s);
        crate::report::feed(s); // kept for `report` (the boot log and the latest lines)
        let _ = Com1.write_str(s);
        crate::early_write(s); // the boot trail on the real screen, until the shell starts
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
    if crate::helpview::on() {
        crate::helpview::feed(args); // `help <word>` / `help <page>` (helpview.rs)
    } else {
        let _ = Tee.write_fmt(args);
    }
    PRINT_LOCK.store(false, Ordering::Release);
    if flags & 0x200 != 0 {
        crate::cpu::sti();
    }
}

/// Everywhere a print goes, with the print lock already held: helpview.rs's lines.
pub fn tee_str(s: &str) {
    let _ = Tee.write_str(s);
}

/// The same, formatted.
pub fn tee_fmt(args: core::fmt::Arguments) {
    let _ = Tee.write_fmt(args);
}

/// COM1 only (not the screen): conback.rs's notes, with the print lock already held.
pub fn com1_fmt(args: core::fmt::Arguments) {
    let _ = Com1.write_fmt(args);
}

/// Shift+PgUp (`up`) / Shift+PgDn from the keyboards' polls (interrupts off): the console
/// scrollback, under the print lock so the other core's prints wait (conback.rs).
pub fn console_page(up: bool) {
    while PRINT_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    crate::conback::page(up);
    PRINT_LOCK.store(false, Ordering::Release);
}

/// Mouse-wheel notches (ps2.rs): scroll the console three lines a notch, under the print lock.
pub fn console_wheel(notches: i32) {
    while PRINT_LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    crate::conback::wheel(notches);
    PRINT_LOCK.store(false, Ordering::Release);
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
