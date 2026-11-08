//! PC stand-in for the ARM `gic.rs`: a PC has no GIC. Its interrupt controller is the
//! local APIC (plus the I/O APIC), driven by `pc/cpu.rs`. Base 0 = "no GIC here".
#![allow(dead_code)]

pub fn dist_base() -> usize { 0 }
pub fn cpu_base() -> usize { 0 }

pub fn describe_bases() {
    crate::kprintln!("gic: none on this PC -- interrupts go through the local APIC (see 'gicinfo')");
}
