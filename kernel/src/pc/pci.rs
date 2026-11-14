//! PCI configuration space for every PC driver: architecture review 1, R1-8 (8 Nov 2026).
//!
//! Nine drivers each carried a private copy of configuration mechanism #1 (ports 0xCF8/0xCFC)
//! and their own bus scan. This is the one copy (AFR-0002 §6 step 1, `pci.Config`); drivers move
//! onto it one per commit with no behaviour change: nvme, ahci, e1000, rtl8169, hda, xhci, virtio, wifi.

use core::arch::asm;

unsafe fn outl(port: u16, v: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags));
}
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}

/// The address written to 0xCF8 for one dword of bus/device/function `b:d.f`.
pub fn cfg_addr(b: u8, d: u8, f: u8, off: u8) -> u32 {
    0x8000_0000 | (b as u32) << 16 | (d as u32) << 11 | (f as u32) << 8 | (off as u32 & 0xFC)
}

/// Read one configuration dword (`off` is rounded down to 4).
pub fn cfg_read32(b: u8, d: u8, f: u8, off: u8) -> u32 {
    // SAFETY: the PCI configuration ports; boot context, single core.
    unsafe {
        outl(0xCF8, cfg_addr(b, d, f, off));
        inl(0xCFC)
    }
}

/// Write one configuration dword.
pub fn cfg_write32(b: u8, d: u8, f: u8, off: u8, v: u32) {
    // SAFETY: as cfg_read32.
    unsafe {
        outl(0xCF8, cfg_addr(b, d, f, off));
        outl(0xCFC, v);
    }
}

/// The first function on buses `0..buses` for which `hit(b, d, f, id)` is true (`id` is
/// configuration dword 0: vendor in the low half, device in the high half). Absent functions
/// are skipped, and an absent function 0 skips the rest of that device.
pub fn find_by(buses: u8, mut hit: impl FnMut(u8, u8, u8, u32) -> bool) -> Option<(u8, u8, u8)> {
    for b in 0..buses {
        for d in 0..32u8 {
            for f in 0..8u8 {
                let id = cfg_read32(b, d, f, 0);
                if id & 0xFFFF == 0xFFFF {
                    if f == 0 {
                        break;
                    }
                    continue;
                }
                if hit(b, d, f, id) {
                    return Some((b, d, f));
                }
            }
        }
    }
    None
}

#[allow(dead_code)] // the drivers now decide from their BINDS (pc/hwdb.rs); kept for tools and tests
/// The first function on buses `0..buses` whose class code (base:sub:interface, 24 bits) is
/// `class`. Same walk the drivers used.
pub fn find_class(class: u32, buses: u8) -> Option<(u8, u8, u8)> {
    find_by(buses, |b, d, f, _| cfg_read32(b, d, f, 8) >> 8 == class)
}

/// BAR `n` (0-5) as a memory address, reading the next BAR as the high half when it is 64-bit.
pub fn bar_mem(b: u8, d: u8, f: u8, n: u8) -> u64 {
    let off = 0x10 + 4 * n;
    let lo = cfg_read32(b, d, f, off);
    (lo & !0xF) as u64 | if lo & 0x6 == 0x4 { (cfg_read32(b, d, f, off + 4) as u64) << 32 } else { 0 }
}

/// OR `bits` into the command register (bit 1 memory decoding, bit 2 bus mastering,
/// bit 10 INTx off).
pub fn enable(b: u8, d: u8, f: u8, bits: u32) {
    cfg_write32(b, d, f, 4, cfg_read32(b, d, f, 4) | bits);
}
