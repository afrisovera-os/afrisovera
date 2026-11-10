//! AHCI (SATA) disks on the PC: roadmap PC step 3's first real driver (5 Nov 2026).
//!
//! Every PC chipset since ~2008 has an AHCI controller (PCI class 01:06:01), and QEMU's q35
//! board has one built in (ICH9, 00:1f.2), so this is the disk a PC kernel meets first.
//! One command slot, one PRD, polled: the same shape as the virtio-blk path it stands in
//! for (`read_sector` / `write_sector` of 512 bytes), so the shared VFS needs no change.
//!
//! Safety rule for the user's disks: the VFS stores files on an AHCI disk only if sector 0
//! is blank or already carries the VFS marker. A disk with a boot sector, a partition table
//! or a file system (0x55AA at 510) is somebody's data -- on QEMU, the UEFI boot disk itself
//! -- and is listed but never written.
//! Multi-block commands (10 Nov 2026), as NVMe and the USB disks got on 9-10 Nov: on the storage
//! disk a read fetches up to 8 KiB (16 sectors, or 2 on a 4Kn drive) with one READ DMA EXT into
//! a read-ahead the next reads are served from, and the VFS's in-order save is gathered by device
//! block (a 4Kn drive's block read once, its eight sectors changed in place) into runs of up to
//! 8 KiB sent with one WRITE DMA EXT. Each way is checked once against one block at a time (reads
//! at start-up or claim, writes at the first save, read back); a mismatch or a refusal turns it off.

use crate::mmu;
use crate::pci;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

// --- HBA registers (AHCI 1.3.1, section 3) ---
const CAP: u64 = 0x00;
const GHC: u64 = 0x04;
const PI: u64 = 0x0C;
const CAP2: u64 = 0x24;
const BOHC: u64 = 0x28;
const VS: u64 = 0x10;
const GHC_AE: u32 = 1 << 31;
const P_CLB: u64 = 0x00;
const P_CLBU: u64 = 0x04;
const P_FB: u64 = 0x08;
const P_FBU: u64 = 0x0C;
const P_IS: u64 = 0x10;
const P_IE: u64 = 0x14;
const P_CMD: u64 = 0x18;
const P_TFD: u64 = 0x20;
const P_SIG: u64 = 0x24;
const P_SSTS: u64 = 0x28;
const P_SCTL: u64 = 0x2C;
const P_SERR: u64 = 0x30;
const P_CI: u64 = 0x38;
const CMD_ST: u32 = 1 << 0;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_ERR: u32 = 1 << 0;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;
const IS_TFES: u32 = 1 << 30;
const SIG_ATA: u32 = 0x0000_0101;

const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_IDENTIFY: u8 = 0xEC;

// --- DMA structures: 1:1-mapped kernel memory, so their addresses are physical ---
#[repr(C, align(1024))]
struct CmdList([u32; 8 * 32]); // 32 headers of 8 dwords
#[repr(C, align(256))]
struct RecvFis([u8; 256]);
#[repr(C, align(128))]
struct CmdTable([u8; 0x80 + 16]); // CFIS/ACMD/reserved, then one PRD
#[repr(C, align(4096))]
struct Buf([u8; 4096]); // one logical sector: 512 B, or 4 KiB on a 4Kn drive

static mut CLIST: CmdList = CmdList([0; 256]);
static mut RFIS: RecvFis = RecvFis([0; 256]);
static mut CTBL: CmdTable = CmdTable([0; 0x80 + 16]);
static mut BUF: Buf = Buf([0; 4096]);
/// Each port's logical sector size in bytes (512, or 4096 on 4Kn drives; IDENTIFY word 106/117).
static mut LBS: [u32; 32] = [512; 32];

static mut ABAR: u64 = 0;
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut PORTS: u32 = 0;
static mut VERSION: u32 = 0;
/// The port whose disk the VFS may use (`None` = no usable disk).
static mut DISK_PORT: Option<u32> = None;
static mut DISK_SECTORS: u64 = 0;
/// AfriSovera's own GPT partition on DISK_PORT holding the VFS (first 512-byte sector, sectors);
/// (0, 0) = the whole disk. Set at boot when the partition already holds AFRIVFS1, or by
/// `formatpart sataN`; sector n of the VFS is then sector PART.0 + n of the disk.
static mut PART: (u64, u64) = (0, 0);
/// Every SATA disk found: (port, sectors, usable for storage).
static mut FOUND: [(u32, u64, bool); 32] = [(0, 0, false); 32];
/// A blank SATA disk (port, sectors), used only after `claimdisk sata`.
static mut BLANK: Option<(u32, u64)> = None;
static mut N_FOUND: usize = 0;
static mut MODEL: [u8; 40] = [b' '; 40];
/// Multi-block commands on DISK_PORT: the read-ahead (RA_N device blocks from RA_AT) and the
/// gathered writes (WB_N device blocks from WB_AT), 8 KiB each, each one PRD.
const RA_BYTES: usize = 8192;
#[repr(C, align(8192))]
struct Ahead([u8; RA_BYTES]);
static mut RA: Ahead = Ahead([0; RA_BYTES]);
static mut RA_AT: u64 = 0;
static mut RA_N: u32 = 0;
static mut WB: Ahead = Ahead([0; RA_BYTES]);
static mut WB_AT: u64 = 0;
static mut WB_N: u32 = 0;
/// 0 = not checked, 1 = on, 2 = off; reads and writes apart.
static mut MULTI: u8 = 0;
static mut MULTIW: u8 = 0;
/// Reads: sectors asked for, served from the read-ahead, multi-block commands, single-block ones.
static mut RSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Writes: sectors written, multi-block commands, single-block ones.
static mut WSTATS: (u64, u64, u64) = (0, 0, 0);

fn hba(off: u64) -> u32 {
    // SAFETY: ABAR is mapped uncached by init before any caller runs.
    unsafe { read_volatile((ABAR + off) as *const u32) }
}
fn hba_w(off: u64, v: u32) {
    // SAFETY: as hba.
    unsafe { write_volatile((ABAR + off) as *mut u32, v) }
}
fn port(p: u32, off: u64) -> u32 {
    hba(0x100 + p as u64 * 0x80 + off)
}
fn port_w(p: u32, off: u64, v: u32) {
    hba_w(0x100 + p as u64 * 0x80 + off, v)
}

fn wait(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..50_000_000u64 {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Stop a port's command engine (AHCI 10.1.2), so its list/FIS pointers may be changed.
fn stop(p: u32) -> bool {
    port_w(p, P_CMD, port(p, P_CMD) & !CMD_ST);
    if !wait(|| port(p, P_CMD) & CMD_CR == 0) {
        return false;
    }
    port_w(p, P_CMD, port(p, P_CMD) & !CMD_FRE);
    wait(|| port(p, P_CMD) & CMD_FR == 0)
}

/// Point port `p` at our command list and FIS area and start it.
fn start(p: u32) -> bool {
    if !stop(p) {
        return false;
    }
    let (cl, fb) = (addr_of!(CLIST) as u64, addr_of!(RFIS) as u64);
    port_w(p, P_CLB, cl as u32);
    port_w(p, P_CLBU, (cl >> 32) as u32);
    port_w(p, P_FB, fb as u32);
    port_w(p, P_FBU, (fb >> 32) as u32);
    port_w(p, P_SERR, 0xFFFF_FFFF);
    port_w(p, P_IS, 0xFFFF_FFFF);
    port_w(p, P_IE, 0); // polled
    port_w(p, P_CMD, port(p, P_CMD) | CMD_FRE);
    port_w(p, P_CMD, port(p, P_CMD) | CMD_ST);
    true
}

/// One ATA command through slot 0 with one PRD over BUF (one logical sector). Returns success.
fn command(p: u32, ata: u8, lba: u64, write: bool) -> bool {
    xfer(p, ata, lba, 1, addr_of!(BUF) as u64, write)
}

/// One ATA command of `k` logical sectors through slot 0 with one PRD over `buf` (BUF, RA or WB:
/// physically contiguous, k * sector size bytes). Returns success.
fn xfer(p: u32, ata: u8, lba: u64, k: u32, buf: u64, write: bool) -> bool {
    if !wait(|| port(p, P_TFD) & (TFD_BSY | TFD_DRQ) == 0) {
        crate::kprintln!("[ahci] port {} busy", p);
        return false;
    }
    let ct = addr_of!(CTBL) as u64;
    // SAFETY: the DMA structures are ours alone; the HBA reads them only after CI is set.
    unsafe {
        let t = &mut (*addr_of_mut!(CTBL)).0;
        t.fill(0);
        // Register H2D FIS
        t[0] = 0x27;
        t[1] = 0x80; // C: this is a command
        t[2] = ata;
        t[4] = lba as u8;
        t[5] = (lba >> 8) as u8;
        t[6] = (lba >> 16) as u8;
        t[7] = 0x40; // LBA mode
        t[8] = (lba >> 24) as u8;
        t[9] = (lba >> 32) as u8;
        t[10] = (lba >> 40) as u8;
        t[12] = k as u8; // sector count (0 would mean 65536: never sent)
        t[13] = (k >> 8) as u8;
        // PRD 0: the buffer, byte count - 1
        t[0x80..0x84].copy_from_slice(&(buf as u32).to_le_bytes());
        t[0x84..0x88].copy_from_slice(&((buf >> 32) as u32).to_le_bytes());
        // byte count - 1: IDENTIFY is always 512 bytes, data is one logical sector
        let bytes = if ata == ATA_IDENTIFY { 512 } else { LBS[p as usize & 31] * k };
        t[0x8C..0x90].copy_from_slice(&(bytes - 1).to_le_bytes());
        let h = &mut (*addr_of_mut!(CLIST)).0;
        h[0] = 5 | if write { 1 << 6 } else { 0 } | 1 << 16; // CFL 5 dwords, W, PRDTL 1
        h[1] = 0; // PRDBC
        h[2] = ct as u32;
        h[3] = (ct >> 32) as u32;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    port_w(p, P_IS, 0xFFFF_FFFF);
    port_w(p, P_CI, 1);
    let ok = wait(|| port(p, P_CI) & 1 == 0 || port(p, P_IS) & IS_TFES != 0);
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    if !ok || port(p, P_IS) & IS_TFES != 0 || port(p, P_TFD) & TFD_ERR != 0 {
        crate::kprintln!(
            "[ahci] port {} command {:#04x} lba {} failed (IS {:#x}, TFD {:#x})",
            p, ata, lba, port(p, P_IS), port(p, P_TFD)
        );
        // recover the port for the next command
        start(p);
        return false;
    }
    true
}

fn buf() -> &'static mut [u8; 4096] {
    // SAFETY: single core, kernel context; only one command is ever in flight.
    unsafe { &mut (*addr_of_mut!(BUF)).0 }
}

/// Find the controller, bring up every SATA disk, and choose the one the VFS may use.
pub fn init() {
    let Some((b, d, f)) = pci::find_class(0x01_06_01, 8) else { return };
    let abar = (pci::cfg_read32(b, d, f, 0x24) & !0xF) as u64; // BAR5 (always 32-bit in AHCI)
    if abar == 0 || !mmu::map_mmio(abar, 0x1100) {
        crate::kprintln!("[ahci] controller at {:02x}:{:02x}.{} has no usable ABAR", b, d, f);
        return;
    }
    pci::enable(b, d, f, 0x406); // memory decoding + bus mastering, INTx off (polled)
    // SAFETY: init runs once at boot, single core, before any reader.
    unsafe {
        ABAR = abar;
        PCI_AT = (b, d, f);
    }
    // take the controller from the firmware (BIOS/OS handoff, AHCI 10.6) when it offers one
    if hba(CAP2) & 1 != 0 {
        hba_w(BOHC, hba(BOHC) | 1 << 1); // OS owned
        if !wait(|| hba(BOHC) & 1 == 0) {
            crate::kprintln!("[ahci] the firmware did not release the controller; continuing");
        }
    }
    hba_w(GHC, hba(GHC) | GHC_AE);
    // a 32-bit-only HBA cannot reach DMA structures above 4 GiB
    if hba(CAP) & 1 << 31 == 0 && [addr_of!(CLIST) as u64, addr_of!(RFIS) as u64, addr_of!(CTBL) as u64, addr_of!(BUF) as u64, addr_of!(RA) as u64, addr_of!(WB) as u64].iter().any(|&a| a >> 32 != 0) {
        crate::kprintln!("[ahci] the controller has 32-bit DMA and our buffers are above 4 GiB: not used");
        return;
    }
    let pi = hba(PI);
    // SAFETY: as above.
    unsafe {
        PORTS = pi;
        VERSION = hba(VS);
    }
    let cap = hba(CAP);
    for p in 0..32u32 {
        if pi & (1 << p) == 0 {
            continue;
        }
        // spin the device up and power it on (staggered spin-up / cold presence detect)
        let want = if cap & 1 << 27 != 0 { 1 << 1 } else { 0 } | if cap & 1 << 24 != 0 { 1 << 2 } else { 0 };
        if want != 0 && port(p, P_CMD) & want != want {
            port_w(p, P_CMD, port(p, P_CMD) | want);
        }
        // a link that is not up (a sleeping SSD after a cold boot): COMRESET, then wait for it
        if port(p, P_SSTS) & 0xF != 3 {
            port_w(p, P_SCTL, (port(p, P_SCTL) & !0xF) | 1);
            crate::timer::busywait_ms(2);
            port_w(p, P_SCTL, port(p, P_SCTL) & !0xF);
            let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
            while port(p, P_SSTS) & 0xF != 3 && crate::timer::cycles() - t0 < hz {
                core::hint::spin_loop();
            }
            if port(p, P_SSTS) & 0xF != 3 {
                continue; // nothing there
            }
            crate::timer::busywait_ms(10); // let the drive settle before its first command
            port_w(p, P_SERR, 0xFFFF_FFFF);
        }
        if port(p, P_SIG) != SIG_ATA {
            continue;
        }
        if !start(p) || !command(p, ATA_IDENTIFY, 0, false) {
            continue;
        }
        let id = buf();
        let w = |i: usize| u16::from_le_bytes([id[i * 2], id[i * 2 + 1]]) as u64;
        let logical = w(100) | w(101) << 16 | w(102) << 32 | w(103) << 48;
        // logical sector size: word 106 valid (bits 15:14 = 01) and bit 12 set -> words 117-118
        // hold it in 16-bit words (4Kn drives: 2048 words = 4096 bytes); else 512
        let lbs = if w(106) & 0xC000 == 0x4000 && w(106) & 1 << 12 != 0 { ((w(117) | w(118) << 16) * 2) as u32 } else { 512 };
        if !matches!(lbs, 512 | 1024 | 2048 | 4096) {
            crate::kprintln!("[ahci] port {}: {}-byte sectors, not supported", p, lbs);
            continue;
        }
        // SAFETY: as above.
        unsafe { LBS[p as usize] = lbs };
        let sectors = logical * (lbs as u64 / 512); // in 512-byte sectors, what the VFS speaks
        let mut model = [b' '; 40];
        for i in 0..20 {
            model[i * 2] = id[54 + i * 2 + 1];
            model[i * 2 + 1] = id[54 + i * 2];
        }
        // Whose disk is it? Only a blank one or our own is ever written.
        // an internal disk is used on its own only if it is already ours; a blank one waits
        // for the owner to claim it (`claimdisk sata`)
        let read = command(p, ATA_READ_DMA_EXT, 0, false);
        let s0 = &buf()[..512];
        let whole = read && &s0[..8] == b"AFRIVFS1";
        let blank = read && s0.iter().all(|&x| x == 0);
        // a disk with another system on it: used only through AfriSovera's own partition
        // (made by installpart, formatted by formatpart), never outside it
        // SAFETY: as above.
        let part = if read && !whole && !blank && unsafe { (*addr_of!(DISK_PORT)).is_none() } { find_part(p, sectors, lbs as u64) } else { None };
        let usable = whole || part.is_some();
        // SAFETY: as above.
        unsafe {
            if blank && (*addr_of!(BLANK)).is_none() {
                BLANK = Some((p, sectors));
            }
        }
        // SAFETY: as above.
        unsafe {
            if N_FOUND < 32 {
                FOUND[N_FOUND] = (p, sectors, usable);
                N_FOUND += 1;
            }
            if usable && (*addr_of!(DISK_PORT)).is_none() {
                DISK_PORT = Some(p);
                DISK_SECTORS = sectors;
                MODEL = model;
                if let Some((a, l)) = part {
                    PART = (a, l);
                    DISK_SECTORS = l;
                }
            }
        }
        crate::kprintln!(
            "[ahci] port {}: SATA disk, {} sectors ({} MiB), {}",
            p,
            sectors,
            sectors / 2048,
            if part.is_some() {
                "AfriSovera's partition (AFRIVFS1): used for storage, the rest left alone"
            } else if usable {
                "ours (AFRIVFS1): used for storage"
            } else if blank {
                "blank: not used unless claimed (claimdisk sata)"
            } else {
                "holds other data (boot disk?): left alone"
            }
        );
    }
    // SAFETY: as above.
    let n = unsafe { N_FOUND };
    crate::kprintln!(
        "[ahci] controller {:02x}:{:02x}.{} (AHCI {}.{}), {} port(s), {} disk(s)",
        b, d, f, hba(VS) >> 16, (hba(VS) >> 8) & 0xFF, pi.count_ones(), n
    );
    let _ = hba(CAP);
    if present() {
        multi_check();
    }
}

/// Device blocks per multi-block command on DISK_PORT (8 KiB over the sector size).
fn kmax() -> u32 {
    // SAFETY: scalar read.
    let Some(p) = (unsafe { DISK_PORT }) else { return 1 };
    (RA_BYTES as u64 / (per(p) * 512)) as u32
}

/// The device block after the storage area (its partition, or the whole disk).
fn end_block() -> u64 {
    // SAFETY: scalar reads.
    let Some(p) = (unsafe { DISK_PORT }) else { return 0 };
    // SAFETY: scalar reads.
    (unsafe { PART.0 + DISK_SECTORS }) / per(p)
}

/// When a disk is taken for storage: one READ of the first k blocks of the storage area must equal
/// the same blocks read one at a time, else multi-block reads stay off (and writes with them).
/// Reads only: safe on any disk. One line says which.
fn multi_check() {
    // SAFETY: single core, kernel/shell context; no command in flight.
    unsafe {
        (MULTI, MULTIW, RA_N, WB_N) = (0, 0, 0, 0);
        let Some(p) = DISK_PORT else { return };
        let first = PART.0 / per(p);
        let k = (kmax() as u64).min(end_block().saturating_sub(first)) as u32;
        if k < 2 {
            (MULTI, MULTIW) = (2, 2);
            return;
        }
        let ra = addr_of_mut!(RA.0) as *mut u8;
        core::ptr::write_bytes(ra, 0xA5, RA_BYTES);
        let bs = LBS[p as usize & 31] as usize;
        let read_ok = xfer(p, ATA_READ_DMA_EXT, first, k, ra as u64, false);
        let mut same = read_ok;
        for b in 0..k as u64 {
            if !same {
                break;
            }
            same = command(p, ATA_READ_DMA_EXT, first + b, false) && buf()[..bs] == core::slice::from_raw_parts(ra.add(b as usize * bs), bs)[..];
        }
        if same {
            (MULTI, RA_AT, RA_N) = (1, first, k);
            crate::kprintln!("[ahci] port {}: multi-block reads on (up to {} sectors per command; the first {} equal one at a time)", p, k, k);
        } else {
            (MULTI, MULTIW, RA_N) = (2, 2, 0);
            crate::kprintln!(
                "[ahci] port {}: multi-block reads off ({}): one sector per command",
                p,
                if read_ok { "they differed from one at a time" } else { "the disk refused one" }
            );
        }
    }
}

/// Device block `b` of DISK_PORT into BUF: from the read-ahead when it holds it, else a new
/// multi-block read of up to 8 KiB from it (once the check passed), else one block.
fn fetch(p: u32, b: u64) -> bool {
    // SAFETY: single core, kernel context, one command in flight.
    unsafe {
        RSTATS.0 += 1;
        let bs = LBS[p as usize & 31] as usize;
        if b >= RA_AT && b < RA_AT + RA_N as u64 {
            let off = (b - RA_AT) as usize * bs;
            buf()[..bs].copy_from_slice(&(&*addr_of!(RA.0))[off..off + bs]);
            RSTATS.1 += 1;
            return true;
        }
        if MULTI == 1 {
            let k = (kmax() as u64).min(end_block().saturating_sub(b)) as u32;
            if k > 1 {
                RA_N = 0;
                if xfer(p, ATA_READ_DMA_EXT, b, k, addr_of!(RA.0) as u64, false) {
                    (RA_AT, RA_N) = (b, k);
                    buf()[..bs].copy_from_slice(&(&*addr_of!(RA.0))[..bs]);
                    RSTATS.2 += 1;
                    return true;
                }
                MULTI = 2;
                crate::kprintln!("[ahci] a multi-block read at sector {} was refused: one sector per command from now on", b);
            }
        }
        RSTATS.3 += 1;
        command(p, ATA_READ_DMA_EXT, b, false)
    }
}

/// Device block `b` of DISK_PORT was written from `src` (or the write failed): keep the read-ahead true.
fn ra_update(b: u64, src: &[u8], ok: bool) {
    // SAFETY: as fetch.
    unsafe {
        if b >= RA_AT && b < RA_AT + RA_N as u64 {
            if ok {
                let Some(p) = DISK_PORT else { return };
                let off = (b - RA_AT) as usize * LBS[p as usize & 31] as usize;
                (&mut (*addr_of_mut!(RA)).0)[off..off + src.len()].copy_from_slice(src);
            } else {
                RA_N = 0; // the disk's copy is unknown now
            }
        }
    }
}

/// Send the gathered writes: one WRITE DMA EXT when multi-block writes are on (the first one is
/// read back block by block and must be equal, else they go off and the run is rewritten one block
/// at a time), else one per block.
fn flush() -> bool {
    // SAFETY: as fetch.
    unsafe {
        let (at, k) = (WB_AT, WB_N);
        if k == 0 {
            return true;
        }
        WB_N = 0;
        let Some(p) = DISK_PORT else { return false };
        let wb = addr_of!(WB.0) as *const u8;
        let bs = LBS[p as usize & 31] as usize;
        let mut done = false;
        if k > 1 && MULTI == 1 && MULTIW != 2 {
            if xfer(p, ATA_WRITE_DMA_EXT, at, k, wb as u64, true) {
                WSTATS.1 += 1;
                done = true;
                if MULTIW == 0 {
                    for j in 0..k as u64 {
                        let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
                        if !(command(p, ATA_READ_DMA_EXT, at + j, false) && buf()[..bs] == s[..]) {
                            done = false;
                            break;
                        }
                    }
                    MULTIW = if done { 1 } else { 2 };
                    if done {
                        crate::kprintln!("[ahci] port {}: multi-block writes on (up to {} sectors per command; the first {} read back equal)", p, kmax(), k);
                    } else {
                        crate::kprintln!("[ahci] port {}: multi-block writes off (read back differently: rewritten one sector per command)", p);
                    }
                }
            } else {
                MULTIW = 2;
                crate::kprintln!("[ahci] a multi-block write at sector {} was refused: one sector per command from now on", at);
            }
        }
        let mut ok = true;
        for j in 0..k as u64 {
            let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
            let good = done || {
                buf()[..bs].copy_from_slice(s);
                WSTATS.2 += 1;
                command(p, ATA_WRITE_DMA_EXT, at + j, true)
            };
            ra_update(at + j, s, good);
            ok = ok && good;
        }
        ok
    }
}

/// `claimdisk sata`: take the blank SATA disk for the VFS (the owner asked for it).
pub fn claim() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if let (None, Some((p, s))) = (DISK_PORT, *addr_of!(BLANK)) {
            DISK_PORT = Some(p);
            DISK_SECTORS = s;
            BLANK = None;
            keep_model(p);
            multi_check();
            return true;
        }
        false
    }
}

/// The model name of the disk on port `p` for lsahci, as init keeps it for a disk used from boot
/// (a disk taken later by claimdisk or formatpart is identified again here).
fn keep_model(p: u32) {
    if command(p, ATA_IDENTIFY, 0, false) {
        let id = buf();
        // SAFETY: scalar state, shell context.
        let m = unsafe { &mut *addr_of_mut!(MODEL) };
        for i in 0..20 {
            m[i * 2] = id[54 + i * 2 + 1];
            m[i * 2 + 1] = id[54 + i * 2];
        }
    }
}

/// At boot: AfriSovera's partition on port `p` in a healthy GPT, already formatted -> (first
/// sector, sectors) to use for the VFS.
fn find_part(p: u32, sectors: u64, lbs: u64) -> Option<(u64, u64)> {
    let (a, l) = crate::gpt::ours_sata(p, sectors, lbs)?;
    let mut s = [0u8; 512];
    if raw_read(p, a, &mut s) && &s[..8] == b"AFRIVFS1" {
        crate::kprintln!("[ahci] port {}: AfriSovera's partition at sector {} ({} MiB) holds our files", p, a, l / 2048);
        return Some((a, l));
    }
    crate::kprintln!("[ahci] port {}: AfriSovera's partition at sector {} ({} MiB) is not formatted yet (formatpart sata{})", p, a, l / 2048, p);
    None
}

/// `formatpart sataN`: keep the VFS in AfriSovera's partition (first sector, sectors) of the
/// disk on port `p` from now on.
pub fn use_part(p: u32, a: u64, l: u64) -> bool {
    let Some(cap) = (0..32).filter_map(raw_disk).find(|d| d.0 == p).map(|d| d.1) else { return false };
    // SAFETY: scalar state, shell context.
    unsafe {
        if (*addr_of!(DISK_PORT)).is_some() || l == 0 || a.saturating_add(l) > cap {
            return false;
        }
        if (*addr_of!(BLANK)).is_some_and(|b| b.0 == p) {
            BLANK = None;
        }
        PART = (a, l);
        DISK_SECTORS = l;
        DISK_PORT = Some(p);
        for i in 0..N_FOUND {
            if FOUND[i].0 == p {
                FOUND[i].2 = true;
            }
        }
    }
    keep_model(p);
    multi_check();
    true
}

/// The disk in use and its partition: (port, first sector, sectors); sectors 0 = the whole disk.
pub fn part() -> Option<(u32, u64, u64)> {
    // SAFETY: scalar reads.
    unsafe { (*addr_of!(DISK_PORT)).map(|p| (p, PART.0, PART.1)) }
}

/// `tablerestore sataN` took AfriSovera's partition on port `p` out of the table: stop writing there.
pub fn release_part(p: u32) -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if DISK_PORT == Some(p) && PART.1 > 0 {
            // the partition is out of the table: nothing more is written there, not even a half save
            (WB_N, RA_N, MULTI, MULTIW) = (0, 0, 0, 0);
            DISK_PORT = None;
            DISK_SECTORS = 0;
            PART = (0, 0);
            for i in 0..N_FOUND {
                if FOUND[i].0 == p {
                    FOUND[i].2 = false;
                }
            }
            return true;
        }
    }
    false
}

pub fn present() -> bool {
    // SAFETY: scalar read.
    unsafe { (*addr_of!(DISK_PORT)).is_some() }
}

pub fn capacity() -> u64 {
    // SAFETY: scalar read.
    unsafe { DISK_SECTORS }
}

/// 512-byte sectors per logical sector of port `p`.
fn per(p: u32) -> u64 {
    // SAFETY: scalar read.
    (unsafe { LBS[p as usize & 31] } as u64 / 512).max(1)
}

pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    // SAFETY: scalar read.
    let Some(p) = (unsafe { DISK_PORT }) else { return false };
    if n >= capacity() {
        return false;
    }
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // writes still gathered go to the disk before anything is read
    if !(flush() && fetch(p, n / per(p))) {
        return false;
    }
    let off = (n % per(p)) as usize * 512;
    out.copy_from_slice(&buf()[off..off + 512]);
    true
}

pub fn write_sector(n: u64, data: &[u8; 512]) -> bool {
    // SAFETY: scalar read.
    let Some(p) = (unsafe { DISK_PORT }) else { return false };
    if n >= capacity() {
        return false;
    }
    // the VFS saves its sectors in order, so a run of device blocks is gathered in WB (up to 8 KiB;
    // one block while multi-block writes are off). A 1, 2 or 4 KiB sector is read once when the
    // run reaches it and its 512-byte pieces changed in place. The run is sent when it is full and
    // its last block complete, on a gap, before any read, or at the VFS's last sector (or a lone
    // write past the VFS area: the self-test).
    let last = n >= 2 * crate::vfs::MAX_FILES as u64;
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // SAFETY: single core, kernel context, one command in flight.
    unsafe {
        let (per, bs) = (per(p), LBS[p as usize & 31] as usize);
        let (b, off) = (n / per, (n % per) as usize * 512);
        let cap = if MULTI == 1 && MULTIW != 2 { kmax() } else { 1 };
        let mut ok = true;
        let tail = WB_AT + WB_N as u64; // the block after the run
        if WB_N > 0 && b + 1 != tail && !(b == tail && WB_N < cap) {
            ok = flush();
        }
        let new = WB_N == 0 || b == WB_AT + WB_N as u64;
        // a new block's other sectors as they are on the disk (the read-ahead, kept equal to every
        // block flushed, or the disk)
        let got = !new || per == 1 || fetch(p, b);
        if got {
            let wb = &mut (*addr_of_mut!(WB)).0;
            if WB_N == 0 {
                WB_AT = b;
            }
            if new {
                if per > 1 {
                    let i = WB_N as usize * bs;
                    wb[i..i + bs].copy_from_slice(&buf()[..bs]);
                }
                WB_N += 1;
            }
            let i = (b - WB_AT) as usize * bs + off;
            wb[i..i + 512].copy_from_slice(data);
            WSTATS.0 += 1;
            if last || (off + 512 == bs && WB_N >= cap) {
                ok = flush() && ok;
            }
        }
        ok && got
    }
}

/// Read-only view for `lsparts`: the i-th disk found (port, 512-byte sectors, sector bytes).
pub fn raw_disk(i: usize) -> Option<(u32, u64, u64)> {
    // SAFETY: scalar reads of boot-time state.
    unsafe {
        if i >= N_FOUND {
            return None;
        }
        let (p, s, _) = FOUND[i];
        Some((p, s, LBS[p as usize & 31] as u64))
    }
}

/// Read one 512-byte sector of the disk on port `p` whoever owns it (never writes).
pub fn raw_read(p: u32, n: u64, out: &mut [u8; 512]) -> bool {
    // the storage disk: gathered writes first, then through the read-ahead
    // SAFETY: scalar read.
    let ours = unsafe { DISK_PORT } == Some(p);
    if !(if ours { flush() && fetch(p, n / per(p)) } else { command(p, ATA_READ_DMA_EXT, n / per(p), false) }) {
        return false;
    }
    let off = (n % per(p)) as usize * 512;
    out.copy_from_slice(&buf()[off..off + 512]);
    true
}

/// Write one 512-byte sector of the disk on port `p` whoever owns it. ONLY installpart/tablerestore/bootcopy call
/// this (gpt.rs), and only after a matching table backup on a USB stick and the owner's code.
pub fn table_write(p: u32, n: u64, data: &[u8; 512]) -> bool {
    let Some(cap) = (0..32).filter_map(raw_disk).find(|d| d.0 == p).map(|d| d.1) else { return false };
    // SAFETY: scalar read.
    let ours = unsafe { DISK_PORT } == Some(p);
    if n >= cap || ours && !flush() || per(p) > 1 && !(if ours { fetch(p, n / per(p)) } else { command(p, ATA_READ_DMA_EXT, n / per(p), false) }) {
        return false;
    }
    let off = (n % per(p)) as usize * 512;
    buf()[off..off + 512].copy_from_slice(data);
    let ok = command(p, ATA_WRITE_DMA_EXT, n / per(p), true);
    if ours {
        // SAFETY: scalar read.
        let bs = unsafe { LBS[p as usize & 31] } as usize;
        ra_update(n / per(p), &buf()[..bs], ok);
    }
    ok
}

/// `lsahci`: the controller and every disk on it.
pub fn list() {
    // SAFETY: scalar reads of boot-time state.
    let (abar, (b, d, f), ports, vs, n) = unsafe { (ABAR, PCI_AT, PORTS, VERSION, N_FOUND) };
    if abar == 0 {
        crate::kprintln!("ahci: no AHCI controller");
        return;
    }
    crate::kprintln!(
        "ahci: controller {:02x}:{:02x}.{} ABAR {:#x}, AHCI {}.{}, ports implemented {:#x}",
        b, d, f, abar, vs >> 16, (vs >> 8) & 0xFF, ports
    );
    for i in 0..n {
        // SAFETY: as above.
        let (p, s, usable) = unsafe { FOUND[i] };
        crate::kprintln!(
            "  port {}: {} sectors ({} MiB){}",
            p,
            s,
            s / 2048,
            if usable || unsafe { DISK_PORT } == Some(p) {
                "  <- storage"
            } else if unsafe { *addr_of!(BLANK) }.is_some_and(|b| b.0 == p) {
                "  (blank: claimdisk sata to use it)"
            } else {
                "  (other data, read-only to us)"
            }
        );
    }
    if present() {
        // SAFETY: as above.
        let m = unsafe { &*addr_of!(MODEL) };
        let model = core::str::from_utf8(m).unwrap_or("?").trim();
        crate::kprintln!("  storage disk model: {}", model);
        // SAFETY: scalar reads.
        let (r, w, mr, mw) = unsafe { (RSTATS, WSTATS, MULTI, MULTIW) };
        let word = |s: u8| match s { 1 => "on", 2 => "off", _ => "not checked yet" };
        crate::kprintln!("  reads: {} sector(s), {} from the read-ahead, {} multi-block command(s), {} single ({})", r.0, r.1, r.2, r.3, word(mr));
        crate::kprintln!("  writes: {} sector(s), {} multi-block command(s), {} single ({})", w.0, w.1, w.2, word(mw));
        if let Some((p, a, l)) = part().filter(|x| x.2 > 0) {
            crate::kprintln!("  storage: port {}, AfriSovera's partition, sectors {}-{} ({} MiB); nothing outside it is written", p, a, a + l - 1, l / 2048);
        }
    }
}
