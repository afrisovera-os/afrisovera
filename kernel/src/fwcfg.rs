//! QEMU `fw_cfg` device driver (MMIO + DMA).
//!
//! `fw_cfg` is how the guest reads firmware/config blobs from QEMU and — via the
//! DMA interface — writes a few configurable items back. We need it for exactly
//! one thing in Phase 1.0: locating and configuring the `ramfb` framebuffer.
//!
//! On the QEMU `virt` board the register block is at MMIO 0x0902_0000:
//!   +0x08  selector register (16-bit)
//!   +0x10  DMA address register (64-bit, **big-endian**) — writing it triggers a
//!          synchronous DMA transfer described by a guest-memory descriptor.
//!
//! All multi-byte `fw_cfg` values are big-endian.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{compiler_fence, Ordering};

const FWCFG_BASE: usize = crate::board::FWCFG_BASE;
const REG_DMA: usize = FWCFG_BASE + 0x10;

/// Selector for the file directory listing.
const FW_CFG_FILE_DIR: u32 = 0x0019;

// DMA control-register bit flags.
const CTL_ERROR: u32 = 1 << 0;
const CTL_READ: u32 = 1 << 1;
const CTL_SELECT: u32 = 1 << 3;
const CTL_WRITE: u32 = 1 << 4;

/// Perform one synchronous `fw_cfg` DMA transfer. `buf_pa` is the guest-physical
/// address of the source/destination buffer (identity-mapped, so == VA).
unsafe fn dma(control: u32, buf_pa: u64, len: u32) -> Result<(), ()> {
    // Descriptor: control (BE u32), length (BE u32), address (BE u64).
    let mut desc = [0u8; 16];
    desc[0..4].copy_from_slice(&control.to_be_bytes());
    desc[4..8].copy_from_slice(&len.to_be_bytes());
    desc[8..16].copy_from_slice(&buf_pa.to_be_bytes());

    let desc_pa = desc.as_ptr() as u64;
    compiler_fence(Ordering::SeqCst);
    // The DMA address register is big-endian; store the byte-swapped descriptor
    // address so QEMU reads it correctly. The write triggers synchronous DMA.
    write_volatile(REG_DMA as *mut u64, desc_pa.to_be());
    compiler_fence(Ordering::SeqCst);

    // QEMU has written the result back into desc[0..4] (control, BE). Read it
    // volatile so the compiler doesn't assume the buffer is unchanged.
    let c0 = read_volatile(desc.as_ptr());
    let c1 = read_volatile(desc.as_ptr().add(1));
    let c2 = read_volatile(desc.as_ptr().add(2));
    let c3 = read_volatile(desc.as_ptr().add(3));
    let done = u32::from_be_bytes([c0, c1, c2, c3]);
    if done & CTL_ERROR != 0 {
        Err(())
    } else {
        Ok(())
    }
}

/// Select an item and read `buf.len()` bytes from its start.
unsafe fn select_read(selector: u32, buf: &mut [u8]) -> Result<(), ()> {
    dma((selector << 16) | CTL_SELECT | CTL_READ, buf.as_ptr() as u64, buf.len() as u32)
}

/// Continue reading the currently-selected item (advances its offset).
unsafe fn read_more(buf: &mut [u8]) -> Result<(), ()> {
    dma(CTL_READ, buf.as_ptr() as u64, buf.len() as u32)
}

/// Select an item and write `buf` into it (used for `etc/ramfb`).
pub fn write_item(selector: u16, buf: &[u8]) -> Result<(), ()> {
    // SAFETY: buffer is a valid, identity-mapped guest-memory region.
    unsafe {
        dma(((selector as u32) << 16) | CTL_SELECT | CTL_WRITE, buf.as_ptr() as u64, buf.len() as u32)
    }
}

#[inline]
unsafe fn rd(p: *const u8, i: usize) -> u8 {
    read_volatile(p.add(i))
}

/// Find a named `fw_cfg` file and return its selector, if present.
pub fn find_file(name: &str) -> Option<u16> {
    // SAFETY: single-threaded bring-up; MMIO addresses are valid on virt.
    unsafe {
        let mut cnt = [0u8; 4];
        select_read(FW_CFG_FILE_DIR, &mut cnt).ok()?;
        let count = u32::from_be_bytes([rd(cnt.as_ptr(), 0), rd(cnt.as_ptr(), 1), rd(cnt.as_ptr(), 2), rd(cnt.as_ptr(), 3)]);

        // Each directory entry: size(BE u32), select(BE u16), reserved(u16), name[56].
        for _ in 0..count {
            let mut e = [0u8; 64];
            read_more(&mut e).ok()?;
            let sel = u16::from_be_bytes([rd(e.as_ptr(), 4), rd(e.as_ptr(), 5)]);
            // Compare the NUL-terminated name.
            let mut len = 0;
            while len < 56 && rd(e.as_ptr(), 8 + len) != 0 {
                len += 1;
            }
            let nb = name.as_bytes();
            if len == nb.len() {
                let mut matches = true;
                for i in 0..len {
                    if rd(e.as_ptr(), 8 + i) != nb[i] {
                        matches = false;
                        break;
                    }
                }
                if matches {
                    return Some(sel);
                }
            }
        }
        None
    }
}
