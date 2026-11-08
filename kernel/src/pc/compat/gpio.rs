//! PC stand-in for the ARM `gpio.rs` (a PL061 on QEMU virt). A PC exposes no GPIO bank
//! the kernel drives yet: `probe()` says so and every pin reads 0, writes go nowhere.
#![allow(dead_code)]

pub fn probe() -> bool { false }
pub fn part_number() -> u32 { 0 }
pub fn direction() -> u8 { 0 }
pub fn read_pins() -> u8 { 0 }
pub fn set_output(_pin: u8) {}
pub fn write_pin(_pin: u8, _high: bool) {}
