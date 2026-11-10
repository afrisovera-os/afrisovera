//! IDE-mode disks on the PC: roadmap PC step 3 (10 Nov 2026).
//!
//! Many PCs from about 2006 to 2014 have their SATA ports set to "IDE" (also called
//! "compatibility", "legacy" or "ATA") in the firmware set-up instead of AHCI, and a Windows
//! installed that way stops starting if the setting is changed. The controller is then PCI
//! class 01:01 (an IDE controller), not 01:06 (AHCI), so ahci.rs never sees the disk. This
//! driver speaks the ATA task-file registers of such a controller: each channel at the old
//! fixed ports (0x1F0/0x3F6 and 0x170/0x376, "compatibility mode") or at the I/O BARs the
//! firmware gave it ("native mode", programming interface bits 0 and 2), two drives per
//! channel, polled with interrupts off (nIEN), one 512-byte sector per PIO command:
//! READ/WRITE SECTORS (LBA28) or their EXT forms (LBA48) above 128 GiB, and FLUSH CACHE after
//! the VFS's last sector so a power-off right after `sync` keeps the files.
//!
//! The same safety rule as every disk here: a disk is used for storage only if sector 0 is
//! already AFRIVFS1 (ours), or blank and the owner types `claimdisk ide`. A disk with a boot
//! sector, a partition table or a file system (Windows, on such a PC) is listed and never
//! written. CD/DVD drives (ATAPI) are named and not used. Proved on QEMU's piix3-ide.
//!
//! Partitions (10 Nov 2026): a Windows PC in IDE mode can only take AfriSovera inside its own
//! partition, so each drive is also a disk for gpt.rs by its number in `lside` (ide0, ide1, ...):
//! lsparts reads it, installpart/tablerestore write its table (raw_read/table_write), formatpart
//! keeps the VFS in AfriSovera's partition (use_part), and at boot a formatted AfriSovera partition
//! in a healthy GPT is used again, as ahci.rs does.
//!
//! Multi-sector PIO (10 Nov 2026): on the storage drive a read fetches up to 16 sectors (8 KiB)
//! with one READ SECTORS command (sector count k, one DRQ block per sector) into a read-ahead, and
//! the VFS's in-order save is gathered into runs of up to 16 sectors sent with one WRITE SECTORS.
//! Each way is checked once against one sector at a time first (reads when the drive is taken for
//! storage, writes at the first save, read back); a mismatch or a refusal turns that way off with
//! one line. Other drives (raw_read, table_write) stay one sector per command.
//! ATA/ATAPI-8 ACS (task file 7.x, IDENTIFY DEVICE 7.16), PCI IDE Controller Specification 1.0.

use crate::pci;
use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut};

const ST_ERR: u8 = 1 << 0;
const ST_DRQ: u8 = 1 << 3;
const ST_DF: u8 = 1 << 5;
const ST_BSY: u8 = 1 << 7;
const CTL_NIEN: u8 = 1 << 1;
const CTL_SRST: u8 = 1 << 2;

const ATA_READ: u8 = 0x20;
const ATA_READ_EXT: u8 = 0x24;
const ATA_WRITE: u8 = 0x30;
const ATA_WRITE_EXT: u8 = 0x34;
const ATA_FLUSH: u8 = 0xE7;
const ATA_FLUSH_EXT: u8 = 0xEA;
const ATA_IDENTIFY: u8 = 0xEC;

unsafe fn inb(port: u16) -> u8 {
    let v: u8;
    asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
unsafe fn outb(port: u16, v: u8) {
    asm!("out dx, al", in("dx") port, in("al") v, options(nomem, nostack, preserves_flags));
}
unsafe fn inw(port: u16) -> u16 {
    let v: u16;
    asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
unsafe fn outw(port: u16, v: u16) {
    asm!("out dx, ax", in("dx") port, in("ax") v, options(nomem, nostack, preserves_flags));
}

#[derive(Clone, Copy)]
struct Drive {
    cmd: u16,     // task-file base (data register)
    ctl: u16,     // device control / alternate status
    chan: u8,     // 0 primary, 1 secondary (of its controller)
    slave: bool,  // device 1
    atapi: bool,  // a CD/DVD drive: named, never used
    sectors: u64, // 512-byte sectors
    lba48: bool,
    flush: bool, // FLUSH CACHE offered (IDENTIFY word 83 bit 12 / word 82 bit 12)
    usable: bool,
    odd: bool, // sectors other than 512 bytes: not read or written at all
    model: [u8; 40],
}

const NONE: Drive = Drive { cmd: 0, ctl: 0, chan: 0, slave: false, atapi: false, sectors: 0, lba48: false, flush: false, usable: false, odd: false, model: [b' '; 40] };

static mut CTRLS: [(u8, u8, u8, u8); 2] = [(0, 0, 0, 0); 2]; // bus, device, function, programming interface
static mut N_CTRL: usize = 0;
static mut DRIVES: [Drive; 8] = [NONE; 8];
static mut N_DRIVES: usize = 0;
static mut DISK: Option<usize> = None; // the drive holding the VFS
static mut BLANK: Option<usize> = None; // the first blank ATA drive (claimdisk ide)
static mut STATS: (u64, u64, u64) = (0, 0, 0); // sectors read, written, cache flushes
/// AfriSovera's partition on DISK: (first sector, sectors); sectors 0 = the whole disk.
static mut PART: (u64, u64) = (0, 0);
static mut SECTOR: [u8; 512] = [0; 512];
/// Multi-sector PIO on DISK: the read-ahead (RA_N sectors from drive sector RA_AT) and the gathered
/// writes (WB_N sectors from WB_AT), up to KMAX sectors each.
const KMAX: u32 = 16;
const RA_BYTES: usize = 512 * KMAX as usize;
static mut RA: [u8; RA_BYTES] = [0; RA_BYTES];
static mut RA_AT: u64 = 0;
static mut RA_N: u32 = 0;
static mut WB: [u8; RA_BYTES] = [0; RA_BYTES];
static mut WB_AT: u64 = 0;
static mut WB_N: u32 = 0;
/// 0 = not checked, 1 = on, 2 = off; reads and writes apart.
static mut MULTI: u8 = 0;
static mut MULTIW: u8 = 0;
/// Reads: sectors asked for, served from the read-ahead, multi-sector commands, single ones.
static mut RSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Writes: sectors written, multi-sector commands, single ones.
static mut WSTATS: (u64, u64, u64) = (0, 0, 0);
/// Big copies (table_write_run): 0 = not tried, 1 = on, 2 = off (one sector at a time, as table_write).
static mut TRUN: u8 = 0;
/// Big copies: runs written in one command, sectors in them.
static mut TSTATS: (u64, u64) = (0, 0);
/// A run read back after it is written.
static mut TRB: [u8; RA_BYTES] = [0; RA_BYTES];

/// Wait up to `ms` milliseconds for `ok`.
fn wait(ms: u64, mut ok: impl FnMut() -> bool) -> bool {
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    loop {
        if ok() {
            return true;
        }
        if crate::timer::cycles() - t0 > hz * ms / 1000 {
            return false;
        }
        core::hint::spin_loop();
    }
}

/// The 400 ns a device needs after a select or a command before its status means anything.
unsafe fn settle(ctl: u16) {
    for _ in 0..5 {
        inb(ctl);
    }
}

/// Select device `slave` on the channel (LBA mode, `hi` = bits 27:24 of an LBA28 address) and wait
/// until it is ready for a command.
unsafe fn select(d: &Drive, hi: u8) -> bool {
    outb(d.ctl, CTL_NIEN);
    outb(d.cmd + 6, 0xE0 | (d.slave as u8) << 4 | (hi & 0xF));
    settle(d.ctl);
    wait(2000, || inb(d.ctl) & (ST_BSY | ST_DRQ) == 0)
}

/// After a command: wait for BSY to clear, then true when the device asks for data (DRQ) and
/// reports no error.
unsafe fn data_ready(d: &Drive, ms: u64) -> bool {
    settle(d.ctl);
    if !wait(ms, || inb(d.ctl) & ST_BSY == 0) {
        return false;
    }
    let s = inb(d.cmd + 7);
    s & (ST_ERR | ST_DF) == 0 && s & ST_DRQ != 0
}

/// Send the address registers and the command for `k` sectors (1 to KMAX) from `lba`.
unsafe fn issue(d: &Drive, lba: u64, k: u32, write: bool) -> bool {
    // LBA28 reaches sectors below 2^28 - 1: a run ending there or above takes the EXT form
    let ext = d.lba48 && lba + k as u64 > (1 << 28) - 1;
    if !select(d, if ext { 0 } else { (lba >> 24) as u8 }) {
        return false;
    }
    if ext {
        // high bytes first (the "previous" contents of each register), then the low ones
        outb(d.cmd + 2, 0);
        outb(d.cmd + 3, (lba >> 24) as u8);
        outb(d.cmd + 4, (lba >> 32) as u8);
        outb(d.cmd + 5, (lba >> 40) as u8);
    }
    outb(d.cmd + 2, k as u8);
    outb(d.cmd + 3, lba as u8);
    outb(d.cmd + 4, (lba >> 8) as u8);
    outb(d.cmd + 5, (lba >> 16) as u8);
    outb(d.cmd + 7, match (ext, write) {
        (false, false) => ATA_READ,
        (true, false) => ATA_READ_EXT,
        (false, true) => ATA_WRITE,
        (true, true) => ATA_WRITE_EXT,
    });
    true
}

/// One PIO command of `k` sectors (1 to KMAX) from `lba` of drive `i`, to or from `buf` (k * 512
/// bytes): the drive asks for each sector's 256 words in turn (DRQ). No line printed on failure.
unsafe fn xfer(i: usize, lba: u64, k: u32, buf: *mut u8, write: bool) -> bool {
    let d = DRIVES[i];
    if k == 0 || k > KMAX || lba.saturating_add(k as u64) > d.sectors || !issue(&d, lba, k, write) {
        return false;
    }
    for j in 0..k as usize {
        if !data_ready(&d, 5000) {
            return false;
        }
        let s = buf.add(j * 512);
        for w in 0..256 {
            if write {
                outw(d.cmd, u16::from_le_bytes([*s.add(2 * w), *s.add(2 * w + 1)]));
            } else {
                let v = inw(d.cmd);
                *s.add(2 * w) = v as u8;
                *s.add(2 * w + 1) = (v >> 8) as u8;
            }
        }
    }
    settle(d.ctl);
    if write {
        if !wait(5000, || inb(d.ctl) & ST_BSY == 0) || inb(d.cmd + 7) & (ST_ERR | ST_DF) != 0 {
            return false;
        }
        STATS.1 += k as u64;
    } else {
        STATS.0 += k as u64;
    }
    true
}

/// Read sector `lba` of drive `i` into SECTOR.
unsafe fn read_raw(i: usize, lba: u64) -> bool {
    if !xfer(i, lba, 1, addr_of_mut!(SECTOR) as *mut u8, false) {
        fail(i, "read", lba);
        return false;
    }
    true
}

/// Write SECTOR to sector `lba` of drive `i`.
unsafe fn write_raw(i: usize, lba: u64) -> bool {
    if !xfer(i, lba, 1, addr_of_mut!(SECTOR) as *mut u8, true) {
        fail(i, "write", lba);
        return false;
    }
    true
}

/// When a drive is taken for storage: one READ SECTORS of the first k sectors of the storage area
/// must equal the same sectors read one at a time, else multi-sector reads stay off (and writes
/// with them). Reads only: safe on any disk. One line says which.
fn multi_check() {
    // SAFETY: single core, kernel/shell context; no command in flight.
    unsafe {
        (MULTI, MULTIW, RA_N, WB_N) = (0, 0, 0, 0);
        let Some(i) = DISK else { return };
        let first = PART.0;
        let k = (KMAX as u64).min(capacity()) as u32;
        if k < 2 {
            (MULTI, MULTIW) = (2, 2);
            return;
        }
        let ra = addr_of_mut!(RA) as *mut u8;
        core::ptr::write_bytes(ra, 0xA5, RA_BYTES);
        let read_ok = xfer(i, first, k, ra, false);
        let mut same = read_ok;
        for j in 0..k as usize {
            if !same {
                break;
            }
            same = read_raw(i, first + j as u64) && (&*addr_of!(SECTOR))[..] == core::slice::from_raw_parts(ra.add(j * 512), 512)[..];
        }
        if same {
            (MULTI, RA_AT, RA_N) = (1, first, k);
            crate::kprintln!("[ide] ide{}: multi-sector reads on (up to {} sectors per command; the first {} equal one at a time)", i, k, k);
        } else {
            (MULTI, MULTIW, RA_N) = (2, 2, 0);
            crate::kprintln!(
                "[ide] ide{}: multi-sector reads off ({}): one sector per command",
                i,
                if read_ok { "they differed from one at a time" } else { "the drive refused one" }
            );
        }
    }
}

/// The drive sector after the storage area (its partition, or the whole disk).
fn end_sector() -> u64 {
    // SAFETY: scalar read.
    unsafe { PART.0 + capacity() }
}

/// Drive sector `n` of DISK into SECTOR: from the read-ahead when it holds it, else a new
/// multi-sector read of up to KMAX sectors from it (once the check passed), else one sector.
unsafe fn fetch(i: usize, n: u64) -> bool {
    RSTATS.0 += 1;
    if n >= RA_AT && n < RA_AT + RA_N as u64 {
        let off = (n - RA_AT) as usize * 512;
        (*addr_of_mut!(SECTOR)).copy_from_slice(&(&*addr_of!(RA))[off..off + 512]);
        RSTATS.1 += 1;
        return true;
    }
    if MULTI == 1 {
        let k = (KMAX as u64).min(end_sector().saturating_sub(n)) as u32;
        if k > 1 {
            RA_N = 0;
            if xfer(i, n, k, addr_of_mut!(RA) as *mut u8, false) {
                (RA_AT, RA_N) = (n, k);
                (*addr_of_mut!(SECTOR)).copy_from_slice(&(&*addr_of!(RA))[..512]);
                RSTATS.2 += 1;
                return true;
            }
            MULTI = 2;
            crate::kprintln!("[ide] a multi-sector read at sector {} was refused: one sector per command from now on", n);
        }
    }
    RSTATS.3 += 1;
    read_raw(i, n)
}

/// Drive sector `n` of DISK was written from `src` (or the write failed): keep the read-ahead true.
unsafe fn ra_update(n: u64, src: &[u8], ok: bool) {
    if n >= RA_AT && n < RA_AT + RA_N as u64 {
        if ok {
            let off = (n - RA_AT) as usize * 512;
            (&mut *addr_of_mut!(RA))[off..off + 512].copy_from_slice(src);
        } else {
            RA_N = 0; // the drive's copy is unknown now
        }
    }
}

/// Send the gathered writes of DISK: one WRITE SECTORS when multi-sector writes are on (the first
/// one is read back sector by sector and must be equal, else they go off and the run is rewritten
/// one sector at a time), else one per sector.
unsafe fn send(i: usize) -> bool {
    let (at, k) = (WB_AT, WB_N);
    if k == 0 {
        return true;
    }
    WB_N = 0;
    let wb = addr_of_mut!(WB) as *mut u8;
    let mut done = false;
    if k > 1 && MULTI == 1 && MULTIW != 2 {
        if xfer(i, at, k, wb, true) {
            WSTATS.1 += 1;
            done = true;
            if MULTIW == 0 {
                for j in 0..k as usize {
                    if !(read_raw(i, at + j as u64) && (&*addr_of!(SECTOR))[..] == core::slice::from_raw_parts(wb.add(j * 512), 512)[..]) {
                        done = false;
                        break;
                    }
                }
                MULTIW = if done { 1 } else { 2 };
                if done {
                    crate::kprintln!("[ide] ide{}: multi-sector writes on (up to {} sectors per command; the first {} read back equal)", i, KMAX, k);
                } else {
                    crate::kprintln!("[ide] ide{}: multi-sector writes off (read back differently: rewritten one sector per command)", i);
                }
            }
        } else {
            MULTIW = 2;
            crate::kprintln!("[ide] a multi-sector write at sector {} was refused: one sector per command from now on", at);
        }
    }
    let mut ok = true;
    for j in 0..k as usize {
        let s = core::slice::from_raw_parts(wb.add(j * 512), 512);
        let good = done || {
            (*addr_of_mut!(SECTOR)).copy_from_slice(s);
            WSTATS.2 += 1;
            write_raw(i, at + j as u64)
        };
        ra_update(at + j as u64, s, good);
        ok = ok && good;
    }
    ok
}

/// FLUSH CACHE: the drive's write cache onto the platters or flash.
unsafe fn flush(i: usize) -> bool {
    let d = DRIVES[i];
    if !d.flush {
        return true;
    }
    if !select(&d, 0) {
        return false;
    }
    outb(d.cmd + 7, if d.lba48 { ATA_FLUSH_EXT } else { ATA_FLUSH });
    settle(d.ctl);
    // a drive may take a while to empty a large cache
    let ok = wait(30_000, || inb(d.ctl) & ST_BSY == 0) && inb(d.cmd + 7) & (ST_ERR | ST_DF) == 0;
    if ok {
        STATS.2 += 1;
    } else {
        crate::kprintln!("[ide] {}: cache flush failed (status {:#x})", name(&d), inb(d.cmd + 7));
    }
    ok
}

unsafe fn fail(i: usize, what: &str, lba: u64) {
    let d = DRIVES[i];
    crate::kprintln!("[ide] {}: {} of sector {} failed (status {:#x}, error {:#x})", name(&d), what, lba, inb(d.cmd + 7), inb(d.cmd + 1));
}

fn name(d: &Drive) -> &'static str {
    match (d.chan, d.slave) {
        (0, false) => "primary master",
        (0, true) => "primary slave",
        (1, false) => "secondary master",
        _ => "secondary slave",
    }
}

/// Soft-reset a channel and look at both its devices.
unsafe fn probe_channel(cmd: u16, ctl: u16, chan: u8) {
    // nothing decodes these ports (no controller or channel switched off): the bus floats high
    if inb(cmd + 7) == 0xFF && inb(ctl) == 0xFF {
        return;
    }
    outb(ctl, CTL_SRST | CTL_NIEN);
    crate::timer::busywait_ms(1);
    outb(ctl, CTL_NIEN);
    crate::timer::busywait_ms(3);
    // a drive spinning up holds BSY for seconds; an empty channel reads 0x7F or 0xFF
    let gone = |s: u8| s == 0xFF || s == 0x7F;
    if !wait(5000, || {
        let s = inb(ctl);
        gone(s) || s & ST_BSY == 0
    }) {
        crate::kprintln!("[ide] channel {:#x}: still busy 5 s after reset: not used", cmd);
        return;
    }
    // the reset signatures of both devices, read before any command: task-file writes reach both
    // devices of a channel, so probing the master would overwrite the slave's
    let mut sigs = [(0u8, 0u8); 2];
    for (k, sig) in sigs.iter_mut().enumerate() {
        outb(cmd + 6, 0xA0 | (k as u8) << 4);
        settle(ctl);
        *sig = (inb(cmd + 4), inb(cmd + 5));
    }
    for slave in [false, true] {
        if N_DRIVES >= 8 {
            return;
        }
        let mut d = Drive { cmd, ctl, chan, slave, ..NONE };
        outb(cmd + 6, 0xA0 | (slave as u8) << 4);
        settle(ctl);
        if gone(inb(cmd + 7)) && gone(inb(ctl)) {
            continue;
        }
        // ATAPI devices answer reset with signature 0x14/0xEB and refuse IDENTIFY DEVICE
        let atapi = |d: Drive| {
            DRIVES[N_DRIVES] = Drive { atapi: true, ..d };
            N_DRIVES += 1;
            crate::kprintln!("[ide] {} ({:#x}): CD/DVD drive (ATAPI): not used", name(&d), cmd);
        };
        if sigs[slave as usize] == (0x14, 0xEB) {
            atapi(d);
            continue;
        }
        for r in 2..6 {
            outb(cmd + r, 0);
        }
        outb(cmd + 7, ATA_IDENTIFY);
        settle(ctl);
        if inb(cmd + 7) == 0 {
            continue; // no device there
        }
        if !wait(3000, || inb(ctl) & ST_BSY == 0) {
            continue;
        }
        // the identify data is ready (DRQ), or the device refused (ERR: not an ATA disk)
        if !wait(1000, || inb(cmd + 7) & (ST_DRQ | ST_ERR) != 0) || inb(cmd + 7) & ST_ERR != 0 {
            // an ATAPI device that refused puts its signature back
            if (inb(cmd + 4), inb(cmd + 5)) == (0x14, 0xEB) {
                atapi(d);
            }
            continue;
        }
        let mut id = [0u16; 256];
        for w in id.iter_mut() {
            *w = inw(cmd);
        }
        settle(ctl);
        if id[49] & 1 << 9 == 0 {
            crate::kprintln!("[ide] {} ({:#x}): disk without LBA addressing: not used", name(&d), cmd);
            continue;
        }
        d.lba48 = id[83] & 0xC000 == 0x4000 && id[83] & 1 << 10 != 0;
        d.sectors = if d.lba48 {
            id[100] as u64 | (id[101] as u64) << 16 | (id[102] as u64) << 32 | (id[103] as u64) << 48
        } else {
            id[60] as u64 | (id[61] as u64) << 16
        };
        d.flush = id[83] & 0xC000 == 0x4000 && id[83] & 1 << 12 != 0 || id[82] != 0xFFFF && id[82] & 1 << 12 != 0;
        for k in 0..20 {
            d.model[2 * k] = (id[27 + k] >> 8) as u8;
            d.model[2 * k + 1] = id[27 + k] as u8;
        }
        let lbs = if id[106] & 0xC000 == 0x4000 && id[106] & 1 << 12 != 0 { (id[117] as u32 | (id[118] as u32) << 16) * 2 } else { 512 };
        let i = N_DRIVES;
        DRIVES[i] = Drive { odd: lbs != 512, ..d };
        N_DRIVES += 1;
        if lbs != 512 {
            crate::kprintln!("[ide] {} ({:#x}): {}-byte sectors, not supported: not used", name(&d), cmd, lbs);
            continue;
        }
        // whose disk is it? only a blank one (after claimdisk ide) or our own is ever written
        let read = read_raw(i, 0);
        let s0 = &*addr_of!(SECTOR);
        let ours = read && &s0[..8] == b"AFRIVFS1";
        let blank = read && s0.iter().all(|&x| x == 0);
        // a GPT disk (Windows) with AfriSovera's partition in it, already formatted
        let part = if read && !ours && !blank && (*addr_of!(DISK)).is_none() { find_part(i) } else { None };
        if ours && (*addr_of!(DISK)).is_none() {
            DISK = Some(i);
            DRIVES[i].usable = true;
        } else if let Some(pa) = part {
            DISK = Some(i);
            PART = pa;
            DRIVES[i].usable = true;
        } else if blank && (*addr_of!(BLANK)).is_none() {
            BLANK = Some(i);
        }
        crate::kprintln!(
            "[ide] {} ({:#x}): ATA disk, {} sectors ({} MiB), {}, {}",
            name(&d),
            cmd,
            d.sectors,
            d.sectors / 2048,
            if d.lba48 { "LBA48" } else { "LBA28" },
            if DISK == Some(i) && part.is_some() {
                "AfriSovera's partition used for storage, the rest left alone"
            } else if DISK == Some(i) {
                "ours (AFRIVFS1): used for storage"
            } else if ours {
                "ours (AFRIVFS1), but another disk is already in use: left alone"
            } else if blank {
                "blank: not used unless claimed (claimdisk ide)"
            } else if !read {
                "sector 0 could not be read: left alone"
            } else {
                "holds other data (Windows?): left alone"
            }
        );
    }
}

/// At boot: AfriSovera's partition on drive `i` in a healthy GPT, already formatted -> (first
/// sector, sectors) to use for the VFS.
fn find_part(i: usize) -> Option<(u64, u64)> {
    // SAFETY: boot context, scalar read.
    let cap = unsafe { DRIVES[i].sectors };
    let (a, l) = crate::gpt::ours_ide(i as u32, cap)?;
    let mut s = [0u8; 512];
    if raw_read(i as u32, a, &mut s) && &s[..8] == b"AFRIVFS1" {
        crate::kprintln!("[ide] ide{}: AfriSovera's partition at sector {} ({} MiB) holds our files", i, a, l / 2048);
        return Some((a, l));
    }
    crate::kprintln!("[ide] ide{}: AfriSovera's partition at sector {} ({} MiB) is not formatted yet (formatpart ide{})", i, a, l / 2048, i);
    None
}

/// The I/O base of BAR `n` of controller `b:d.f`, or None when it is not an I/O BAR.
fn bar_io(b: u8, d: u8, f: u8, n: u8) -> Option<u16> {
    let v = pci::cfg_read32(b, d, f, 0x10 + 4 * n);
    (v & 1 == 1 && v & !3 != 0 && v >> 16 == 0).then_some((v & 0xFFFC) as u16)
}

pub fn init() {
    // every IDE controller (class 01:01, any programming interface), at most two
    // SAFETY: init runs once at boot, single core, before any reader.
    unsafe {
        pci::find_by(8, |b, d, f, _| {
            let c = pci::cfg_read32(b, d, f, 8);
            if c >> 16 == 0x0101 && N_CTRL < 2 {
                CTRLS[N_CTRL] = (b, d, f, (c >> 8) as u8);
                N_CTRL += 1;
            }
            false
        });
        let mut legacy = [false; 2]; // the fixed ports belong to one controller only
        for k in 0..N_CTRL {
            let (b, d, f, pi) = CTRLS[k];
            pci::enable(b, d, f, 0x401); // I/O decoding, INTx off (polled)
            crate::kprintln!(
                "[ide] controller {:02x}:{:02x}.{} (IDE mode, interface {:#04x}: primary {}, secondary {})",
                b, d, f, pi,
                if pi & 1 != 0 { "native" } else { "compatibility" },
                if pi & 4 != 0 { "native" } else { "compatibility" }
            );
            for chan in 0..2u8 {
                let native = pi & (1 << (2 * chan)) != 0;
                let ports = if native {
                    match (bar_io(b, d, f, 2 * chan), bar_io(b, d, f, 2 * chan + 1)) {
                        (Some(c), Some(t)) => Some((c, t + 2)),
                        _ => None,
                    }
                } else if !legacy[chan as usize] {
                    legacy[chan as usize] = true;
                    Some(if chan == 0 { (0x1F0, 0x3F6) } else { (0x170, 0x376) })
                } else {
                    None
                };
                match ports {
                    Some((c, t)) => probe_channel(c, t, chan),
                    None => crate::kprintln!("[ide] channel {} of {:02x}:{:02x}.{}: no I/O ports: not used", chan, b, d, f),
                }
            }
        }
        if N_CTRL > 0 {
            let (nc, nd) = (N_CTRL, N_DRIVES);
            let disks = (0..nd).filter(|&i| !DRIVES[i].atapi).count();
            crate::kprintln!("[ide] {} controller(s), {} disk(s), {} CD/DVD drive(s)", nc, disks, nd - disks);
        }
        if present() {
            multi_check();
        }
    }
}

/// `claimdisk ide`: take the blank IDE-mode disk for the VFS (the owner asked for it).
pub fn claim() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if let (None, Some(i)) = (DISK, BLANK) {
            DISK = Some(i);
            BLANK = None;
            DRIVES[i].usable = true;
            multi_check();
            return true;
        }
        false
    }
}

pub fn present() -> bool {
    // SAFETY: scalar read.
    unsafe { (*addr_of!(DISK)).is_some() }
}

pub fn capacity() -> u64 {
    // SAFETY: scalar reads.
    unsafe { (*addr_of!(DISK)).map_or(0, |i| if PART.1 > 0 { PART.1 } else { DRIVES[i].sectors }) }
}

pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    // SAFETY: single core, kernel context, one command at a time.
    unsafe {
        let Some(i) = DISK else { return false };
        // writes still gathered go to the drive before anything is read
        if n >= capacity() || !(send(i) && fetch(i, n + PART.0)) {
            return false;
        }
        out.copy_from_slice(&*addr_of!(SECTOR));
        true
    }
}

pub fn write_sector(n: u64, data: &[u8; 512]) -> bool {
    // the VFS's last sector (or a lone write past its area: the self-test) ends a save: the drive
    // then empties its write cache, so a power-off right after sync keeps the files
    let last = n >= 2 * crate::vfs::MAX_FILES as u64;
    // the VFS saves its sectors in order, so a run is gathered in WB (up to KMAX sectors; one while
    // multi-sector writes are off) and sent when full, on a gap, before any read, or at the last
    // sector
    // SAFETY: as read_sector.
    unsafe {
        let Some(i) = DISK else { return false };
        if n >= capacity() {
            return false;
        }
        let n = n + PART.0;
        let cap = if MULTI == 1 && MULTIW != 2 { KMAX } else { 1 };
        let mut ok = true;
        if WB_N > 0 && n != WB_AT + WB_N as u64 {
            ok = send(i);
        }
        if WB_N == 0 {
            WB_AT = n;
        }
        let off = WB_N as usize * 512;
        (&mut *addr_of_mut!(WB))[off..off + 512].copy_from_slice(data);
        WB_N += 1;
        WSTATS.0 += 1;
        if last || WB_N >= cap {
            ok = send(i) && ok;
        }
        ok && (!last || flush(i))
    }
}

/// For gpt.rs (lsparts, installpart, ...): the i-th drive as a disk, if it is an ATA disk with
/// 512-byte sectors: (its number, 512-byte sectors, sector bytes). CD/DVD drives are skipped.
pub fn raw_disk(i: usize) -> Option<(u32, u64, u64)> {
    // SAFETY: scalar reads of boot-time state.
    unsafe {
        let n = N_DRIVES;
        (0..n).filter(|&k| !DRIVES[k].atapi && !DRIVES[k].odd).nth(i).map(|k| (k as u32, DRIVES[k].sectors, 512))
    }
}

fn disk_ok(k: u32) -> bool {
    // SAFETY: scalar reads of boot-time state.
    unsafe { (k as usize) < N_DRIVES && !DRIVES[k as usize].atapi && !DRIVES[k as usize].odd }
}

/// Read one sector of drive `k` (ide`k`) whoever owns it (never writes).
pub fn raw_read(k: u32, n: u64, out: &mut [u8; 512]) -> bool {
    if !disk_ok(k) {
        return false;
    }
    // SAFETY: single core, kernel context, one command at a time.
    unsafe {
        // the storage drive's gathered writes go first, so this reads what the drive will hold
        if DISK == Some(k as usize) && !send(k as usize) {
            return false;
        }
        if !read_raw(k as usize, n) {
            return false;
        }
        out.copy_from_slice(&*addr_of!(SECTOR));
    }
    true
}

/// Write one sector of drive `k` whoever owns it, cache flushed after it. ONLY installpart,
/// tablerestore, installdisk and bootcopy call this (gpt.rs, esp.rs), and only after a matching
/// table backup on a USB stick and the owner's code.
pub fn table_write(k: u32, n: u64, data: &[u8; 512]) -> bool {
    if !disk_ok(k) {
        return false;
    }
    // SAFETY: as raw_read.
    unsafe {
        let mine = DISK == Some(k as usize);
        if mine && !send(k as usize) {
            return false;
        }
        (*addr_of_mut!(SECTOR)).copy_from_slice(data);
        let ok = write_raw(k as usize, n);
        if mine {
            ra_update(n, data, ok);
        }
        ok && flush(k as usize)
    }
}

/// Write `data` (1 to 16 whole sectors) from sector `n` of drive `k` whoever owns it: ONE WRITE
/// SECTORS, read back and compared, then ONE cache flush, so installdisk and bootcopy do not flush
/// after every sector. A run refused or read back different is written again one sector at a time
/// (table_write) and runs stay off until power-off, with one line. The same callers and the same
/// rule as table_write.
pub fn table_write_run(k: u32, n: u64, data: &[u8]) -> bool {
    let c = data.len() / 512;
    if c == 0 || c * 512 != data.len() || c > KMAX as usize || !disk_ok(k) {
        return false;
    }
    let single = || data.chunks_exact(512).enumerate().all(|(j, s)| table_write(k, n + j as u64, s.try_into().unwrap()));
    // SAFETY: as raw_read; xfer only reads `data` when writing.
    unsafe {
        if c == 1 || TRUN == 2 {
            return single();
        }
        let i = k as usize;
        let mine = DISK == Some(i);
        if mine && !send(i) {
            return false;
        }
        let rb = addr_of_mut!(TRB) as *mut u8;
        let ok = xfer(i, n, c as u32, data.as_ptr() as *mut u8, true)
            && xfer(i, n, c as u32, rb, false)
            && (&*addr_of!(TRB))[..data.len()] == *data;
        if mine {
            for j in 0..c {
                ra_update(n + j as u64, &data[j * 512..j * 512 + 512], ok);
            }
        }
        if !ok {
            crate::kprintln!(
                "[ide] ide{}: {} sectors written in one command were refused or read back different, so big copies go one sector at a time from now on",
                k, c
            );
            TRUN = 2;
            return single();
        }
        TRUN = 1;
        TSTATS.0 += 1;
        TSTATS.1 += c as u64;
        flush(i)
    }
}

/// `formatpart ideN`: keep the VFS in AfriSovera's partition (first sector, sectors) of drive `k`
/// from now on.
pub fn use_part(k: u32, a: u64, l: u64) -> bool {
    if !disk_ok(k) {
        return false;
    }
    // SAFETY: scalar state, shell context.
    unsafe {
        let i = k as usize;
        if (*addr_of!(DISK)).is_some() || l == 0 || a.saturating_add(l) > DRIVES[i].sectors {
            return false;
        }
        if BLANK == Some(i) {
            BLANK = None;
        }
        PART = (a, l);
        DISK = Some(i);
        DRIVES[i].usable = true;
    }
    multi_check();
    true
}

/// The drive in use and its partition: (drive number, first sector, sectors); sectors 0 = the whole disk.
pub fn part() -> Option<(u32, u64, u64)> {
    // SAFETY: scalar reads.
    unsafe { (*addr_of!(DISK)).map(|i| (i as u32, PART.0, PART.1)) }
}

/// `tablerestore ideN` took AfriSovera's partition on drive `k` out of the table: stop writing there.
pub fn release_part(k: u32) -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if DISK == Some(k as usize) && PART.1 > 0 {
            // the partition is out of the table: nothing more is written there, not even a half save
            (WB_N, RA_N, MULTI, MULTIW) = (0, 0, 0, 0);
            DISK = None;
            PART = (0, 0);
            DRIVES[k as usize].usable = false;
            return true;
        }
    }
    false
}

/// `lside`: the IDE controllers and every drive on them.
pub fn list() {
    // SAFETY: scalar reads of boot-time state.
    unsafe {
        if N_CTRL == 0 {
            crate::kprintln!("ide: no IDE-mode controller (SATA in AHCI mode is lsahci)");
            return;
        }
        for k in 0..N_CTRL {
            let (b, d, f, pi) = CTRLS[k];
            crate::kprintln!("ide: controller {:02x}:{:02x}.{}, interface {:#04x}", b, d, f, pi);
        }
        for i in 0..N_DRIVES {
            let d = DRIVES[i];
            let model = core::str::from_utf8(&d.model).unwrap_or("?").trim();
            if d.atapi {
                crate::kprintln!("  {} ({:#x}): CD/DVD drive (ATAPI), not used", name(&d), d.cmd);
                continue;
            }
            if d.odd {
                crate::kprintln!("  {} ({:#x}): {}, sectors other than 512 bytes, not used", name(&d), d.cmd, model);
                continue;
            }
            crate::kprintln!(
                "  ide{}: {} ({:#x}): {}, {} sectors ({} MiB){}",
                i,
                name(&d),
                d.cmd,
                model,
                d.sectors,
                d.sectors / 2048,
                if DISK == Some(i) && PART.1 > 0 {
                    "  <- storage (AfriSovera's partition)"
                } else if DISK == Some(i) {
                    "  <- storage"
                } else if BLANK == Some(i) {
                    "  (blank: claimdisk ide to use it)"
                } else {
                    "  (other data, read-only to us)"
                }
            );
        }
        let (a, l) = PART;
        if l > 0 {
            crate::kprintln!("  storage partition: sectors {}-{} ({} MiB)", a, a + l - 1, l / 2048);
        }
        if present() {
            let (r, w, fl) = STATS;
            crate::kprintln!("  storage: {} sector(s) read, {} written, {} cache flush(es)", r, w, fl);
            let (r, w, mr, mw) = (RSTATS, WSTATS, MULTI, MULTIW);
            let word = |s: u8| match s { 1 => "on", 2 => "off", _ => "not checked yet" };
            crate::kprintln!("  reads: {} sector(s), {} from the read-ahead, {} multi-sector command(s), {} single ({})", r.0, r.1, r.2, r.3, word(mr));
            crate::kprintln!("  writes: {} sector(s), {} multi-sector command(s), {} single ({})", w.0, w.1, w.2, word(mw));
        }
        if TRUN != 0 {
            let t = TSTATS;
            crate::kprintln!(
                "  big copies (installdisk, bootcopy): {} run(s) in one command, {} sector(s), each read back and flushed once ({})",
                t.0, t.1, if TRUN == 1 { "on" } else { "off" }
            );
        }
    }
}
