//! PC stand-in for the ARM `board.rs` (the per-board peripheral map). A PC's console, clock
//! and firmware-config device are fixed I/O PORTS, not MMIO windows, and it has no GIC,
//! PL061 GPIO or virtio-MMIO bus: those bases are 0 ("none"). The constants keep the ARM
//! names so `shell.rs` compiles unchanged; the commands that would DEREFERENCE them on
//! ARM (date, rtcinfo, uartinfo, gicinfo, fwcfg ...) are answered on the PC by
//! `shell_pc::intercept` with the PC's real devices instead.
#![allow(dead_code)]

use core::sync::atomic::{AtomicU32, Ordering};

pub struct Map {
    pub name: &'static str,
    pub uart: usize,
    pub uart_kind: &'static str,
    pub gic_dist: usize,
    pub gic_cpu: usize,
    pub rtc: usize,
    pub gpio: usize,
    pub fwcfg: usize,
    pub virtio: u64,
    pub note: &'static str,
}

pub const PC: Map = Map {
    name: "pc-x86_64 (UEFI)",
    uart: 0x3F8,      // COM1, I/O port
    uart_kind: "16550",
    gic_dist: 0,      // no GIC: local APIC + I/O APIC
    gic_cpu: 0,
    rtc: 0x70,        // CMOS RTC, I/O ports 0x70/0x71
    gpio: 0,          // none driven
    fwcfg: 0x510,     // QEMU fw_cfg, I/O ports 0x510/0x511 (QEMU only)
    virtio: 0,        // virtio is PCI on a PC (later step)
    note: "x86-64 PC booted by UEFI: COM1 console, CMOS RTC, local APIC timer, GOP framebuffer",
};

pub const ACTIVE: Map = PC;
pub const NAME: &str = ACTIVE.name;
pub const UART_BASE: usize = ACTIVE.uart;
pub const GIC_DIST_BASE: usize = ACTIVE.gic_dist;
pub const GIC_CPU_BASE: usize = ACTIVE.gic_cpu;
pub const RTC_BASE: usize = ACTIVE.rtc;
pub const GPIO_BASE: usize = ACTIVE.gpio;
pub const FWCFG_BASE: usize = ACTIVE.fwcfg;
pub const VIRTIO_MMIO_BASE: u64 = ACTIVE.virtio;

/// The resolved map. On ARM `rtc` is the PL031's MMIO address, which the shared `vfs.rs`
/// reads (one 32-bit load) for file times. On the PC it is the address of a word refreshed
/// from the CMOS clock on every call, so that same load returns real epoch seconds.
#[derive(Clone, Copy)]
pub struct Resolved {
    pub uart: usize,
    pub gic_dist: usize,
    pub rtc: usize,
}

static RTC_SHADOW: AtomicU32 = AtomicU32::new(0);

pub fn resolved() -> Resolved {
    RTC_SHADOW.store(crate::cmos::epoch() as u32, Ordering::Relaxed);
    Resolved { uart: UART_BASE, gic_dist: 0, rtc: RTC_SHADOW.as_ptr() as usize }
}

fn row(label: &str, v: usize, what: &str) {
    if v == 0 {
        crate::kprintln!("  {:<16} none         {}", label, what);
    } else {
        crate::kprintln!("  {:<16} {:#010x}   {}", label, v, what);
    }
}

pub fn describe() {
    crate::kprintln!("board: {}", NAME);
    crate::kprintln!("  {}", ACTIVE.note);
    row("uart", UART_BASE, "16550 COM1 (I/O port)");
    row("rtc", RTC_BASE, "CMOS RTC (I/O port)");
    row("fw_cfg", FWCFG_BASE, "QEMU fw_cfg (I/O port, QEMU only)");
    row("gic", 0, "a PC has a local APIC instead");
    row("gpio", 0, "");
    row("virtio-mmio", 0, "PC virtio is PCI (later step)");
}

pub fn describe_named(name: &str) {
    crate::kprintln!("board {}: the ARM board table is not built into the PC kernel", name);
    describe();
}

pub fn describe_machine() {
    crate::kprintln!("machine: x86-64 PC (UEFI firmware, ACPI; no device tree)");
}

pub fn describe_resolved() {
    crate::kprintln!("board resolved: compile-time PC map (a PC has no device tree to override it)");
    describe();
}

pub fn list() {
    crate::kprintln!("boards compiled in: {} (the ARM board table lives in the ARM kernel)", NAME);
}

pub fn check() {
    crate::kprintln!("board check: PC map -- COM1 0x3f8, CMOS RTC 0x70, fw_cfg 0x510 (I/O ports): ok");
}

pub fn machine_selftest() {
    crate::kprintln!("machine selftest: the ARM machine table is not built into the PC kernel");
}

pub fn resolve_selftest() {
    crate::kprintln!("resolve selftest: no device tree on a PC -- nothing to resolve");
}
