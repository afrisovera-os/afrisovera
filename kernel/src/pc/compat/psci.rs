//! PC stand-in for the ARM `psci.rs` (power control through firmware). On a PC: the firmware's ResetSystem
//! (kept from boot) first, then ACPI (S5 through PM1 control, the FADT reset register), then the chipset
//! (0xCF9, the keyboard controller's reset line). `poweroff`, `shutdown`, `reboot`, `restart`, the power button
//! and the desk's power menu all come here (14 Nov 2026, the owner's HP).
#![allow(dead_code)]

/// Under the test rig only (fw_cfg opt/afrisovera/power-acpi): skip the firmware, so ACPI is what acts.
fn acpi_first() -> bool {
    crate::qemu::fw_cfg_file(b"opt/afrisovera/power-acpi")
}

pub fn poweroff() -> ! {
    if !acpi_first() {
        crate::kprintln!("[pc] switching off (the firmware's ResetSystem)");
        crate::uefi::try_reset(crate::runtime_services(), true);
        crate::kprintln!("[pc] the firmware did not switch off; trying ACPI");
    }
    crate::kprintln!("[pc] switching off (ACPI S5)");
    let why = crate::acpi::sleep_s5();
    crate::kprintln!("[pc] ACPI could not switch off ({}): everything is stopped, it is safe to hold the power button", why);
    crate::cpu::halt_forever();
}

pub fn reboot() -> ! {
    if !acpi_first() {
        crate::kprintln!("[pc] restarting (the firmware's ResetSystem)");
        crate::uefi::try_reset(crate::runtime_services(), false);
        crate::kprintln!("[pc] the firmware did not restart; trying ACPI");
    }
    crate::kprintln!("[pc] restarting (the ACPI reset register)");
    let why = crate::acpi::reset_register();
    crate::kprintln!("[pc] {}; trying the chipset", why);
    crate::uefi::reset_cold(core::ptr::null());
}
