//! PC stand-in for the ARM `fwcfg.rs` (fw_cfg over MMIO). The shared `framebuffer.rs`
//! looks for QEMU's `ramfb` here; the PC's screen is the UEFI GOP framebuffer instead,
//! so there is no ramfb to configure. (The PC's own port-I/O fw_cfg reader is `qemu.rs`.)
#![allow(dead_code)]

pub fn find_file(_name: &str) -> Option<u16> { None }
pub fn write_item(_selector: u16, _buf: &[u8]) -> Result<(), ()> { Err(()) }
