//! PL061 GPIO controller (QEMU `virt`) — a minimal, read-only bring-up.
//!
//! On QEMU `virt` an ARM PL061 lives at MMIO base `board::GPIO_BASE`. This driver
//! identifies the device by its ARM PrimeCell identification registers (proving a
//! real MMIO read to a distinct peripheral) and reads the pin-state register.
//! Direction/output/interrupt configuration is deliberately not done yet — nothing
//! is wired to the pins under emulation, so writing them would be theatre. This is
//! the honest, verifiable first step: the device is present and readable.

use core::ptr::{read_volatile, write_volatile};

const GPIO_BASE: usize = crate::board::GPIO_BASE;

// Data register: with the PL061 address-masking scheme, offset 0x3FC (all 8 bits
// of the mask set) reads/writes all pins at once.
const GPIO_DATA_ALL: usize = 0x3FC;
// Direction register: bit n = 1 -> pin n is an output.
const GPIO_DIR: usize = 0x400;
// ARM PrimeCell identification registers (one byte each, in the low bits).
const PERIPH_ID0: usize = 0xFE0; // 0x61 for PL061
const PERIPH_ID1: usize = 0xFE4; // 0x10 (part[11:8] in low nibble)

#[inline]
fn reg(off: usize) -> *const u32 {
    (GPIO_BASE + off) as *const u32
}

#[inline]
fn rd(off: usize) -> u32 {
    // SAFETY: fixed, valid Device-mapped MMIO on the virt board.
    unsafe { read_volatile(reg(off)) }
}

#[inline]
fn wr(off: usize, val: u32) {
    // SAFETY: fixed, valid Device-mapped MMIO on the virt board.
    unsafe { write_volatile((GPIO_BASE + off) as *mut u32, val) };
}

/// Configure pin `pin` (0..7) as an output by setting its GPIODIR bit.
pub fn set_output(pin: u8) {
    if pin >= 8 {
        return;
    }
    let dir = rd(GPIO_DIR) | (1u32 << pin);
    wr(GPIO_DIR, dir);
}

/// Drive pin `pin` (0..7) high or low. Uses the PL061 address-masking scheme:
/// the write only affects the pin whose mask bit is set, so other pins are
/// untouched. (The pin must be an output for the level to take effect.)
pub fn write_pin(pin: u8, high: bool) {
    if pin >= 8 {
        return;
    }
    let mask_off = (1usize << pin) << 2; // masked-data address for this pin
    wr(mask_off, if high { 0xFF } else { 0x00 });
}

/// The GPIODIR register (bit n set = pin n is an output).
pub fn direction() -> u8 {
    (rd(GPIO_DIR) & 0xFF) as u8
}

/// The 12-bit ARM PrimeCell part number. For a PL061 this is `0x061`.
pub fn part_number() -> u32 {
    let id0 = rd(PERIPH_ID0) & 0xFF;
    let id1 = rd(PERIPH_ID1) & 0xFF;
    ((id1 & 0x0F) << 8) | id0
}

/// True if a PL061 is present at the expected base (part number 0x061).
pub fn probe() -> bool {
    part_number() == 0x061
}

/// Current state of the 8 GPIO pins (read-only snapshot).
pub fn read_pins() -> u8 {
    (rd(GPIO_DATA_ALL) & 0xFF) as u8
}
