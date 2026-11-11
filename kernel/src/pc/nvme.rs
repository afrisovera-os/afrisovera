//! NVMe disks on the PC: roadmap PC step 3, storage (5 Nov 2026).
//!
//! The disk in most PCs sold today. PCI class 01:08:02, registers in BAR0. One admin queue
//! pair, one I/O queue pair, polled, one 512-byte block per command: the same
//! `read_sector` / `write_sector` shape as the AHCI and virtio-blk paths, so the shared VFS
//! needs no change (pc/disk.rs picks the disk). Same safety rule as AHCI: the namespace is
//! used for storage only if its sector 0 is blank or already AFRIVFS1.
//! NVMe base specification 1.4: registers 3.1, queues 4, admin commands 5, I/O commands 6.
//! Multi-block commands (10 Nov 2026), as the USB disks got on 9-10 Nov: a read fetches up to
//! 8 KiB (two 4 KiB pages: PRP1 + PRP2, NLB = k - 1) into a read-ahead the next reads are
//! served from, and the VFS's in-order save is gathered by device block (on a 4 KiB-block SSD
//! each block read once, its eight sectors changed in place) into runs of up to 8 KiB sent with
//! one Write. Each way is checked once against one block at a time (reads at start-up, writes
//! at the first save, read back) before it is trusted; a mismatch or a refusal turns it off.

use crate::mmu;
use crate::pci;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

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
/// Multi-block commands: the read-ahead (RA_N device blocks from RA_AT) and the gathered writes
/// (WB_N device blocks from WB_AT), 8 KiB each, 8 KiB aligned so each is exactly two pages.
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
/// Largest transfer the controller takes (Identify Controller MDTS; 0 there = no limit), bytes.
static mut MAXB: u64 = RA_BYTES as u64;
/// Reads: sectors asked for, served from the read-ahead, multi-block commands, single-block ones.
static mut RSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Writes: sectors written, multi-block commands, single-block ones.
static mut WSTATS: (u64, u64, u64) = (0, 0, 0);
/// Big copies (table_write_run): 0 = not tried, 1 = on, 2 = off (one sector at a time, as table_write).
static mut TRUN: u8 = 0;
/// Big copies: runs sent in one command, sectors in them.
static mut TSTATS: (u64, u64) = (0, 0);

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
    let Some((b, d, f)) = pci::find_class(0x01_08_02, 8) else { return };
    let bar = pci::bar_mem(b, d, f, 0);
    if bar == 0 || !mmu::map_mmio(bar, 0x2000) {
        crate::kprintln!("[nvme] controller at {:02x}:{:02x}.{} has no usable BAR0", b, d, f);
        return;
    }
    pci::enable(b, d, f, 0x406); // memory decoding + bus mastering, INTx off (polled)
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
    // MDTS (byte 77): the largest transfer, in units of the 4 KiB page as a power of two
    let mdts = data()[77] as u32;
    // SAFETY: as above.
    unsafe { MAXB = if mdts == 0 || mdts > 20 { RA_BYTES as u64 } else { (4096u64 << mdts).min(RA_BYTES as u64) } };
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
    if matches!(lba_bytes, 512 | 1024 | 2048 | 4096) {
        multi_check();
    }
}

/// Device blocks per multi-block command: 8 KiB (or the controller's MDTS) over the block size.
fn kmax() -> u32 {
    // SAFETY: scalar reads of boot-time state.
    unsafe { (MAXB / LBA_BYTES.max(1)) as u32 }
}

/// At start-up: one Read of the first k blocks must equal the same blocks read one at a time,
/// else multi-block reads stay off (and writes with them). Reads only: safe on any disk. One
/// boot line says which.
fn multi_check() {
    // SAFETY: boot, single core; no command in flight.
    unsafe {
        let k = (kmax() as u64).min(SECTORS) as u32;
        if k < 2 {
            (MULTI, MULTIW) = (2, 2);
            return;
        }
        let ra = addr_of_mut!(RA.0) as *mut u8;
        core::ptr::write_bytes(ra, 0xA5, RA_BYTES);
        let bs = LBA_BYTES as usize;
        let read_ok = rw(OPC_READ, 0, k, ra as u64);
        let mut same = read_ok;
        for b in 0..k as u64 {
            if !same {
                break;
            }
            same = io(OPC_READ, b) && data()[..bs] == core::slice::from_raw_parts(ra.add(b as usize * bs), bs)[..];
        }
        if same {
            (MULTI, RA_AT, RA_N) = (1, 0, k);
            crate::kprintln!("[nvme] multi-block reads on (up to {} blocks per command; the first {} equal one at a time)", k, k);
        } else {
            (MULTI, MULTIW, RA_N) = (2, 2, 0);
            crate::kprintln!(
                "[nvme] multi-block reads off ({}): one block per command",
                if read_ok { "they differed from one block at a time" } else { "the controller refused one" }
            );
        }
    }
}

/// One Read or Write of `k` device blocks from `lba` on namespace 1 through `buf` (RA or WB:
/// 8 KiB aligned, so a transfer over one page takes PRP2 = the second page, no PRP list).
fn rw(opc: u8, lba: u64, k: u32, buf: u64) -> bool {
    // SAFETY: scalar reads.
    if !unsafe { READY } || k == 0 || k > kmax() {
        return false;
    }
    let mut c = cmd(opc, 1, buf);
    // SAFETY: scalar read.
    if k as u64 * unsafe { LBA_BYTES } > 4096 {
        let p2 = buf + 4096;
        c[8] = p2 as u32;
        c[9] = (p2 >> 32) as u32;
    }
    c[10] = lba as u32;
    c[11] = (lba >> 32) as u32;
    c[12] = k - 1; // NLB is zero-based
    match submit(1, c) {
        Some(0) => true,
        Some(s) => {
            crate::kprintln!("[nvme] {} of {} blocks at {} refused (status {:#x})", if opc == OPC_READ { "read" } else { "write" }, k, lba, s);
            false
        }
        None => false,
    }
}

/// Device block `b` into DATA: from the read-ahead when it holds it, else a new multi-block read
/// of up to 8 KiB from it (once the boot check passed), else one block.
fn fetch(b: u64) -> bool {
    // SAFETY: single core, kernel context, one command in flight.
    unsafe {
        RSTATS.0 += 1;
        let bs = LBA_BYTES as usize;
        if b >= RA_AT && b < RA_AT + RA_N as u64 {
            let off = (b - RA_AT) as usize * bs;
            data()[..bs].copy_from_slice(&(&*addr_of!(RA.0))[off..off + bs]);
            RSTATS.1 += 1;
            return true;
        }
        if MULTI == 1 {
            let k = (kmax() as u64).min(SECTORS.saturating_sub(b)) as u32;
            if k > 1 {
                RA_N = 0;
                if rw(OPC_READ, b, k, addr_of!(RA.0) as u64) {
                    (RA_AT, RA_N) = (b, k);
                    data()[..bs].copy_from_slice(&(&*addr_of!(RA.0))[..bs]);
                    RSTATS.2 += 1;
                    return true;
                }
                MULTI = 2;
                crate::kprintln!("[nvme] a multi-block read at block {} was refused: one block per command from now on", b);
            }
        }
        RSTATS.3 += 1;
        io(OPC_READ, b)
    }
}

/// Device block `b` was written from `src` (or the write failed): keep the read-ahead true.
fn ra_update(b: u64, src: &[u8], ok: bool) {
    // SAFETY: as fetch.
    unsafe {
        if b >= RA_AT && b < RA_AT + RA_N as u64 {
            if ok {
                let off = (b - RA_AT) as usize * LBA_BYTES as usize;
                (&mut (*addr_of_mut!(RA)).0)[off..off + src.len()].copy_from_slice(src);
            } else {
                RA_N = 0; // the disk's copy is unknown now
            }
        }
    }
}

/// Send the gathered writes: one Write when multi-block writes are on (the first one is read back
/// block by block and must be equal, else they go off and the run is rewritten one block at a
/// time), else one Write per block.
fn flush() -> bool {
    // SAFETY: as fetch.
    unsafe {
        let (at, k) = (WB_AT, WB_N);
        if k == 0 {
            return true;
        }
        WB_N = 0;
        let wb = addr_of!(WB.0) as *const u8;
        let bs = LBA_BYTES as usize;
        let mut done = false;
        if k > 1 && MULTI == 1 && MULTIW != 2 {
            if rw(OPC_WRITE, at, k, wb as u64) {
                WSTATS.1 += 1;
                done = true;
                if MULTIW == 0 {
                    for j in 0..k as u64 {
                        let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
                        if !(io(OPC_READ, at + j) && data()[..bs] == s[..]) {
                            done = false;
                            break;
                        }
                    }
                    MULTIW = if done { 1 } else { 2 };
                    if done {
                        crate::kprintln!("[nvme] multi-block writes on (up to {} blocks per command; the first {} read back equal)", kmax(), k);
                    } else {
                        crate::kprintln!("[nvme] multi-block writes off (read back differently: rewritten one block per command)");
                    }
                }
            } else {
                MULTIW = 2;
                crate::kprintln!("[nvme] a multi-block write at block {} was refused: one block per command from now on", at);
            }
        }
        let mut ok = true;
        for j in 0..k as u64 {
            let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
            let good = done || {
                data()[..bs].copy_from_slice(s);
                WSTATS.2 += 1;
                io(OPC_WRITE, at + j)
            };
            ra_update(at + j, s, good);
            ok = ok && good;
        }
        ok
    }
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
    // writes still gathered go to the disk before anything is read
    if !(flush() && fetch(n / per())) {
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
    // the VFS saves its sectors in order, so a run of device blocks is gathered in WB (up to
    // 8 KiB; one block while multi-block writes are off). A block bigger than a sector (1, 2 or
    // 4 KiB) is read once when the run reaches it and its sectors changed in place, so a 4 KiB
    // SSD is written once per block, not once per sector. The run is sent when it is full and its
    // last block complete, on a gap, before any read, or at the VFS's last sector (or a lone
    // write past the VFS area: the self-test).
    let last = n >= 2 * crate::vfs::MAX_FILES as u64;
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // SAFETY: single core, kernel context, one command in flight.
    unsafe {
        let (per, bs) = (per(), LBA_BYTES as usize);
        let (b, off) = (n / per, (n % per) as usize * 512);
        let cap = if MULTI == 1 && MULTIW != 2 { kmax() } else { 1 };
        let mut ok = true;
        let tail = WB_AT + WB_N as u64; // the block after the run
        if WB_N > 0 && b + 1 != tail && !(b == tail && WB_N < cap) {
            ok = flush();
        }
        let new = WB_N == 0 || b == WB_AT + WB_N as u64;
        // a new block's other sectors as they are on the disk (fetch: the read-ahead, kept equal
        // to every block flushed, or the disk)
        let got = !new || per == 1 || fetch(b);
        if got {
            let wb = &mut (*addr_of_mut!(WB)).0;
            if WB_N == 0 {
                WB_AT = b;
            }
            if new {
                if per > 1 {
                    let i = WB_N as usize * bs;
                    wb[i..i + bs].copy_from_slice(&data()[..bs]);
                }
                WB_N += 1;
            }
            let i = (b - WB_AT) as usize * bs + off;
            wb[i..i + 512].copy_from_slice(buf);
            WSTATS.0 += 1;
            if last || (off + 512 == bs && WB_N >= cap) {
                ok = flush() && ok;
            }
        }
        ok && got
    }
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
    if n >= cap || !(flush() && fetch(n / per())) {
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
    if n >= cap || !flush() || (per() > 1 && !fetch(n / per())) {
        return false;
    }
    let off = (n % per()) as usize * 512;
    data()[off..off + 512].copy_from_slice(buf);
    let ok = io(OPC_WRITE, n / per());
    // SAFETY: scalar read.
    let bs = unsafe { LBA_BYTES } as usize;
    ra_update(n / per(), &data()[..bs], ok);
    ok
}

/// Write `data` (1 to 16 whole 512-byte sectors) from sector `n` of namespace 1 whoever owns it: ONE
/// Write, then ONE Read and compared, so installdisk and bootcopy do not send a command for every sector
/// (11 Nov 2026, as ahci.rs). WB carries the run and RA the read back (gathered writes go out first and
/// the read-ahead is dropped). A run refused or read back different is written again one sector at a
/// time (table_write) and runs stay off until power-off, with one line. One sector at a time also while
/// multi-block reads are off. A namespace with blocks bigger than 512 bytes (4 KiB on many laptops) gets
/// whole device blocks (11 Nov 2026): a block the run only partly covers is read first and the run laid
/// into it, and a run wider than one command takes (8 KiB, or less if the controller says so) goes out
/// as several commands, each one a run. The same callers and the same rule as table_write.
pub fn table_write_run(n: u64, data: &[u8]) -> bool {
    let c = data.len() / 512;
    if c == 0 || c * 512 != data.len() || c * 512 > RA_BYTES {
        return false;
    }
    let Some((cap, _)) = raw_info() else { return false };
    let single = || data.chunks_exact(512).enumerate().all(|(j, s)| table_write(n + j as u64, s.try_into().unwrap()));
    // SAFETY: single core, kernel/shell context, one command in flight; WB and RA are ours while no
    // write is gathered (flush) and the read-ahead is dropped.
    unsafe {
        if c == 1 || TRUN == 2 || MULTI != 1 {
            return single();
        }
        if n.saturating_add(c as u64) > cap || !flush() {
            return false;
        }
        RA_N = 0;
        let (per, bs) = (per(), LBA_BYTES as usize);
        let kk = (kmax() as u64).min((RA_BYTES / bs.max(1)) as u64); // device blocks per command
        if kk == 0 {
            return single();
        }
        let (wb, rb) = (addr_of_mut!(WB.0) as *mut u8, addr_of_mut!(RA.0) as *mut u8);
        let mut i = 0usize; // sectors of the run sent so far
        while i < c {
            let s = n + i as u64;
            let b0 = s / per;
            let end = ((b0 + kk) * per).min(n + c as u64); // the sector after this command's part
            let k = ((end - 1) / per - b0 + 1) as u32;
            let (len, head, take) = (k as usize * bs, (s - b0 * per) as usize * 512, (end - s) as usize * 512);
            // a device block only partly in the run: what the disk holds now around it
            let whole = head == 0 && take == len;
            let ok = (whole || rw(OPC_READ, b0, k, wb as u64)) && {
                core::slice::from_raw_parts_mut(wb.add(head), take).copy_from_slice(&data[i * 512..i * 512 + take]);
                rw(OPC_WRITE, b0, k, wb as u64)
                    && rw(OPC_READ, b0, k, rb as u64)
                    && core::slice::from_raw_parts(rb, len) == core::slice::from_raw_parts(wb as *const u8, len)
            };
            if !ok {
                crate::kprintln!(
                    "[nvme] {} sectors written in one command were refused or read back different, so big copies go one sector at a time from now on",
                    take / 512
                );
                TRUN = 2;
                return single();
            }
            TRUN = 1;
            TSTATS.0 += 1;
            TSTATS.1 += (take / 512) as u64;
            i += take / 512;
        }
        true
    }
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
        // SAFETY: scalar reads.
        let (r, w, mr, mw) = unsafe { (RSTATS, WSTATS, MULTI, MULTIW) };
        let word = |s: u8| match s { 1 => "on", 2 => "off", _ => "not checked yet" };
        crate::kprintln!("  reads: {} sector(s), {} from the read-ahead, {} multi-block command(s), {} single ({})", r.0, r.1, r.2, r.3, word(mr));
        crate::kprintln!("  writes: {} sector(s), {} multi-block command(s), {} single ({})", w.0, w.1, w.2, word(mw));
        // SAFETY: scalar reads.
        let (tr, t) = unsafe { (TRUN, TSTATS) };
        if tr != 0 {
            crate::kprintln!(
                "  big copies (installdisk, bootcopy): {} run(s) in one command, {} sector(s), each read back ({})",
                t.0, t.1, if tr == 1 { "on" } else { "off" }
            );
        }
    }
}
