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

use crate::mmu;
use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

// --- PCI configuration space (mechanism #1, ports 0xCF8/0xCFC) ---
unsafe fn outl(port: u16, v: u32) {
    asm!("out dx, eax", in("dx") port, in("eax") v, options(nomem, nostack, preserves_flags));
}
unsafe fn inl(port: u16) -> u32 {
    let v: u32;
    asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags));
    v
}
fn cfg_addr(b: u8, d: u8, f: u8, off: u8) -> u32 {
    0x8000_0000 | (b as u32) << 16 | (d as u32) << 11 | (f as u32) << 8 | (off as u32 & 0xFC)
}
fn cfg32(b: u8, d: u8, f: u8, off: u8) -> u32 {
    // SAFETY: the PCI configuration ports; single core with interrupts masked at init.
    unsafe {
        outl(0xCF8, cfg_addr(b, d, f, off));
        inl(0xCFC)
    }
}
fn cfg_write32(b: u8, d: u8, f: u8, off: u8, v: u32) {
    // SAFETY: as cfg32.
    unsafe {
        outl(0xCF8, cfg_addr(b, d, f, off));
        outl(0xCFC, v);
    }
}

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

/// One ATA command through slot 0 with one 512-byte PRD over BUF. Returns success.
fn command(p: u32, ata: u8, lba: u64, write: bool) -> bool {
    if !wait(|| port(p, P_TFD) & (TFD_BSY | TFD_DRQ) == 0) {
        crate::kprintln!("[ahci] port {} busy", p);
        return false;
    }
    let buf = addr_of!(BUF) as u64;
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
        t[12] = 1; // one sector
        // PRD 0: the buffer, byte count - 1
        t[0x80..0x84].copy_from_slice(&(buf as u32).to_le_bytes());
        t[0x84..0x88].copy_from_slice(&((buf >> 32) as u32).to_le_bytes());
        // byte count - 1: IDENTIFY is always 512 bytes, data is one logical sector
        let bytes = if ata == ATA_IDENTIFY { 512 } else { LBS[p as usize & 31] };
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
    let mut at = None;
    'scan: for b in 0..8u8 {
        for d in 0..32u8 {
            for f in 0..8u8 {
                let id = cfg32(b, d, f, 0);
                if id & 0xFFFF == 0xFFFF {
                    if f == 0 {
                        break;
                    }
                    continue;
                }
                if cfg32(b, d, f, 8) >> 8 == 0x01_06_01 {
                    at = Some((b, d, f));
                    break 'scan;
                }
            }
        }
    }
    let Some((b, d, f)) = at else { return };
    let abar = (cfg32(b, d, f, 0x24) & !0xF) as u64; // BAR5
    if abar == 0 || !mmu::map_mmio(abar, 0x1100) {
        crate::kprintln!("[ahci] controller at {:02x}:{:02x}.{} has no usable ABAR", b, d, f);
        return;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory decoding + bus mastering, INTx off (polled)
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
    if hba(CAP) & 1 << 31 == 0 && [addr_of!(CLIST) as u64, addr_of!(RFIS) as u64, addr_of!(CTBL) as u64, addr_of!(BUF) as u64].iter().any(|&a| a >> 32 != 0) {
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
}

/// `claimdisk sata`: take the blank SATA disk for the VFS (the owner asked for it).
pub fn claim() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if let (None, Some((p, s))) = (DISK_PORT, *addr_of!(BLANK)) {
            DISK_PORT = Some(p);
            DISK_SECTORS = s;
            BLANK = None;
            return true;
        }
        false
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
    // the model name for lsahci, as init keeps it for a disk used from boot
    if command(p, ATA_IDENTIFY, 0, false) {
        let id = buf();
        // SAFETY: scalar state, shell context.
        let m = unsafe { &mut *addr_of_mut!(MODEL) };
        for i in 0..20 {
            m[i * 2] = id[54 + i * 2 + 1];
            m[i * 2 + 1] = id[54 + i * 2];
        }
    }
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
    if !command(p, ATA_READ_DMA_EXT, n / per(p), false) {
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
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // smaller than the drive's sector: read it, change our 512 bytes, write it back
    if per(p) > 1 && !command(p, ATA_READ_DMA_EXT, n / per(p), false) {
        return false;
    }
    let off = (n % per(p)) as usize * 512;
    buf()[off..off + 512].copy_from_slice(data);
    command(p, ATA_WRITE_DMA_EXT, n / per(p), true)
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
    if !command(p, ATA_READ_DMA_EXT, n / per(p), false) {
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
    if n >= cap || per(p) > 1 && !command(p, ATA_READ_DMA_EXT, n / per(p), false) {
        return false;
    }
    let off = (n % per(p)) as usize * 512;
    buf()[off..off + 512].copy_from_slice(data);
    command(p, ATA_WRITE_DMA_EXT, n / per(p), true)
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
        if let Some((p, a, l)) = part().filter(|x| x.2 > 0) {
            crate::kprintln!("  storage: port {}, AfriSovera's partition, sectors {}-{} ({} MiB); nothing outside it is written", p, a, a + l - 1, l / 2048);
        }
    }
}
