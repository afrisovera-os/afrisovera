//! PC stand-in for the ARM `fdt.rs`. A PC describes itself through ACPI and the UEFI
//! memory map, not a device tree: `info()` reports "no tree" (valid = false).
#![allow(dead_code)]

#[derive(Clone, Copy)]
pub struct Info {
    pub valid: bool,
    pub ram_base: u64,
    pub ram_size: u64,
    pub uart_base: u64,
    pub gic_base: u64,
    pub gic_cpu_base: u64,
    pub rtc_base: u64,
    pub uart_kind: &'static str,
    pub reference: bool,
}

pub fn info() -> Info {
    Info {
        valid: false,
        ram_base: 0,
        ram_size: 0,
        uart_base: 0,
        gic_base: 0,
        gic_cpu_base: 0,
        rtc_base: 0,
        uart_kind: "",
        reference: false,
    }
}

pub fn model() -> &'static str {
    "pc (no device tree: ACPI + UEFI memory map)"
}
