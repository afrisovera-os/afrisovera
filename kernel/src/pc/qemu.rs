//! "Power off when done" — only when the emulator asks for it.
//!
//! A test boot should end itself; a real PC must keep its screen up. The signal is a
//! file in QEMU's firmware-config device (`fw_cfg`, I/O ports 0x510/0x511 on x86 — the
//! port-I/O twin of the ARM kernel's `fwcfg.rs`): the test passes
//! `-fw_cfg name=opt/afrisovera/poweroff,string=1`. A real PC has no fw_cfg, the
//! signature read fails, and nothing happens.

use crate::cpu::{inb, outw};

const SELECTOR: u16 = 0x510;
const DATA: u16 = 0x511;
const KEY_SIGNATURE: u16 = 0x0000;
const KEY_FILE_DIR: u16 = 0x0019;

fn read(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: the fw_cfg data port, only read after the signature matched.
        *b = unsafe { inb(DATA) };
    }
}

fn present() -> bool {
    // SAFETY: selects the signature item; on a machine without fw_cfg the write goes
    // nowhere and the reads return 0xFF, which cannot spell "QEMU".
    unsafe { outw(SELECTOR, KEY_SIGNATURE) };
    let mut sig = [0u8; 4];
    read(&mut sig);
    &sig == b"QEMU"
}

/// True when this boot runs under QEMU AND the test asked for `opt/afrisovera/<name>`.
pub fn fw_cfg_file(name: &[u8]) -> bool {
    if !present() {
        return false;
    }
    // SAFETY: fw_cfg is present (signature matched); select the file directory.
    unsafe { outw(SELECTOR, KEY_FILE_DIR) };
    let mut n = [0u8; 4];
    read(&mut n);
    let count = u32::from_be_bytes(n);
    for _ in 0..count.min(256) {
        let mut e = [0u8; 64]; // u32 size, u16 select, u16 reserved, char name[56]
        read(&mut e);
        let nm = &e[8..];
        let len = nm.iter().position(|&c| c == 0).unwrap_or(nm.len());
        if &nm[..len] == name {
            return true;
        }
    }
    false
}

/// Read fw_cfg item `key` into `out` (the PC shell's `fwcfg` command). False, with `out`
/// untouched, when there is no fw_cfg device (not running under QEMU).
pub fn fw_cfg_read(key: u16, out: &mut [u8]) -> bool {
    if !present() {
        return false;
    }
    // SAFETY: fw_cfg is present (signature matched); select the item, then read it.
    unsafe { outw(SELECTOR, key) };
    read(out);
    true
}
