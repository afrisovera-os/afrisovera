//! PC stand-in for the ARM `psci.rs` (power control through firmware). On a PC the
//! firmware's power control is UEFI's ResetSystem runtime service, kept from boot.
#![allow(dead_code)]

pub fn poweroff() -> ! {
    crate::kprintln!("[pc] powering off (UEFI ResetSystem: shutdown)");
    crate::uefi::shutdown(crate::runtime_services());
}

pub fn reboot() -> ! {
    crate::kprintln!("[pc] rebooting (UEFI ResetSystem: cold reset)");
    crate::uefi::reset_cold(crate::runtime_services());
}
