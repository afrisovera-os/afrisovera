//! NVMe disks on the PC: roadmap PC step 3, storage (5 Nov 2026).
//!
//! The disk in most PCs sold today. PCI class 01:08:02, registers in BAR0. One admin queue
//! pair, one I/O queue pair, polled, one 512-byte block per command: the same
//! `read_sector` / `write_sector` shape as the AHCI and virtio-blk paths, so the shared VFS
//! needs no change (pc/disk.rs picks the disk). Same safety rule as AHCI: the namespace is
//! used for storage only if its sector 0 is blank or already AFRIVFS1.
//! NVMe base specification 1.4: registers 3.1, queues 4, admin commands 5, I/O commands 6.

use crate::mmu;
use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

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
    // SAFETY: the PCI configuration ports; boot context, single core.
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

const R_CAP: u64 = 0x00;
const R_VS: u64 = 0x08;
const R_CC: u64 = 0x14;
const R_CSTS: u64 = 0x1C;
const R_AQA: u64 = 0x24;
const R_ASQ: u64 = 0x28;
const R_ACQ: u64 = 0x30;

const QN: usize = 16; // entries per queue (admin and I/O)
const OPC_IDENTIFY: u8 = 0x06;
const OPC_CREATE_SQ: u8 = 0x01;
const OPC_CREATE_CQ: u8 = 0x05;
const OPC_WRITE: u8 = 0x01;
const OPC_READ: u8 = 0x02;

#[repr(C, align(4096))]
struct Sq([[u32; 16]; QN]);
#[repr(C, align(4096))]
struct Cq([[u32; 4]; QN]);
#[repr(C, align(4096))]
struct Page([u8; 4096]);

static mut ASQ: Sq = Sq([[0; 16]; QN]);
static mut ACQ: Cq = Cq([[0; 4]; QN]);
static mut IOSQ: Sq = Sq([[0; 16]; QN]);
static mut IOCQ: Cq = Cq([[0; 4]; QN]);
/// Identify data, and the 512-byte block buffer of every read/write (its first 512 bytes).
static mut DATA: Page = Page([0; 4096]);

/// One queue pair's driver-side state: SQ tail, CQ head, the phase we expect, the next id.
#[derive(Clone, Copy)]
struct Pair {
    tail: usize,
    head: usize,
    phase: u32,
    cid: u16,
}
static mut ADMIN: Pair = Pair { tail: 0, head: 0, phase: 1, cid: 0 };
static mut IO: Pair = Pair { tail: 0, head: 0, phase: 1, cid: 0 };

static mut BAR: u64 = 0;
static mut DSTRD: u64 = 0;
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut READY: bool = false; // controller up, I/O queues created
static mut USABLE: bool = false; // namespace 1 may hold the VFS
/// Namespace 1 is blank (sector 0 all zero): usable only after `claimdisk nvme`.
static mut BLANK: bool = false;
static mut SECTORS: u64 = 0;
/// AfriSovera's own GPT partition holding the VFS (first 512-byte sector, sectors); (0, 0) =
/// the whole namespace. Set at boot when the partition already holds AFRIVFS1, or by
/// `formatpart nvme`; sector n of the VFS is then sector PART.0 + n of the disk.
static mut PART: (u64, u64) = (0, 0);
static mut LBA_BYTES: u64 = 0;
static mut MODEL: [u8; 40] = [b' '; 40];

fn r32(off: u64) -> u32 {
    // SAFETY: BAR0 is mapped uncached by init before any register access.
    unsafe { read_volatile((BAR + off) as *const u32) }
}
fn w32(off: u64, v: u32) {
    // SAFETY: as r32.
    unsafe { write_volatile((BAR + off) as *mut u32, v) }
}
fn r64(off: u64) -> u64 {
    r32(off) as u64 | (r32(off + 4) as u64) << 32
}
fn w64(off: u64, v: u64) {
    w32(off, v as u32);
    w32(off + 4, (v >> 32) as u32);
}
fn doorbell(q: u64, completion: bool) -> u64 {
    // SAFETY: scalar read of boot-time state.
    0x1000 + (2 * q + completion as u64) * (4 << unsafe { DSTRD })
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

/// Submit one command on queue `q` (0 = admin, 1 = I/O) and poll for its completion.
/// `cmd[0]` gets the opcode and command id; returns the completion status (0 = success).
fn submit(q: u64, mut cmd: [u32; 16]) -> Option<u16> {
    // SAFETY: single core, kernel context, one command in flight per queue; the device reads
    // the entry only after the doorbell write, and writes the completion we then read.
    unsafe {
        let (pair, sq, cq) = if q == 0 {
            (&mut *addr_of_mut!(ADMIN), addr_of_mut!(ASQ.0) as *mut [u32; 16], addr_of!(ACQ.0) as *const [u32; 4])
        } else {
            (&mut *addr_of_mut!(IO), addr_of_mut!(IOSQ.0) as *mut [u32; 16], addr_of!(IOCQ.0) as *const [u32; 4])
        };
        let cid = pair.cid;
        pair.cid = pair.cid.wrapping_add(1);
        cmd[0] |= (cid as u32) << 16;
        write_volatile(sq.add(pair.tail), cmd);
        pair.tail = (pair.tail + 1) % QN;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w32(doorbell(q, false), pair.tail as u32);
        let slot = cq.add(pair.head);
        let phase = pair.phase;
        if !wait(|| (read_volatile(slot)[3] >> 16) & 1 == phase) {
            crate::kprintln!("[nvme] queue {} command {:#x} timed out", q, cmd[0] & 0xFF);
            return None;
        }
        let e = read_volatile(slot);
        pair.head += 1;
        if pair.head == QN {
            pair.head = 0;
            pair.phase ^= 1;
        }
        w32(doorbell(q, true), pair.head as u32);
        if e[3] & 0xFFFF != cid as u32 {
            crate::kprintln!("[nvme] completion for id {} while waiting for {}", e[3] & 0xFFFF, cid);
        }
        Some(((e[3] >> 17) & 0x7FFF) as u16)
    }
}

fn data() -> &'static mut [u8; 4096] {
    // SAFETY: single core, kernel context, one command in flight.
    unsafe { &mut (*addr_of_mut!(DATA)).0 }
}

fn cmd(opc: u8, nsid: u32, prp1: u64) -> [u32; 16] {
    let mut c = [0u32; 16];
    c[0] = opc as u32;
    c[1] = nsid;
    c[6] = prp1 as u32;
    c[7] = (prp1 >> 32) as u32;
    c
}

/// Find the controller, bring it up with one I/O queue pair, and size namespace 1.
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
                if cfg32(b, d, f, 8) >> 8 == 0x01_08_02 {
                    at = Some((b, d, f));
                    break 'scan;
                }
            }
        }
    }
    let Some((b, d, f)) = at else { return };
    let lo = cfg32(b, d, f, 0x10);
    let bar = (lo & !0xF) as u64 | if lo & 0x6 == 0x4 { (cfg32(b, d, f, 0x14) as u64) << 32 } else { 0 };
    if bar == 0 || !mmu::map_mmio(bar, 0x2000) {
        crate::kprintln!("[nvme] controller at {:02x}:{:02x}.{} has no usable BAR0", b, d, f);
        return;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory decoding + bus mastering, INTx off (polled)
    // SAFETY: boot, single core, before any reader.
    unsafe {
        BAR = bar;
        PCI_AT = (b, d, f);
    }
    let cap = r64(R_CAP);
    // SAFETY: as above.
    unsafe { DSTRD = (cap >> 32) & 0xF };
    if (cap >> 48) & 0xF != 0 {
        crate::kprintln!("[nvme] controller's smallest page is {} KiB, not 4: not used", 4u64 << ((cap >> 48) & 0xF));
        return;
    }
    // CAP.TO: how long the controller may take to (un)set ready, in 500 ms units (real SSDs
    // can take seconds after a warm reboot)
    let to_ms = ((cap >> 24) & 0xFF).max(1) * 500;
    let ready_wait = |want: u32| {
        let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
        loop {
            let st = r32(R_CSTS);
            if st & 1 == want || st & 2 != 0 {
                return st & 1 == want && st & 2 == 0;
            }
            if (crate::timer::cycles() - t0) / (hz / 1000).max(1) > to_ms {
                return false;
            }
            core::hint::spin_loop();
        }
    };
    // fresh queues: the completion phase bits must start at 0
    // SAFETY: the controller is not using them (it is being reset).
    unsafe {
        core::ptr::write_bytes(addr_of_mut!(ACQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(IOCQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(ASQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(IOSQ), 0, 1);
        *addr_of_mut!(ADMIN) = Pair { tail: 0, head: 0, phase: 1, cid: 0 };
        *addr_of_mut!(IO) = Pair { tail: 0, head: 0, phase: 1, cid: 0 };
    }
    // Reset: disable, wait for not-ready, then point the admin queues and enable.
    w32(R_CC, 0);
    if !ready_wait(0) {
        crate::kprintln!("[nvme] controller did not stop");
        return;
    }
    w32(R_AQA, ((QN as u32 - 1) << 16) | (QN as u32 - 1));
    w64(R_ASQ, addr_of!(ASQ) as u64);
    w64(R_ACQ, addr_of!(ACQ) as u64);
    w32(R_CC, 1 | 6 << 16 | 4 << 20); // EN, NVM command set, 4 KiB pages, 64 B SQE, 16 B CQE
    if !ready_wait(1) {
        crate::kprintln!("[nvme] controller did not become ready (CSTS {:#x})", r32(R_CSTS));
        return;
    }
    let buf = addr_of!(DATA) as u64;
    // Identify controller (CNS 1): the model number at bytes 24..64.
    let mut c = cmd(OPC_IDENTIFY, 0, buf);
    c[10] = 1;
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] identify controller failed");
        return;
    }
    let mut model = [b' '; 40];
    model.copy_from_slice(&data()[24..64]);
    // Identify namespace 1 (CNS 0): size in blocks, and the block size of its format.
    if submit(0, cmd(OPC_IDENTIFY, 1, buf)) != Some(0) {
        crate::kprintln!("[nvme] identify namespace 1 failed");
        return;
    }
    let id = data();
    let nsze = u64::from_le_bytes(id[0..8].try_into().unwrap_or([0; 8]));
    let fmt = (id[26] & 0xF) as usize;
    let lbads = id[128 + fmt * 4 + 2];
    let lba_bytes = 1u64 << lbads;
    // The I/O queue pair: completion queue 1, then submission queue 1 feeding it.
    let mut c = cmd(OPC_CREATE_CQ, 0, addr_of!(IOCQ) as u64);
    c[10] = ((QN as u32 - 1) << 16) | 1;
    c[11] = 1; // physically contiguous, no interrupts (polled)
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] create I/O completion queue failed");
        return;
    }
    let mut c = cmd(OPC_CREATE_SQ, 0, addr_of!(IOSQ) as u64);
    c[10] = ((QN as u32 - 1) << 16) | 1;
    c[11] = 1 << 16 | 1; // completion queue 1, physically contiguous
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] create I/O submission queue failed");
        return;
    }
    // SAFETY: as above.
    unsafe {
        READY = true;
        SECTORS = nsze;
        LBA_BYTES = lba_bytes;
        MODEL = model;
    }
    let vs = r32(R_VS);
    // 512-byte sectors are what the VFS speaks; 1, 2 and 4 KiB blocks (4 KiB is common on real
    // SSDs) hold several of them -- read_sector / write_sector map onto the block (see `per`)
    let usable = if !matches!(lba_bytes, 512 | 1024 | 2048 | 4096) {
        crate::kprintln!("[nvme] namespace 1 uses {}-byte blocks -- not used for storage", lba_bytes);
        false
    } else {
        // an internal disk is used on its own only if it is already ours; a blank one waits
        // for the owner to claim it (`claimdisk nvme`), so a new, empty SSD is never taken
        // silently
        let mut s0 = [0u8; 512];
        let read = io(OPC_READ, 0);
        if read {
            s0.copy_from_slice(&data()[..512]);
        }
        // SAFETY: as above.
        unsafe { BLANK = read && s0.iter().all(|&x| x == 0) };
        // a disk with another system on it: used only through AfriSovera's own partition
        // (made by installpart, formatted by formatpart), never outside it
        read && (&s0[..8] == b"AFRIVFS1" || (!unsafe { BLANK } && find_part()))
    };
    // SAFETY: as above.
    unsafe { USABLE = usable };
    crate::kprintln!(
        "[nvme] namespace 1: {} blocks of {} B ({} MiB), {}",
        nsze,
        lba_bytes,
        nsze * lba_bytes / (1024 * 1024),
        if usable && unsafe { PART.1 } > 0 {
            "AfriSovera's partition (AFRIVFS1): used for storage, the rest left alone"
        } else if usable {
            "ours (AFRIVFS1): used for storage"
        } else if unsafe { BLANK } {
            "blank: not used unless claimed (claimdisk nvme)"
        } else {
            "holds other data: left alone"
        }
    );
    crate::kprintln!(
        "[nvme] controller {:02x}:{:02x}.{} (NVMe {}.{}), I/O queue pair ready",
        b, d, f, vs >> 16, (vs >> 8) & 0xFF
    );
}

/// One block read or write on namespace 1 through the I/O queue, over DATA's first 512 B.
fn io(opc: u8, lba: u64) -> bool {
    // SAFETY: scalar read.
    if !unsafe { READY } {
        return false;
    }
    let mut c = cmd(opc, 1, addr_of!(DATA) as u64);
    c[10] = lba as u32;
    c[11] = (lba >> 32) as u32;
    c[12] = 0; // one block
    match submit(1, c) {
        Some(0) => true,
        Some(s) => {
            crate::kprintln!("[nvme] {} of block {} failed (status {:#x})", if opc == OPC_READ { "read" } else { "write" }, lba, s);
            false
        }
        None => false,
    }
}

/// `claimdisk nvme`: take the blank namespace for the VFS (the owner asked for it).
pub fn claim() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if READY && BLANK && !USABLE {
            USABLE = true;
            BLANK = false;
            return true;
        }
        false
    }
}

/// At boot: AfriSovera's partition in a healthy GPT, already formatted -> use it.
fn find_part() -> bool {
    let Some((a, l)) = crate::gpt::ours_nvme() else { return false };
    let mut p0 = [0u8; 512];
    if raw_read(a, &mut p0) && &p0[..8] == b"AFRIVFS1" {
        // SAFETY: scalar state, boot context.
        unsafe { PART = (a, l) };
        crate::kprintln!("[nvme] AfriSovera's partition at sector {} ({} MiB) holds our files", a, l / 2048);
        return true;
    }
    crate::kprintln!("[nvme] AfriSovera's partition at sector {} ({} MiB) is not formatted yet (formatpart nvme)", a, l / 2048);
    false
}

/// `formatpart nvme`: keep the VFS in AfriSovera's partition (first sector, sectors) from now on.
pub fn use_part(a: u64, l: u64) -> bool {
    let Some((cap, _)) = raw_info() else { return false };
    // SAFETY: scalar state, shell context.
    unsafe {
        if USABLE || l == 0 || a.saturating_add(l) > cap {
            return false;
        }
        PART = (a, l);
        USABLE = true;
    }
    true
}

/// The partition in use (first sector, sectors), (0, 0) for none.
pub fn part() -> (u64, u64) {
    // SAFETY: scalar read.
    unsafe { PART }
}

/// `tablerestore nvme` took AfriSovera's partition out of the table: stop writing there.
pub fn release_part() -> bool {
    // SAFETY: scalar state, shell context.
    unsafe {
        if USABLE && PART.1 > 0 {
            USABLE = false;
            PART = (0, 0);
            return true;
        }
    }
    false
}

pub fn present() -> bool {
    // SAFETY: scalar reads.
    unsafe { READY && USABLE }
}

/// 512-byte sectors per device block (1 for 512-byte LBAs, 8 for 4 KiB ones).
fn per() -> u64 {
    // SAFETY: scalar read.
    (unsafe { LBA_BYTES } / 512).max(1)
}

/// Capacity in 512-byte sectors.
pub fn capacity() -> u64 {
    // SAFETY: scalar reads.
    if unsafe { PART.1 } > 0 { unsafe { PART.1 } } else { (unsafe { SECTORS }) * per() }
}

pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    if !present() || n >= capacity() {
        return false;
    }
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    if !io(OPC_READ, n / per()) {
        return false;
    }
    let off = (n % per()) as usize * 512;
    out.copy_from_slice(&data()[off..off + 512]);
    true
}

pub fn write_sector(n: u64, buf: &[u8; 512]) -> bool {
    if !present() || n >= capacity() {
        return false;
    }
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // a sector smaller than the block: read the block, change our 512 bytes, write it back
    if per() > 1 && !io(OPC_READ, n / per()) {
        return false;
    }
    let off = (n % per()) as usize * 512;
    data()[off..off + 512].copy_from_slice(buf);
    io(OPC_WRITE, n / per())
}

/// Read-only view for `lsparts`: (512-byte sectors, block bytes) of namespace 1 once the
/// controller is up, whether or not it holds our files.
pub fn raw_info() -> Option<(u64, u64)> {
    // SAFETY: scalar reads of boot-time state.
    unsafe { if READY && matches!(LBA_BYTES, 512 | 1024 | 2048 | 4096) { Some((SECTORS * per(), LBA_BYTES)) } else { None } }
}

/// Read one 512-byte sector of namespace 1 even when the disk is not ours (never writes).
pub fn raw_read(n: u64, out: &mut [u8; 512]) -> bool {
    let Some((cap, _)) = raw_info() else { return false };
    if n >= cap || !io(OPC_READ, n / per()) {
        return false;
    }
    let off = (n % per()) as usize * 512;
    out.copy_from_slice(&data()[off..off + 512]);
    true
}

/// Write one 512-byte sector of namespace 1 whoever owns it. ONLY installpart/tablerestore/bootcopy call this
/// (gpt.rs), and only after a matching table backup on a USB stick and the owner's typed code.
pub fn table_write(n: u64, buf: &[u8; 512]) -> bool {
    let Some((cap, _)) = raw_info() else { return false };
    if n >= cap || (per() > 1 && !io(OPC_READ, n / per())) {
        return false;
    }
    let off = (n % per()) as usize * 512;
    data()[off..off + 512].copy_from_slice(buf);
    io(OPC_WRITE, n / per())
}

/// `lsnvme`: the controller and its namespace.
pub fn list() {
    // SAFETY: scalar reads of boot-time state.
    let (bar, (b, d, f), ready, usable, n, lb, pt) = unsafe { (BAR, *addr_of!(PCI_AT), READY, USABLE, SECTORS, LBA_BYTES, PART) };
    if bar == 0 {
        crate::kprintln!("nvme: no NVMe controller");
        return;
    }
    let vs = r32(R_VS);
    crate::kprintln!("nvme: controller {:02x}:{:02x}.{} BAR0 {:#x}, NVMe {}.{}, {}", b, d, f, bar, vs >> 16, (vs >> 8) & 0xFF,
        if ready { "ready" } else { "NOT ready" });
    if ready {
        // SAFETY: as above.
        let m = unsafe { &*addr_of!(MODEL) };
        crate::kprintln!(
            "  namespace 1: {} blocks of {} B ({} MiB){}",
            n, lb, n * lb / (1024 * 1024),
            if usable { "  <- storage" } else if unsafe { BLANK } { "  (blank: claimdisk nvme to use it)" } else { "  (other data, read-only to us)" }
        );
        if usable && pt.1 > 0 {
            crate::kprintln!("  storage: AfriSovera's partition, sectors {}-{} ({} MiB); nothing outside it is written", pt.0, pt.0 + pt.1 - 1, pt.1 / 2048);
        }
        crate::kprintln!("  model: {}", core::str::from_utf8(m).unwrap_or("?").trim());
    }
}
