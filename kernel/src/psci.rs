//! ARM PSCI (Power State Coordination Interface) — clean reset/shutdown.
//!
//! Until now the kernel idled forever after the shell. PSCI is the standard
//! firmware interface for power operations on ARM. On QEMU `virt` (started at EL1
//! with no secure monitor) the conduit is `HVC`: the guest puts a PSCI function
//! id in w0 and executes `hvc #0`; QEMU services it. We implement the two
//! system-level calls — SYSTEM_RESET (reboot) and SYSTEM_OFF (poweroff).

use core::arch::asm;

const PSCI_SYSTEM_OFF: u64 = 0x8400_0008;
const PSCI_SYSTEM_RESET: u64 = 0x8400_0009;

#[inline(never)]
fn psci_call(function_id: u64) {
    // SAFETY: a well-formed PSCI SMC32 call over the HVC conduit. SYSTEM_OFF /
    // SYSTEM_RESET take no arguments and do not return on success.
    unsafe {
        asm!("hvc #0", in("x0") function_id, options(nomem, nostack));
    }
}

/// Reset the machine via PSCI SYSTEM_RESET. Does not return on success; if the
/// call somehow returns, park the CPU rather than run off into undefined state.
pub fn reboot() -> ! {
    psci_call(PSCI_SYSTEM_RESET);
    loop {
        crate::cpu::wfi();
    }
}

/// Power the machine off via PSCI SYSTEM_OFF. On QEMU this makes the emulator
/// exit. Does not return on success.
pub fn poweroff() -> ! {
    psci_call(PSCI_SYSTEM_OFF);
    loop {
        crate::cpu::wfi();
    }
}
