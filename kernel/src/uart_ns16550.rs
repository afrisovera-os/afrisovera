//! DesignWare 8250/16550 UART driver — the console for boards whose primary UART
//! is **not** a PL011.
//!
//! Milestone 2.723 gave PINE64 (Allwinner A64) a peripheral map and was careful to
//! say the map was not a port: the A64's UART0 at `0x01C2_8000` is a DesignWare
//! 8250/16550-class part, so pointing the PL011 driver at that address would drive
//! the wrong registers. This module is the missing half — a second driver behind
//! the same console interface, selected per board by [`crate::board::UART_IS_8250`].
//!
//! ## Register layout (and why the offsets are ×4)
//!
//! A 16550 has eight byte-wide registers. The Allwinner (and most ARM DesignWare)
//! integration exposes them as **32-bit registers on a 4-byte stride** — device
//! trees spell this `reg-shift = <2>` — so register *index* `i` lives at byte
//! offset `i << 2`:
//!
//! | reg | index | offset | meaning |
//! |---|---|---|---|
//! | `THR` / `RBR` | 0 | `+0x00` | write = transmit byte, read = received byte |
//! | `IER` | 1 | `+0x04` | interrupt enable (we leave it masked: polled console) |
//! | `FCR` / `IIR` | 2 | `+0x08` | write = FIFO control, read = interrupt id |
//! | `LCR` | 3 | `+0x0c` | line control (word length, DLAB) |
//! | `MCR` | 4 | `+0x10` | modem control |
//! | `LSR` | 5 | `+0x14` | line status — bit 0 `DR` (rx ready), bit 5 `THRE` (tx room) |
//!
//! Getting this shift wrong is the single most likely way an 8250 port produces
//! silence, which is exactly why [`selftest`] pins the offsets down.
//!
//! ## Baud rate: deliberately not touched
//!
//! Programming the divisor means setting `LCR.DLAB`, writing `DLL`/`DLH`, and
//! knowing the UART input clock — which on the A64 comes from the CCU and is a
//! board/firmware concern. Both realistic entry paths (ATF → U-Boot, or a JTAG
//! load after U-Boot ran) hand us a UART that is **already configured and
//! printing**, so [`Ns16550::init`] only enables and clears the FIFOs and leaves
//! the divisor alone. Reprogramming a working baud rate from a kernel that cannot
//! yet read the clock tree would be a way to *lose* the console, not gain one.
//!
//! ## Honesty about verification
//!
//! There is no QEMU machine for the A64 (`sun50i`) and we do not have the silicon
//! on the bench, so this driver has never driven a real 8250. What *is* verified,
//! on every test run, is its **register discipline**: [`selftest`] points a driver
//! instance at a RAM-backed fake register block and asserts it reads/writes the
//! right offsets, honours `THRE`/`DR`, and — when `THRE` never sets — gives up
//! after a bounded spin instead of hanging the kernel forever. That is the part a
//! port gets wrong on paper; the rest waits for hardware.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

/// Register stride, as a shift: DesignWare integrations map the 16550's byte-wide
/// registers onto a 4-byte grid (`reg-shift = <2>` in the device tree).
pub const REG_SHIFT: usize = 2;

/// Byte offset of 16550 register index `i` under [`REG_SHIFT`].
const fn reg(i: usize) -> usize {
    i << REG_SHIFT
}

/// Transmit holding / receive buffer register (index 0).
pub const THR: usize = reg(0);
/// Receive buffer register — same address as [`THR`], read instead of written.
pub const RBR: usize = reg(0);
/// Interrupt enable register (index 1).
pub const IER: usize = reg(1);
/// FIFO control register (index 2, write-only; reads give IIR).
pub const FCR: usize = reg(2);
/// Line control register (index 3).
pub const LCR: usize = reg(3);
/// Modem control register (index 4).
pub const MCR: usize = reg(4);
/// Line status register (index 5).
pub const LSR: usize = reg(5);

/// `LSR.DR` — a received byte is waiting in `RBR`.
pub const LSR_DR: u32 = 1 << 0;
/// `LSR.THRE` — the transmit holding register is empty (room for a byte).
pub const LSR_THRE: u32 = 1 << 5;

/// `FCR.FIFOE` — enable the RX/TX FIFOs.
const FCR_FIFOE: u32 = 1 << 0;
/// `FCR.RFIFOR` — reset (flush) the RX FIFO.
const FCR_RFIFOR: u32 = 1 << 1;
/// `FCR.XFIFOR` — reset (flush) the TX FIFO.
const FCR_XFIFOR: u32 = 1 << 2;
/// What [`Ns16550::init`] writes: FIFOs on, both directions flushed.
pub const FCR_INIT: u32 = FCR_FIFOE | FCR_RFIFOR | FCR_XFIFOR;

/// Maximum `LSR.THRE` polls before [`Ns16550::write_byte`] drops the byte.
///
/// The PL011 driver spins forever on `TXFF`, which is fine on a board we know
/// boots. This driver's whole reason to exist is a board nobody has ever booted:
/// if the UART clock is not running, an unbounded spin hangs the kernel at its
/// first character with no diagnostic at all. A bounded spin degrades to *lost
/// output* instead of a *dead kernel* — recoverable, and observable as a `false`
/// return. At any sane baud rate this many polls is orders of magnitude longer
/// than one character time, so it never trips on a working UART.
pub const TX_SPIN_LIMIT: u32 = 200_000;

/// A DesignWare 8250/16550 UART at a given MMIO base.
///
/// Carrying the base (rather than reading `board::UART_BASE` directly like the
/// PL011 driver does) is what makes the driver testable off its own board: the
/// self-test points an instance at RAM.
#[derive(Clone, Copy)]
pub struct Ns16550 {
    base: usize,
}

impl Ns16550 {
    /// A driver instance for the UART at `base`.
    pub const fn at(base: usize) -> Self {
        Self { base }
    }

    #[inline]
    fn r(&self, off: usize) -> *mut u32 {
        (self.base + off) as *mut u32
    }

    /// Enable and flush the FIFOs. Leaves the divisor and line format alone (see
    /// the module docs) — firmware configured those before it handed us the port.
    pub fn init(&self) {
        // SAFETY: `base` is either the board's UART MMIO window or the self-test's
        // own scratch block; FCR is a plain write-only register.
        unsafe { write_volatile(self.r(FCR), FCR_INIT) };
    }

    /// Transmit one byte. Returns `false` if `THRE` never came up within
    /// [`TX_SPIN_LIMIT`] polls — the byte is dropped rather than hanging the core.
    pub fn write_byte(&self, b: u8) -> bool {
        let mut spins = 0u32;
        // SAFETY: `base` is valid MMIO (or the self-test scratch block); LSR is
        // read-only and the read has no side effects on a 16550.
        while unsafe { read_volatile(self.r(LSR)) } & LSR_THRE == 0 {
            spins += 1;
            if spins >= TX_SPIN_LIMIT {
                return false; // UART not transmitting: drop, don't hang
            }
            core::hint::spin_loop();
        }
        // SAFETY: as above; THRE is set, so the holding register has room.
        unsafe { write_volatile(self.r(THR), b as u32) };
        true
    }

    /// Non-blocking receive: a byte if `LSR.DR` says one is waiting.
    ///
    /// Reading `RBR` is what *clears* `DR`, so this must not be called
    /// speculatively — check-then-read is the whole protocol.
    pub fn read_byte(&self) -> Option<u8> {
        // SAFETY: `base` is valid MMIO (or the self-test scratch block).
        unsafe {
            if read_volatile(self.r(LSR)) & LSR_DR == 0 {
                None
            } else {
                Some(read_volatile(self.r(RBR)) as u8)
            }
        }
    }
}

/// The console instance for this build. Only ever *used* when
/// [`crate::board::UART_IS_8250`] is true; on a PL011 board the dispatch in
/// `uart.rs` is a constant-false branch the optimizer deletes.
pub const CONSOLE: Ns16550 = Ns16550::at(crate::board::UART_BASE);

// --- Self-test ---------------------------------------------------------------
// A fake register block in RAM. 32 words covers every offset the driver touches
// (LSR at +0x14 is the highest) with room to spare, and u32 alignment matches the
// 4-byte stride the driver assumes.
const SCRATCH_WORDS: usize = 32;
static mut SCRATCH: [u32; SCRATCH_WORDS] = [0; SCRATCH_WORDS];

fn scratch_base() -> usize {
    addr_of_mut!(SCRATCH) as usize
}

/// Read the fake register block at byte offset `off`.
fn peek(off: usize) -> u32 {
    // SAFETY: `off` is one of the module's register constants, all < 128 bytes.
    unsafe { read_volatile((scratch_base() + off) as *const u32) }
}

/// Write the fake register block at byte offset `off` (sets up the "hardware").
fn poke(off: usize, v: u32) {
    // SAFETY: as `peek`.
    unsafe { write_volatile((scratch_base() + off) as *mut u32, v) };
}

fn clear_scratch() {
    for i in 0..SCRATCH_WORDS {
        poke(i * 4, 0);
    }
}

/// Exercise the driver against the RAM-backed fake register block and print a
/// pass/fail line per check. Returns `(passed, total)`.
///
/// This proves register *discipline* — offsets, flag polarity, the check-then-read
/// RX protocol, and the bounded TX spin — not that the driver drives an A64. See
/// the module docs on verification.
pub fn selftest() -> (u32, u32) {
    let dev = Ns16550::at(scratch_base());
    let mut pass = 0u32;
    let mut total = 0u32;
    let mut check = |ok: bool, what: &str| {
        total += 1;
        if ok {
            pass += 1;
            crate::kprintln!("  [ok]   {}", what);
        } else {
            crate::kprintln!("  [FAIL] {}", what);
        }
    };

    // 1) init(): FIFOs enabled + both flushed, and nothing else disturbed.
    clear_scratch();
    poke(LCR, 0x03); // firmware's 8N1 setting — init() must not clobber it
    dev.init();
    check(
        peek(FCR) == FCR_INIT && peek(LCR) == 0x03 && peek(IER) == 0 && peek(MCR) == 0,
        "init: FCR +0x08 = FIFO enable + rx/tx reset; LCR/IER/MCR untouched",
    );

    // 2) TX with room: the byte lands in THR at +0x00, not somewhere else.
    clear_scratch();
    poke(LSR, LSR_THRE);
    let sent = dev.write_byte(b'A');
    check(
        sent && peek(THR) == b'A' as u32,
        "tx: THRE set -> byte written to THR +0x00",
    );

    // 3) TX with no room: bounded spin gives up, drops the byte, never hangs.
    clear_scratch();
    poke(LSR, 0); // THRE clear forever — a dead/unclocked UART
    let sent = dev.write_byte(b'B');
    check(
        !sent && peek(THR) == 0,
        "tx: THRE clear -> bounded spin drops the byte (no hang, THR untouched)",
    );

    // 4) RX empty: DR clear means no byte, and RBR must not be read.
    clear_scratch();
    poke(LSR, LSR_THRE); // room to send, nothing received
    poke(RBR, 0x5A); // stale value the driver must NOT hand back
    check(dev.read_byte().is_none(), "rx: DR clear -> read_byte() = none");

    // 5) RX ready: DR set -> the byte comes from RBR at +0x00.
    poke(RBR, b'Z' as u32);
    poke(LSR, LSR_THRE | LSR_DR);
    check(
        dev.read_byte() == Some(b'Z'),
        "rx: DR set -> read_byte() = the byte in RBR +0x00",
    );

    // 6) The stride itself: register index i must sit at i<<2, so THR and LSR are
    //    0x14 apart. A reg-shift-0 driver would collide THR/IER/FCR into one word.
    check(
        REG_SHIFT == 2 && LSR - THR == 0x14 && LCR - THR == 0x0c && FCR - THR == 0x08,
        "layout: reg-shift 2 -> THR +0x00, FCR +0x08, LCR +0x0c, LSR +0x14",
    );

    clear_scratch();
    (pass, total)
}
