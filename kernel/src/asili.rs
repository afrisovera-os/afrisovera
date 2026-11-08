//! Asili — device registry / Device Manager (v0).
//!
//! The architecture ([`docs/02`]) puts drivers in **userspace** behind a stable
//! ABI, supervised by a Device Manager that enumerates hardware and binds
//! drivers. That is a Phase 1+ build-out. This module is the v0 seed: a kernel
//! registry that records the devices the bring-up code has brought online, so
//! the system (and the `devices` shell command) can enumerate what hardware is
//! present and healthy — the "Device Manager enumerates hardware" milestone.

#[derive(Clone, Copy)]
#[allow(dead_code)] // Input/Storage/Net are declared ahead of their drivers
pub enum Class {
    Firmware,
    Console,
    Interrupt,
    Timer,
    Display,
    Input,
    Storage,
    Net,
    Audio,
    Entropy,
    Gpio,
}

impl Class {
    pub fn as_str(&self) -> &'static str {
        match self {
            Class::Firmware => "firmware",
            Class::Console => "console",
            Class::Interrupt => "interrupt",
            Class::Timer => "timer",
            Class::Display => "display",
            Class::Input => "input",
            Class::Storage => "storage",
            Class::Net => "net",
            Class::Audio => "audio",
            Class::Entropy => "entropy",
            Class::Gpio => "gpio",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Device {
    pub name: &'static str,
    pub class: Class,
    pub mmio: u64,
    pub ok: bool,
}

const MAX_DEVICES: usize = 16;
static mut DEVICES: [Option<Device>; MAX_DEVICES] = [None; MAX_DEVICES];
static mut COUNT: usize = 0;

/// Record a device that bring-up has brought online.
pub fn register(name: &'static str, class: Class, mmio: u64, ok: bool) {
    // SAFETY: single-threaded bring-up; registration happens before the shell.
    unsafe {
        if COUNT < MAX_DEVICES {
            DEVICES[COUNT] = Some(Device { name, class, mmio, ok });
            COUNT += 1;
        }
    }
}

/// Number of registered devices.
pub fn count() -> usize {
    // SAFETY: scalar read.
    unsafe { COUNT }
}

/// Fetch device `i`, if present.
pub fn get(i: usize) -> Option<Device> {
    // SAFETY: bounds-checked read of Copy data.
    unsafe {
        if i < COUNT {
            DEVICES[i]
        } else {
            None
        }
    }
}
