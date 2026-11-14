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
//! Bigger and several at once (12 Nov 2026, driver engineer, NVME_SPEED_PLAN.md, architect's Q-a to Q-c):
//! the read-ahead and the write-gather are 64 KiB; a transfer over two pages takes a PRP list (NVMe
//! 1.4 4.3, one list page per command, the first entry may start inside its page); a run longer
//! than the controller takes in one command (MDTS, in CAP.MPSMIN pages, 0 = no limit, capped at
//! 2 MiB) goes out as up to 8 commands with ONE doorbell, their completions taken in any order by
//! command id. A command that gets no answer within 5 s ends in a controller reset BEFORE its buffer
//! or list page is used again (the reset ends everything the controller held), then it is sent once
//! more; ids restart only at that reset, so a late completion can never be taken for a newer
//! command's. The last write of a VFS save is followed by an NVMe Flush when the controller has a
//! volatile write cache. `nvmetest prp|drop` are the test switches.
//! MSI-X (12 Nov 2026, slice 2): the I/O completion queue interrupts on vector 0x50 (entry 0 of the
//! controller's MSI-X table, mapped uncached, programmed with the function masked), and a command
//! waiting for its completion sleeps (`sti; hlt`, the timer bounds it) instead of spinning: less power
//! and heat, the other core not starved. NOT faster: a synchronous read still waits for the disk. A
//! batch whose completions came without any interrupt switches MSI-X off (masked, polled as before)
//! with one line; `nvmetest lose` masks the entry behind the driver's back to prove it.

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
const OPC_FLUSH: u8 = 0x00;
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
/// CAP.TO in ms: how long the controller may take to (un)set ready.
static mut TO_MS: u64 = 500;
/// Controller resets after an I/O command got no answer (shown by lsnvme once there was one).
static mut RESETS: u32 = 0;
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
/// (WB_N device blocks from WB_AT), 64 KiB each, page aligned and in the identity-mapped kernel
/// image, so a buffer's address is its DMA address (AFR-0002 4.4 rule 3).
const RA_BYTES: usize = 65536;
#[repr(C, align(4096))]
struct Ahead([u8; RA_BYTES]);
/// Commands sent with one doorbell, at most; each has its own PRP list page.
const NFLY: usize = 8;
#[repr(C, align(4096))]
struct PrpList([u64; 512]);
static mut PRPL: [PrpList; NFLY] = [const { PrpList([0; 512]) }; NFLY];
/// The most one command moves when the controller sets no limit (MDTS 0).
const MAX_XFER: u64 = 2 << 20;
/// The controller has a volatile write cache (Identify Controller VWC bit 0): a save ends with a Flush.
static mut VWC: bool = false;
/// Queue counters: most commands in flight at once, commands with a PRP list, largest transfer (bytes),
/// Flushes, late completions set aside, completions dropped by `nvmetest drop`.
static mut QSTATS: (u32, u64, u64, u64, u64, u64) = (0, 0, 0, 0, 0, 0);
/// `nvmetest drop`: completions still to be lost on purpose (each must end in a reset, then a resend).
static mut DROP: u32 = 0;
/// MSI-X: 0 = not set up (polled), 1 = on, 2 = switched off after a batch completed with no interrupt.
static mut MSIX: u8 = 0;
/// The MSI-X table (uncached), its size, why it is not used.
static mut MSIX_TAB: u64 = 0;
static mut MSIX_N: u32 = 0;
static mut MSIX_WHY: &str = "not set up yet";
/// Interrupts taken on NVME_VECTOR (counted by the handler, which runs on core 0 only).
static IRQS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Batches waited for asleep, and woken by an interrupt.
static mut WSTAT: (u64, u64) = (0, 0);
static mut RA: Ahead = Ahead([0; RA_BYTES]);
static mut RA_AT: u64 = 0;
static mut RA_N: u32 = 0;
static mut WB: Ahead = Ahead([0; RA_BYTES]);
static mut WB_AT: u64 = 0;
static mut WB_N: u32 = 0;
/// 0 = not checked, 1 = on, 2 = off; reads and writes apart.
static mut MULTI: u8 = 0;
static mut MULTIW: u8 = 0;
/// Largest transfer the controller takes in one command (Identify Controller MDTS in CAP.MPSMIN pages;
/// 0 there = no limit, then MAX_XFER), bytes.
static mut MAXB: u64 = 4096;
/// Reads: sectors asked for, served from the read-ahead, multi-block commands, single-block ones.
static mut RSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Writes: sectors written, multi-block commands, single-block ones.
static mut WSTATS: (u64, u64, u64) = (0, 0, 0);
/// Big copies (table_write_run): 0 = not tried, 1 = on, 2 = off (one sector at a time, as table_write).
static mut TRUN: u8 = 0;
/// Big copies: runs sent in one command, sectors in them.
static mut TSTATS: (u64, u64) = (0, 0);

/// NVME_VECTOR's handler (cpu.rs pc_trap): count it. The completion itself is read by the waiting command.
pub fn irq() {
    IRQS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// Find the MSI-X capability (PCI capability id 0x11), map its table uncached, and point entry 0 at this
/// core's local APIC with NVME_VECTOR, the whole function masked while it is programmed. Polled (and a
/// reason kept for lsnvme) when there is none, or when this core's APIC id does not fit an MSI address
/// (x2APIC above 255 needs interrupt remapping).
fn msix_setup(b: u8, d: u8, f: u8) {
    let why = (|| {
        if pci::cfg_read32(b, d, f, 4) >> 16 & 0x10 == 0 {
            return Err("no capability list");
        }
        let mut p = (pci::cfg_read32(b, d, f, 0x34) & 0xFC) as u8;
        let mut cap = 0u8;
        for _ in 0..48 {
            if p < 0x40 {
                break;
            }
            let h = pci::cfg_read32(b, d, f, p);
            if h & 0xFF == 0x11 {
                cap = p;
                break;
            }
            p = ((h >> 8) & 0xFC) as u8;
        }
        if cap == 0 {
            return Err("the controller has no MSI-X capability");
        }
        let apic = crate::cpu::lapic_id();
        if apic > 255 {
            return Err("this core's APIC id is above 255 (would need interrupt remapping)");
        }
        let h = pci::cfg_read32(b, d, f, cap);
        let n = ((h >> 16) & 0x7FF) + 1;
        let t = pci::cfg_read32(b, d, f, cap + 4);
        let tab = pci::bar_mem(b, d, f, (t & 7) as u8);
        if tab == 0 || t & 7 > 5 {
            return Err("its MSI-X table BAR is not usable");
        }
        let tab = tab + (t & !7) as u64;
        if !mmu::map_mmio(tab, 16 * n as u64) {
            return Err("its MSI-X table could not be mapped");
        }
        // enable with the function masked (control bits 15 and 14), program entry 0, unmask it, then the function
        pci::cfg_write32(b, d, f, cap, (h & 0x3FFF_FFFF) | 3 << 30);
        // SAFETY: entry 0 of the table just mapped uncached: address low/high, data, vector control.
        unsafe {
            let e = tab as *mut u32;
            write_volatile(e, 0xFEE0_0000 | apic << 12);
            write_volatile(e.add(1), 0);
            write_volatile(e.add(2), crate::cpu::NVME_VECTOR as u32); // fixed, edge
            write_volatile(e.add(3), 0); // unmasked
            MSIX_TAB = tab;
            MSIX_N = n;
        }
        pci::cfg_write32(b, d, f, cap, (h & 0x3FFF_FFFF) | 1 << 31);
        Ok(())
    })();
    // SAFETY: boot, single core.
    unsafe {
        match why {
            Ok(()) => {
                MSIX = 1;
                MSIX_WHY = "";
            }
            Err(w) => MSIX_WHY = w,
        }
    }
}

/// MSI-X off from now on: entry 0 masked, completions polled (the queue keeps IEN, masked is enough).
fn msix_off(why: &'static str) {
    // SAFETY: scalar state; the table was mapped by msix_setup.
    unsafe {
        if MSIX_TAB != 0 {
            write_volatile((MSIX_TAB + 12) as *mut u32, 1);
        }
        MSIX = 2;
        MSIX_WHY = why;
    }
}

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

/// Poll `done` for up to 5 seconds of real time (a spin count ran from a fifth of a second to
/// seconds, depending on the CPU, and a command that runs out of it now costs a controller reset).
fn wait(mut done: impl FnMut() -> bool) -> bool {
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    loop {
        if done() {
            return true;
        }
        if crate::timer::cycles().wrapping_sub(t0) / (hz / 1000).max(1) > 5000 {
            return false;
        }
        core::hint::spin_loop();
    }
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
        // A command that timed out earlier may still complete later, in the slot this command
        // would read first. Its status belongs to it, not to this command: take it off the queue
        // and keep waiting, so one slow command never shifts every later status by one.
        for _ in 0..QN {
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
            let status = ((e[3] >> 17) & 0x7FFF) as u16;
            if e[3] & 0xFFFF == cid as u32 {
                return Some(status);
            }
            crate::kprintln!(
                "[nvme] a late completion for command id {} (status {:#x}) set aside while waiting for {}",
                e[3] & 0xFFFF, status, cid
            );
        }
        crate::kprintln!("[nvme] queue {} command {:#x}: no completion of its own", q, cmd[0] & 0xFF);
        None
    }
}

/// Wait until CSTS.RDY is `want`, up to CAP.TO; false on a timeout or a fatal status (CFS).
fn ready_wait(want: u32) -> bool {
    // SAFETY: scalar read of boot-time state.
    let (t0, hz, to_ms) = (crate::timer::cycles(), crate::timer::hz().max(1), unsafe { TO_MS });
    loop {
        let st = r32(R_CSTS);
        if st & 1 == want || st & 2 != 0 {
            return st & 1 == want && st & 2 == 0;
        }
        if crate::timer::cycles().wrapping_sub(t0) / (hz / 1000).max(1) > to_ms {
            return false;
        }
        core::hint::spin_loop();
    }
}

/// Reset the controller and start it again with empty admin queues. Err(1): it did not stop,
/// Err(2): it did not become ready.
fn start_admin() -> Result<(), u8> {
    // Reset first: once CC.EN is off and CSTS.RDY reads 0 the controller has ended every command
    // it held, so nothing can still land in a queue or a buffer (architect's Q-a); only then are the
    // queues cleared (the completion phase bits must start at 0).
    w32(R_CC, 0);
    if !ready_wait(0) {
        return Err(1);
    }
    // SAFETY: the controller is stopped and not using them; single core.
    unsafe {
        core::ptr::write_bytes(addr_of_mut!(ACQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(IOCQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(ASQ), 0, 1);
        core::ptr::write_bytes(addr_of_mut!(IOSQ), 0, 1);
        *addr_of_mut!(ADMIN) = Pair { tail: 0, head: 0, phase: 1, cid: 0 };
        *addr_of_mut!(IO) = Pair { tail: 0, head: 0, phase: 1, cid: 0 };
    }
    // Point the admin queues and enable.
    w32(R_AQA, ((QN as u32 - 1) << 16) | (QN as u32 - 1));
    w64(R_ASQ, addr_of!(ASQ) as u64);
    w64(R_ACQ, addr_of!(ACQ) as u64);
    w32(R_CC, 1 | 6 << 16 | 4 << 20); // EN, NVM command set, 4 KiB pages, 64 B SQE, 16 B CQE
    if !ready_wait(1) {
        return Err(2);
    }
    Ok(())
}

/// The I/O queue pair: completion queue 1, then submission queue 1 feeding it.
fn make_io_queues() -> bool {
    let mut c = cmd(OPC_CREATE_CQ, 0, addr_of!(IOCQ) as u64);
    c[10] = ((QN as u32 - 1) << 16) | 1;
    // physically contiguous; with MSI-X, interrupts enabled (IEN, bit 1) on vector 0 of the table (IV = 0)
    // SAFETY: scalar read.
    c[11] = if unsafe { MSIX } != 0 { 1 | 1 << 1 } else { 1 };
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] create I/O completion queue failed");
        return false;
    }
    let mut c = cmd(OPC_CREATE_SQ, 0, addr_of!(IOSQ) as u64);
    c[10] = ((QN as u32 - 1) << 16) | 1;
    c[11] = 1 << 16 | 1; // completion queue 1, physically contiguous
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] create I/O submission queue failed");
        return false;
    }
    true
}

/// An I/O command got no answer (a hung controller, or one that reported a fatal status):
/// before 12 Nov every later command then waited in vain too, so the disk was lost until a
/// restart. Now the controller is reset (CC.EN off and on, which also ends whatever it still
/// held) and both queue pairs are made again, as Linux nvme_reset_ctrl does; the command that
/// failed stays failed, the next one runs on the fresh queues. The read-ahead is dropped.
fn restart(why: &str) -> bool {
    // SAFETY: single core, kernel context; commands the controller may still hold end at its reset
    // (start_admin) before any buffer is used again.
    unsafe {
        READY = false;
        RA_N = 0;
        RESETS = RESETS.saturating_add(1);
    }
    crate::kprintln!("[nvme] {}: resetting the controller and making its queues again (CSTS {:#x})", why, r32(R_CSTS));
    let ok = match start_admin() {
        Ok(()) => make_io_queues(),
        Err(_) => false,
    };
    if ok {
        // SAFETY: as above.
        unsafe { READY = true };
        crate::kprintln!("[nvme] controller ready again after the reset; the read-ahead was dropped");
    } else {
        w32(R_CC, 0); // stopped, so no command it may still hold writes into our buffers later
        crate::kprintln!("[nvme] the controller did not come back after the reset (CSTS {:#x}): the NVMe disk is not used until the next start", r32(R_CSTS));
    }
    ok
}

/// `nvmereset`: reset the controller by hand (the same path a command with no answer takes),
/// then read block 0 to show the fresh queues work. Nothing is written.
pub fn reset_cmd() {
    // SAFETY: scalar reads.
    if unsafe { BAR == 0 || !READY } {
        crate::kprintln!("nvmereset: no ready NVMe controller");
        return;
    }
    if restart("nvmereset asked for it") {
        let ok = io(OPC_READ, 0);
        crate::kprintln!("nvmereset: read of block 0 after the reset {}", if ok { "worked" } else { "FAILED" });
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

/// Asili v0 BINDS (pc/hwdb.rs): the probe below decides from this table.
pub(crate) const BINDS: &[crate::hwdb::Part] = &[crate::hwdb::pci_class(0x01_08_02, 0xFF_FF_FF, "NVMe storage controller")];

/// The PCI function this driver took, for `hwscan`.
pub fn at() -> Option<(u8, u8, u8)> {
    // SAFETY: scalar reads of boot-time state.
    unsafe { (READY).then_some(PCI_AT) }
}

/// Find the controller, bring it up with one I/O queue pair, and size namespace 1.
pub fn init() {
    let Some((b, d, f)) = pci::find_by(8, |b, d, f, id| crate::hwdb::pci_part(BINDS, b, d, f, id).is_some()) else { return };
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
    // SAFETY: as above.
    unsafe { TO_MS = ((cap >> 24) & 0xFF).max(1) * 500 };
    match start_admin() {
        Err(1) => {
            crate::kprintln!("[nvme] controller did not stop");
            return;
        }
        Err(_) => {
            crate::kprintln!("[nvme] controller did not become ready (CSTS {:#x})", r32(R_CSTS));
            return;
        }
        Ok(()) => {}
    }
    let buf = addr_of!(DATA) as u64;
    // Identify controller (CNS 1): the model number at bytes 24..64.
    let mut c = cmd(OPC_IDENTIFY, 0, buf);
    c[10] = 1;
    if submit(0, c) != Some(0) {
        crate::kprintln!("[nvme] identify controller failed");
        w32(R_CC, 0); // stopped: nothing it still holds may land in DATA later
        return;
    }
    let mut model = [b' '; 40];
    model.copy_from_slice(&data()[24..64]);
    // MDTS (byte 77): the largest transfer as a power of two of the smallest page (CAP.MPSMIN, 4 KiB
    // here: init refuses any other); 0 = no limit. VWC (byte 525 bit 0): a volatile write cache.
    let (mdts, mpsmin) = (data()[77] as u32, ((cap >> 48) & 0xF) as u32);
    // SAFETY: as above.
    unsafe {
        MAXB = if mdts == 0 || mdts > 20 { MAX_XFER } else { ((4096u64 << mpsmin) << mdts).min(MAX_XFER) };
        VWC = data()[525] & 1 != 0;
    }
    // Identify namespace 1 (CNS 0): size in blocks, and the block size of its format.
    if submit(0, cmd(OPC_IDENTIFY, 1, buf)) != Some(0) {
        crate::kprintln!("[nvme] identify namespace 1 failed");
        w32(R_CC, 0);
        return;
    }
    let id = data();
    let nsze = u64::from_le_bytes(id[0..8].try_into().unwrap_or([0; 8]));
    let fmt = (id[26] & 0xF) as usize;
    let lbads = id[128 + fmt * 4 + 2];
    let lba_bytes = 1u64 << lbads;
    msix_setup(b, d, f);
    if !make_io_queues() {
        w32(R_CC, 0);
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
    // SSDs) hold several of them; read_sector / write_sector map onto the block (see `per`)
    let usable = if !matches!(lba_bytes, 512 | 1024 | 2048 | 4096) {
        crate::kprintln!("[nvme] namespace 1 uses {}-byte blocks: not used for storage", lba_bytes);
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
        // SAFETY: scalar reads of boot-time state.
        let (vwc, maxb) = unsafe { (VWC, MAXB) };
        if krun() > kmax() {
            crate::kprintln!(
                "[nvme] the controller takes {} KiB per command: a run of up to {} blocks goes out as up to {} commands at once",
                maxb / 1024, krun(), krun().div_ceil(kmax())
            );
        }
        if vwc {
            crate::kprintln!("[nvme] the controller has a volatile write cache: every save ends with a Flush");
        }
        // SAFETY: scalar reads of boot-time state.
        match unsafe { (MSIX, MSIX_N, MSIX_WHY) } {
            (1, n, _) => crate::kprintln!(
                "[nvme] MSI-X on: I/O completions interrupt on vector {:#x} (entry 0 of {}, local APIC {}); a waiting command sleeps",
                crate::cpu::NVME_VECTOR, n, crate::cpu::lapic_id()
            ),
            (_, _, why) => crate::kprintln!("[nvme] no MSI-X ({}): I/O completions are polled", why),
        }
    }
}

/// Device blocks per command: the controller's MDTS (or MAX_XFER) over the block size.
fn kmax() -> u32 {
    // SAFETY: scalar reads of boot-time state.
    unsafe { (MAXB / LBA_BYTES.max(1)).max(1) as u32 }
}

/// Device blocks in one run (the 64 KiB read-ahead or write-gather), sent as one or more commands.
fn krun() -> u32 {
    // SAFETY: scalar read of boot-time state.
    (RA_BYTES as u64 / unsafe { LBA_BYTES }.max(1)) as u32
}

/// PRP1 and PRP2 for `len` bytes at `buf` (identity mapped: the address is the DMA address). Over two
/// pages, PRP2 points at list page `slot`, filled with every page after the first; the first entry
/// may start inside its page (NVMe 1.4 4.3). `len` is at most MAX_XFER, so one list page is enough.
fn prps(buf: u64, len: u64, slot: usize) -> (u64, u64) {
    let first = 4096 - (buf & 0xFFF);
    if len <= first {
        return (buf, 0);
    }
    let rest = buf + first; // page aligned
    if len <= first + 4096 {
        return (buf, rest);
    }
    let n = ((len - first).div_ceil(4096) as usize).min(512);
    // SAFETY: slot < NFLY; the list page belongs to this command until its completion is taken or the
    // controller is reset (submit_io, run_io).
    unsafe {
        let l = &mut (*addr_of_mut!(PRPL))[slot].0;
        for (i, e) in l.iter_mut().enumerate().take(n) {
            *e = rest + 4096 * i as u64;
        }
        QSTATS.1 += 1;
        (buf, addr_of!((*addr_of!(PRPL))[slot]) as u64)
    }
}

/// Send `cmds` (at most NFLY) on the I/O queue with ONE doorbell and take their completions in any
/// order by command id; `res[i]` = cmds[i]'s status. False when one got no answer within 5 s: the
/// caller then resets the controller before any buffer or list page of these commands is used again.
fn submit_io(cmds: &mut [[u32; 16]], res: &mut [Option<u16>]) -> bool {
    let n = cmds.len().min(NFLY).min(res.len());
    // SAFETY: single core, kernel context; the I/O queue is empty when this starts (every earlier
    // batch was either answered in full or ended in a reset), so n <= NFLY < QN entries fit.
    unsafe {
        let pair = &mut *addr_of_mut!(IO);
        let (sq, cq) = (addr_of_mut!(IOSQ.0) as *mut [u32; 16], addr_of!(IOCQ.0) as *const [u32; 4]);
        let mut cids = [0u16; NFLY];
        for i in 0..n {
            cids[i] = pair.cid;
            pair.cid = pair.cid.wrapping_add(1);
            cmds[i][0] |= (cids[i] as u32) << 16;
            write_volatile(sq.add(pair.tail), cmds[i]);
            pair.tail = (pair.tail + 1) % QN;
            res[i] = None;
        }
        let irq0 = IRQS.load(core::sync::atomic::Ordering::Relaxed);
        let sleep = MSIX == 1 && crate::cpu::interrupts_on();
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w32(doorbell(1, false), pair.tail as u32);
        QSTATS.0 = QSTATS.0.max(n as u32);
        let mut left = n;
        while left > 0 {
            let slot = cq.add(pair.head);
            let phase = pair.phase;
            let ok = if sleep {
                wait(|| crate::cpu::idle_until(|| (read_volatile(slot)[3] >> 16) & 1 == phase))
            } else {
                wait(|| (read_volatile(slot)[3] >> 16) & 1 == phase)
            };
            if !ok {
                crate::kprintln!("[nvme] {} of {} command(s) sent together got no answer", left, n);
                return false;
            }
            let e = read_volatile(slot);
            pair.head += 1;
            if pair.head == QN {
                pair.head = 0;
                pair.phase ^= 1;
            }
            w32(doorbell(1, true), pair.head as u32);
            if DROP > 0 {
                DROP -= 1;
                QSTATS.5 += 1;
                crate::kprintln!("[nvme] nvmetest: the completion of command id {} is lost on purpose", e[3] & 0xFFFF);
                continue;
            }
            let id = (e[3] & 0xFFFF) as u16;
            match (0..n).find(|&i| cids[i] == id && res[i].is_none()) {
                Some(i) => {
                    res[i] = Some(((e[3] >> 17) & 0x7FFF) as u16);
                    left -= 1;
                }
                None => {
                    QSTATS.4 += 1;
                    crate::kprintln!("[nvme] a completion for command id {} that is not in flight was set aside", id);
                }
            }
        }
        if sleep {
            WSTAT.0 += 1;
            // the batch is done: its interrupt has come by now, or comes within a moment (the controller writes
            // the completion first, then the message); none within 2 ms means interrupts are lost: poll instead
            let woke = || IRQS.load(core::sync::atomic::Ordering::Relaxed) != irq0;
            let t0 = crate::timer::cycles();
            let ms2 = (crate::timer::hz() / 500).max(1);
            while !woke() && crate::timer::cycles().wrapping_sub(t0) < ms2 {
                core::hint::spin_loop();
            }
            if woke() {
                WSTAT.1 += 1;
            } else {
                msix_off("a batch completed without its interrupt");
                crate::kprintln!("[nvme] a completion came without its MSI-X interrupt: MSI-X off, completions are polled from now on");
            }
        }
        true
    }
}

/// Run `cmds` with `submit_io`; on no answer, reset the controller (which ends everything it held, so
/// the buffers and list pages are ours again) and send them once more on the fresh queues.
fn run_io(cmds: &[[u32; 16]], res: &mut [Option<u16>]) -> bool {
    let n = cmds.len().min(NFLY);
    for attempt in 0..2 {
        let mut c = [[0u32; 16]; NFLY];
        c[..n].copy_from_slice(&cmds[..n]);
        if submit_io(&mut c[..n], res) {
            return true;
        }
        if !restart("a command got no answer") || attempt == 1 {
            return false;
        }
        crate::kprintln!("[nvme] sending the {} command(s) again after the reset", n);
    }
    false
}


/// At start-up: one Read of the first k blocks must equal the same blocks read one at a time,
/// else multi-block reads stay off (and writes with them). Reads only: safe on any disk. One
/// boot line says which.
fn multi_check() {
    // SAFETY: boot, single core; no command in flight.
    unsafe {
        let k = (krun() as u64).min(SECTORS) as u32;
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
            crate::kprintln!("[nvme] multi-block reads on (up to {} blocks per read, {} per command; the first {} equal one at a time)", k, kmax().min(k), k);
        } else {
            (MULTI, MULTIW, RA_N) = (2, 2, 0);
            crate::kprintln!(
                "[nvme] multi-block reads off ({}): one block per command",
                if read_ok { "they differed from one block at a time" } else { "the controller refused one" }
            );
        }
    }
}

/// One Read or Write of `k` device blocks (at most a run, `krun`) from `lba` on namespace 1 through `buf`:
/// one command when the controller takes it all at once, else up to NFLY commands per doorbell.
fn rw(opc: u8, lba: u64, k: u32, buf: u64) -> bool {
    // SAFETY: scalar reads.
    if !unsafe { READY } || k == 0 || k > krun() {
        return false;
    }
    // SAFETY: scalar read of boot-time state.
    let bs = unsafe { LBA_BYTES };
    let per_cmd = kmax();
    let mut done = 0u32;
    while done < k {
        let mut cmds = [[0u32; 16]; NFLY];
        let mut n = 0;
        while n < NFLY && done < k {
            let m = per_cmd.min(k - done);
            let at = buf + done as u64 * bs;
            let (p1, p2) = prps(at, m as u64 * bs, n);
            let mut c = cmd(opc, 1, p1);
            c[8] = p2 as u32;
            c[9] = (p2 >> 32) as u32;
            let l = lba + done as u64;
            c[10] = l as u32;
            c[11] = (l >> 32) as u32;
            c[12] = m - 1; // NLB is zero-based
            // SAFETY: scalar counter.
            unsafe { QSTATS.2 = QSTATS.2.max(m as u64 * bs) };
            cmds[n] = c;
            n += 1;
            done += m;
        }
        let mut res = [None; NFLY];
        if !run_io(&cmds[..n], &mut res) {
            return false;
        }
        if let Some(s) = res[..n].iter().flatten().find(|&&s| s != 0) {
            crate::kprintln!("[nvme] {} of {} blocks at {} refused (status {:#x})", if opc == OPC_READ { "read" } else { "write" }, k, lba, s);
            return false;
        }
    }
    true
}

/// An NVMe Flush when the controller has a volatile write cache: what it acknowledged is then on the
/// medium. True without one.
fn cache_flush() -> bool {
    // SAFETY: scalar reads.
    if !unsafe { VWC && READY } {
        return true;
    }
    let mut res = [None; 1];
    let ok = run_io(&[cmd(OPC_FLUSH, 1, 0)], &mut res) && res[0] == Some(0);
    // SAFETY: scalar counter.
    unsafe { QSTATS.3 += 1 };
    if !ok {
        crate::kprintln!("[nvme] the Flush after a save or an install step failed: the last writes may still be only in the disk's cache");
    }
    ok
}

/// The end of an install step (gpt.rs Disk::settle): gathered writes out, then the Flush.
pub fn settle() -> bool {
    flush() && cache_flush()
}

/// `nvmetest prp`: blocks from an odd block read into a buffer at page offset 512 (so the first PRP
/// entry starts inside its page and the rest go in a PRP list) must equal the same blocks read one at a
/// time. `nvmetest drop`: lose the next completion on purpose (the reset path, then the resend). Reads only.
pub fn test_cmd(arg: &str) {
    // SAFETY: scalar reads; shell context, no command in flight.
    if unsafe { BAR == 0 || !READY } {
        crate::kprintln!("nvmetest: no ready NVMe controller");
        return;
    }
    match arg {
        "lose" => {
            // SAFETY: scalar reads; the table entry's vector control word, as msix_off writes it.
            unsafe {
                if MSIX != 1 || MSIX_TAB == 0 {
                    crate::kprintln!("nvmetest lose: MSI-X is not on");
                    return;
                }
                write_volatile((MSIX_TAB + 12) as *mut u32, 1);
            }
            crate::kprintln!("nvmetest lose: MSI-X entry 0 masked behind the driver's back: the next completion comes without an interrupt");
        }
        "drop" => {
            // SAFETY: scalar state.
            unsafe { DROP = 1 };
            crate::kprintln!("nvmetest: the next I/O completion will be lost on purpose (the reset path takes over)");
        }
        "prp" => {
            // SAFETY: scalar reads of boot-time state.
            let (bs, n) = unsafe { (LBA_BYTES as usize, SECTORS) };
            if !flush() {
                crate::kprintln!("nvmetest prp: the gathered writes could not be sent first");
                return;
            }
            let k = ((RA_BYTES - 512) / bs).min(n.saturating_sub(1) as usize) as u32;
            if k < 2 {
                crate::kprintln!("nvmetest prp: the disk is too small");
                return;
            }
            // SAFETY: RA is ours while the read-ahead is dropped; one batch at a time.
            unsafe {
                RA_N = 0;
                let ra = addr_of_mut!(RA.0) as *mut u8;
                core::ptr::write_bytes(ra, 0xA5, RA_BYTES);
                let ok = rw(OPC_READ, 1, k, ra as u64 + 512);
                let mut same = ok;
                for b in 0..k as u64 {
                    if !same {
                        break;
                    }
                    same = io(OPC_READ, 1 + b) && data()[..bs] == core::slice::from_raw_parts(ra.add(512 + b as usize * bs), bs)[..];
                }
                crate::kprintln!(
                    "nvmetest prp: {} blocks from block 1 read into a buffer at page offset 512 ({} command(s), PRP list): {}",
                    k,
                    k.div_ceil(kmax()),
                    if !ok { "the read FAILED" } else if same { "equal to one block at a time" } else { "DIFFERENT from one block at a time" }
                );
            }
        }
        _ => crate::kprintln!("nvmetest: say prp (an odd-block read into a buffer at page offset 512, compared), drop (lose the next completion) or lose (mask the MSI-X entry)"),
    }
}

/// Device block `b` into DATA: from the read-ahead when it holds it, else a new multi-block read
/// of up to 64 KiB from it (once the boot check passed), else one block.
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
            let k = (krun() as u64).min(SECTORS.saturating_sub(b)) as u32;
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
                        crate::kprintln!("[nvme] multi-block writes on (up to {} blocks per write, {} per command; the first {} read back equal)", krun(), kmax().min(krun()), k);
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
    let mut res = [None; 1];
    if !run_io(&[c], &mut res) {
        return false;
    }
    match res[0] {
        Some(0) => true,
        s => {
            crate::kprintln!(
                "[nvme] {} of block {} failed (status {:#x})",
                if opc == OPC_READ { "read" } else { "write" },
                lba,
                s.unwrap_or(0xFFFF)
            );
            false
        }
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
    // 64 KiB; one block while multi-block writes are off). A block bigger than a sector (1, 2 or
    // 4 KiB) is read once when the run reaches it and its sectors changed in place, so a 4 KiB
    // SSD is written once per block, not once per sector. The run is sent when it is full and its
    // last block complete, on a gap, before any read, or at the VFS's last sector (or a lone
    // write past the VFS area: the self-test).
    // the save's own last sector (vfs.rs writes big files' extension sectors first, then 0..=48, so 48
    // ends every save); 12 Nov 2026: before, every sector past 48 counted, so each extension sector went
    // out alone, and with the Flush after a save each would have cost a Flush too. A lone write anywhere
    // else (the block self-test) waits in the gather until the next read, which sends it first.
    let last = n == 2 * crate::vfs::MAX_FILES as u64;
    // SAFETY: scalar read.
    let n = n + unsafe { PART.0 };
    // SAFETY: single core, kernel context, one command in flight.
    unsafe {
        let (per, bs) = (per(), LBA_BYTES as usize);
        let (b, off) = (n / per, (n % per) as usize * 512);
        let cap = if MULTI == 1 && MULTIW != 2 { krun() } else { 1 };
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
            if last {
                ok = cache_flush() && ok; // the save is on the medium, not only in the disk's cache
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
/// as several commands (rw), each part checked. The same callers and the same rule as table_write.
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
        let kk = krun() as u64; // device blocks per run (rw sends it as one or more commands)
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
        let (q, maxb, vwc) = unsafe { (QSTATS, MAXB, VWC) };
        crate::kprintln!(
            "  queue: most commands in flight at once {}, PRP-list commands {}, largest transfer {} KiB, controller takes {} KiB per command",
            q.0, q.1, q.2 / 1024, maxb / 1024
        );
        if vwc {
            crate::kprintln!("  volatile write cache: yes, {} Flush(es) sent (one at the end of every save and every install step)", q.3);
        }
        // SAFETY: scalar reads.
        let (m, why, w) = unsafe { (MSIX, MSIX_WHY, WSTAT) };
        let irqs = IRQS.load(core::sync::atomic::Ordering::Relaxed);
        match m {
            1 => crate::kprintln!(
                "  interrupts: MSI-X on vector {:#x}, {} taken; {} batch(es) waited for asleep, {} woken by their interrupt",
                crate::cpu::NVME_VECTOR, irqs, w.0, w.1
            ),
            2 => crate::kprintln!("  interrupts: MSI-X off ({}), polled; {} taken before, {} batch(es) woken by them", why, irqs, w.1),
            _ => crate::kprintln!("  interrupts: none ({}), polled", why),
        }
        if q.4 + q.5 != 0 {
            crate::kprintln!("  completions: {} late one(s) set aside, {} lost on purpose (nvmetest drop)", q.4, q.5);
        }
        // SAFETY: scalar reads.
        let (tr, t, rs) = unsafe { (TRUN, TSTATS, RESETS) };
        if rs != 0 {
            crate::kprintln!("  controller resets (a command got no answer, or nvmereset): {}", rs);
        }
        if tr != 0 {
            crate::kprintln!(
                "  big copies (installdisk, bootcopy): {} run(s) in one command, {} sector(s), each read back ({})",
                t.0, t.1, if tr == 1 { "on" } else { "off" }
            );
        }
    }
}
