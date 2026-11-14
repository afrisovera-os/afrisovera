//! xHCI (USB 3) host controller, a USB boot keyboard and a USB boot mouse on the PC: roadmap
//! PC step 3, "USB keyboard/mouse (xHCI)" (keyboard 5 Nov 2026, mouse 5 Nov 2026).
//!
//! Every PC since ~2012 has an xHCI controller (PCI class 0C:03:30), and a laptop's own
//! keyboard is often a USB device behind it. This driver brings the controller up (command
//! ring, one event ring), resets each connected port, gives the first HID boot keyboard a
//! slot and an address, reads its descriptors, configures its interrupt-IN endpoint, and
//! from then on the local-APIC timer interrupt turns each 8-byte boot report into
//! characters on the same console input ring COM1 fills, so the shell can be typed on a
//! real USB keyboard. A HID boot mouse (3/1/2) on another port is enumerated the same way;
//! its 3-byte reports move a pointer over the 640x480 console (clamped, drawn by screen.rs)
//! and count button presses. One keyboard and one mouse, polled, no hot-plug yet.
//! xHCI 1.2: registers 5, rings/TRBs 4.9-4.11 and 6.4, contexts 6.2, enumeration 4.3.

use crate::mmu;
use crate::pci;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

// PCI configuration space through the shared pc/pci.rs (review R1-8).
use pci::{cfg_read32 as cfg32, cfg_write32};

// TRB types
const T_NORMAL: u32 = 1;
const T_SETUP: u32 = 2;
const T_DATA: u32 = 3;
const T_STATUS: u32 = 4;
const T_ISOCH: u32 = 5;
const T_LINK: u32 = 6;
const T_ENABLE_SLOT: u32 = 9;
const T_DISABLE_SLOT: u32 = 10;
const T_ADDRESS_DEVICE: u32 = 11;
const T_CONFIGURE_EP: u32 = 12;
const T_EVALUATE_CTX: u32 = 13;
const T_RESET_EP: u32 = 14;
const T_STOP_EP: u32 = 15;
const T_SET_TR_DEQ: u32 = 16;
const T_NOOP: u32 = 23;
const CC_STALL: u32 = 6;
const T_EV_TRANSFER: u32 = 32;
const T_EV_COMMAND: u32 = 33;
const T_EV_PORT: u32 = 34;
const CC_SUCCESS: u32 = 1;
const CC_SHORT: u32 = 13;

#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Trb {
    param: u64,
    status: u32,
    control: u32,
}
const TRB0: Trb = Trb { param: 0, status: 0, control: 0 };

const NCMD: usize = 32;
const NEV: usize = 64;
const NXFER: usize = 32;

// F2 (9 Nov 2026): aligned to 1024, at least its size (no ring is bigger), so no ring crosses a
// 64 KiB block; pages for contexts and arrays, a buffer's own size for buffers (dma_check)
#[repr(C, align(1024))]
struct Ring<const N: usize>([Trb; N]);
#[repr(C, align(64))]
struct Erst([u64; 2]); // one segment: base, size
#[repr(C, align(4096))]
struct Dcbaa([u64; 256]);
#[repr(C, align(4096))]
struct Ctx([u8; 4096]);
#[repr(C, align(512))]
struct Buf([u8; 512]);

/// Controllers driven at once (7 Nov 2026): every xHCI with a device plugged in at power-on, up to
/// the 4 pci_find lists. Each has its own command ring, event ring and device context array (the
/// DMA structures below, indexed by CUR) and its own registers and ring positions (`Bank`, swapped
/// in by `select`); devices, transfer rings and the shared input context / control buffer are one
/// pool (enumeration and control transfers run one at a time).
const NCTL: usize = 4;
static mut CMD: [Ring<NCMD>; NCTL] = [const { Ring([TRB0; NCMD]) }; NCTL];
static mut EVT: [Ring<NEV>; NCTL] = [const { Ring([TRB0; NEV]) }; NCTL];
/// Devices a slot is kept for: one keyboard, one mouse.
const NDEV: usize = 9; // a keyboard, a mouse, a USB Ethernet adapter, a hub, a USB stick, a headset, a serial cable, a camera, Bluetooth
static mut EP0: [Ring<NXFER>; NDEV] = [const { Ring([TRB0; NXFER]) }; NDEV];
static mut EPIN: [Ring<NXFER>; NDEV] = [const { Ring([TRB0; NXFER]) }; NDEV];
static mut ERST: [Erst; NCTL] = [const { Erst([0; 2]) }; NCTL];
static mut DCBAA: [Dcbaa; NCTL] = [const { Dcbaa([0; 256]) }; NCTL];
static mut INCTX: Ctx = Ctx([0; 4096]);
static mut OUTCTX: [Ctx; NDEV] = [const { Ctx([0; 4096]) }; NDEV];
static mut SCRATCH_ARR: [Dcbaa; NCTL] = [const { Dcbaa([0; 256]) }; NCTL];
static mut DBUF: Buf = Buf([0; 512]);
static mut REPORT: [Buf; NDEV] = [const { Buf([0; 512]) }; NDEV];

/// A producer ring's state: next slot and its cycle bit.
#[derive(Clone, Copy)]
struct Prod {
    idx: usize,
    cycle: u32,
}
static mut PCMD: Prod = Prod { idx: 0, cycle: 1 };
static mut PEP0: [Prod; NDEV] = [Prod { idx: 0, cycle: 1 }; NDEV];
static mut PEPIN: [Prod; NDEV] = [Prod { idx: 0, cycle: 1 }; NDEV];
static mut EV_IDX: usize = 0;
static mut EV_CYCLE: u32 = 1;

static mut OP: u64 = 0; // operational registers
static mut RT: u64 = 0; // runtime registers
static mut DB: u64 = 0; // doorbell array
static mut CSZ: usize = 32; // context size (32 or 64 bytes)
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut PORTS: u32 = 0;
static mut CONNECTED: u32 = 0;

/// The controller whose registers and ring positions are in the statics above.
static mut CUR: usize = 0;
/// How many controllers run (CUR < NRUN); 0 until the first one is up.
static mut NRUN: usize = 0;
/// A mainline `select` is half done: the timer poll stands aside.
static mut SWITCHING: bool = false;
/// The other controllers' copies of OP, RT, DB, CSZ, PCI_AT, PORTS, CONNECTED and the
/// command/event ring positions.
#[derive(Clone, Copy)]
struct Bank {
    pcmd: Prod,
    ev_idx: usize,
    ev_cycle: u32,
    op: u64,
    rt: u64,
    db: u64,
    csz: usize,
    pci_at: (u8, u8, u8),
    ports: u32,
    connected: u32,
}
const BANK0: Bank = Bank {
    pcmd: Prod { idx: 0, cycle: 1 },
    ev_idx: 0,
    ev_cycle: 1,
    op: 0,
    rt: 0,
    db: 0,
    csz: 32,
    pci_at: (0, 0, 0),
    ports: 0,
    connected: 0,
};
static mut BANKS: [Bank; NCTL] = [BANK0; NCTL];

/// Make controller `c` the current one (its registers, ring positions). Nothing to do with a
/// single controller, so a one-controller PC runs exactly as before.
unsafe fn select(c: usize) {
    if c == CUR || c >= NCTL {
        return;
    }
    let was = SWITCHING;
    SWITCHING = true;
    BANKS[CUR] = Bank {
        pcmd: PCMD, ev_idx: EV_IDX, ev_cycle: EV_CYCLE, op: OP, rt: RT, db: DB, csz: CSZ,
        pci_at: PCI_AT, ports: PORTS, connected: CONNECTED,
    };
    let b = BANKS[c];
    (PCMD, EV_IDX, EV_CYCLE, OP, RT, DB, CSZ) = (b.pcmd, b.ev_idx, b.ev_cycle, b.op, b.rt, b.db, b.csz);
    (PCI_AT, PORTS, CONNECTED) = (b.pci_at, b.ports, b.connected);
    CUR = c;
    SWITCHING = was;
}
const KIND_KBD: u8 = 1;
const KIND_MOUSE: u8 = 2;
const KIND_NET: u8 = 3;
const KIND_HUB: u8 = 4;
const KIND_MSC: u8 = 5;
const KIND_TABLET: u8 = 6;
const KIND_AUDIO: u8 = 7;
const KIND_SERIAL: u8 = 8;
const KIND_CAM: u8 = 9;
const KIND_BT: u8 = 10;

// --- an absolute pointer: a USB touchscreen, graphics tablet or a remote console's (KVM/BMC)
// mouse. Not a boot device, so its reports are laid out by its HID report descriptor
// (HID 1.11 section 6.2.2), read once and parsed for X, Y and the buttons. ---
/// One field of a report: where it starts (bits from the report's first byte, the report ID
/// byte included), how wide, and its logical range.
#[derive(Clone, Copy)]
struct Field {
    bit: u32,
    size: u32,
    min: i32,
    max: i32,
}
const NOF: Field = Field { bit: 0, size: 0, min: 0, max: 0 };
/// The absolute pointer's report: its report ID (0 = none), absolute X and Y, and the
/// buttons (`nbtn` one-bit fields from `btn`; a touchscreen's tip switch counts as the left).
#[derive(Clone, Copy)]
struct Abs {
    id: u8,
    x: Field,
    y: Field,
    btn: u32,
    nbtn: u32,
    tip: bool,
}
static mut TAB: Abs = Abs { id: 0, x: NOF, y: NOF, btn: 0, nbtn: 0, tip: false };
static mut TAB_LEN: u32 = 8; // bytes asked for per report (the endpoint's packet size)
static mut TABLET_SLOT: u32 = 0;
static mut TABLET_PORT: u32 = 0;
static mut TREPORTS: u64 = 0;

/// Every variable Input field of a HID report descriptor, in order: `on(report id, usage
/// (page << 16 | id), the field, the Input item's flags, its index in the item, the item's
/// count)`. A field's bit counts from the report's first byte, the report ID byte included.
/// None when the descriptor uses more report IDs than are followed. Push/pop and long items
/// are skipped.
fn hid_inputs(r: &[u8], mut on: impl FnMut(u8, u32, Field, u32, u32, u32)) -> Option<()> {
    let (mut page, mut lmin, mut lmax, mut lmax_u, mut rsize, mut rcount, mut rid) = (0u32, 0i32, 0i32, 0i32, 0u32, 0u32, 0u8);
    let (mut usages, mut nu, mut umin, mut umax, mut range) = ([0u32; 16], 0usize, 0u32, 0u32, false);
    let (mut offs, mut noffs) = ([(0u8, 0u32); 8], 0usize); // (report id, its next input bit)
    let mut i = 0;
    while i < r.len() {
        let b = r[i];
        if b == 0xFE {
            // a long item: 0xFE, its data size, its tag, the data
            i += 3 + *r.get(i + 1)? as usize;
            continue;
        }
        let n = [0usize, 1, 2, 4][(b & 3) as usize];
        if i + 1 + n > r.len() {
            break;
        }
        let mut u = 0u32;
        for k in 0..n {
            u |= (r[i + 1 + k] as u32) << (8 * k);
        }
        let s = match n {
            1 => u as u8 as i8 as i32,
            2 => u as u16 as i16 as i32,
            4 => u as i32,
            _ => 0,
        };
        let full = |v: u32| if n == 4 { v } else { page << 16 | v };
        match ((b >> 2) & 3, b >> 4) {
            (1, 0) => page = u,
            (1, 1) => lmin = s,
            (1, 2) => (lmax, lmax_u) = (s, u as i32),
            (1, 7) => rsize = u,
            (1, 8) => rid = u as u8,
            (1, 9) => rcount = u,
            (2, 0) => {
                if nu < usages.len() {
                    usages[nu] = full(u);
                    nu += 1;
                }
            }
            (2, 1) => (umin, range) = (full(u), true),
            (2, 2) => (umax, range) = (full(u), true),
            (0, 8) => {
                // an Input item: its fields follow the previous ones of the same report
                let o = match offs[..noffs].iter().position(|e| e.0 == rid) {
                    Some(o) => o,
                    None if noffs < offs.len() => {
                        offs[noffs] = (rid, if rid != 0 { 8 } else { 0 });
                        noffs += 1;
                        noffs - 1
                    }
                    None => return None,
                };
                let base = offs[o].1;
                let max = if lmax < lmin { lmax_u } else { lmax };
                if u & 1 == 0 && u & 2 != 0 {
                    for k in 0..rcount.min(64) {
                        let usage = if nu > 0 {
                            usages[(k as usize).min(nu - 1)]
                        } else if range {
                            (umin + k).min(umax)
                        } else {
                            0
                        };
                        on(rid, usage, Field { bit: base + k * rsize, size: rsize, min: lmin, max }, u, k, rcount);
                    }
                }
                offs[o].1 = base + rsize * rcount;
                (nu, range) = (0, false);
            }
            (0, _) => (nu, range) = (0, false), // output, feature, collection, end: locals end
            _ => {}
        }
        i += 1 + n;
    }
    Some(())
}

/// A HID report descriptor -> the first absolute X/Y pair (Generic Desktop X 0x30, Y 0x31,
/// input, variable, not relative) and, in the same report, the buttons (Button page 1..3) or
/// else a digitizer's tip switch (0x0D:0x42). None for a relative mouse, a keyboard, or
/// anything without absolute X and Y.
fn parse_abs(r: &[u8]) -> Option<Abs> {
    let mut x: Option<(u8, Field)> = None;
    let mut y: Option<Field> = None;
    let (mut btns, mut nb) = ([(0u8, 0u32, 0u32); 4], 0usize); // first buttons per report id
    let (mut tips, mut nt) = ([(0u8, 0u32); 4], 0usize); // first tip switch per report id
    hid_inputs(r, |rid, usage, f, flags, k, rcount| {
        let absolute = flags & 4 == 0 && f.size > 0 && f.size <= 32 && f.max > f.min;
        match usage {
            0x0001_0030 if absolute && x.is_none() => x = Some((rid, f)),
            0x0001_0031 if absolute && y.is_none() && x.map(|x| x.0) == Some(rid) => y = Some(f),
            0x0009_0001 if f.size == 1 && nb < btns.len() && !btns[..nb].iter().any(|e| e.0 == rid) => {
                btns[nb] = (rid, f.bit, (rcount - k).min(3));
                nb += 1;
            }
            0x000D_0042 if f.size == 1 && nt < tips.len() && !tips[..nt].iter().any(|e| e.0 == rid) => {
                tips[nt] = (rid, f.bit);
                nt += 1;
            }
            _ => {}
        }
    })?;
    let (id, x) = x?;
    let y = y?;
    let mut a = Abs { id, x, y, btn: 0, nbtn: 0, tip: false };
    if let Some(&(_, bit, n)) = btns[..nb].iter().find(|e| e.0 == id) {
        (a.btn, a.nbtn) = (bit, n);
    } else if let Some(&(_, bit)) = tips[..nt].iter().find(|e| e.0 == id) {
        (a.btn, a.nbtn, a.tip) = (bit, 1, true);
    }
    Some(a)
}

// --- a USB mouse's wheel (9 Nov 2026, F7 for USB): a boot report has no wheel (HID B.2:
// buttons, X, Y only), so a mouse whose report descriptor has one is kept in the report
// protocol and its reports are read by the descriptor's layout. ---
/// A relative mouse's report: its report ID (0 = none), relative X, Y and wheel (Generic
/// Desktop 0x38), and the buttons (`nbtn` one-bit fields from `btn`).
#[derive(Clone, Copy)]
struct Rel {
    id: u8,
    x: Field,
    y: Field,
    wheel: Field,
    btn: u32,
    nbtn: u32,
}
static mut MREL: Option<Rel> = None; // Some: the mouse is read in the report protocol
static mut MREP_LEN: u32 = 8; // bytes asked for per report then (the endpoint's packet size)
static mut MWHEEL: u64 = 0; // wheel notches seen

/// A HID report descriptor -> the first relative X/Y pair with a wheel in the same report,
/// and that report's buttons. None for a mouse without a wheel (it stays a boot mouse).
fn parse_rel(r: &[u8]) -> Option<Rel> {
    let mut x: Option<(u8, Field)> = None;
    let (mut y, mut w): (Option<Field>, Option<Field>) = (None, None);
    let (mut btns, mut nb) = ([(0u8, 0u32, 0u32); 4], 0usize); // first buttons per report id
    hid_inputs(r, |rid, usage, f, flags, k, rcount| {
        let rel = flags & 4 != 0 && f.size >= 2 && f.size <= 32 && f.min < 0;
        let same = x.map(|x| x.0) == Some(rid);
        match usage {
            0x0001_0030 if rel && x.is_none() => x = Some((rid, f)),
            0x0001_0031 if rel && y.is_none() && same => y = Some(f),
            0x0001_0038 if rel && w.is_none() && same => w = Some(f),
            0x0009_0001 if f.size == 1 && nb < btns.len() && !btns[..nb].iter().any(|e| e.0 == rid) => {
                btns[nb] = (rid, f.bit, (rcount - k).min(3));
                nb += 1;
            }
            _ => {}
        }
    })?;
    let (id, x) = x?;
    let mut m = Rel { id, x, y: y?, wheel: w?, btn: 0, nbtn: 0 };
    if let Some(&(_, bit, n)) = btns[..nb].iter().find(|e| e.0 == id) {
        (m.btn, m.nbtn) = (bit, n);
    }
    Some(m)
}

/// `size` bits of report `r` from bit `bit` (little-endian, as HID packs them).
fn bits(r: &[u8], bit: u32, size: u32) -> u32 {
    let mut v = 0u32;
    for k in 0..size.min(32) {
        let b = bit + k;
        if r.get((b / 8) as usize).is_some_and(|&x| x >> (b % 8) & 1 != 0) {
            v |= 1 << k;
        }
    }
    v
}

/// A field's value, sign-extended when its logical minimum is negative.
fn signed(r: &[u8], f: Field) -> i32 {
    let v = bits(r, f.bit, f.size) as i64;
    let v = if f.min < 0 && f.size < 32 && v >> (f.size - 1) & 1 != 0 { v - (1 << f.size) } else { v };
    v as i32
}

/// A field's value scaled onto 0..=`span`-1 pixels.
fn scale(r: &[u8], f: Field, span: i32) -> i32 {
    let mut v = bits(r, f.bit, f.size) as i64;
    if f.min < 0 && f.size < 32 && v >> (f.size - 1) & 1 != 0 {
        v -= 1 << f.size; // a signed field
    }
    let (lo, hi) = (f.min as i64, f.max as i64);
    ((v.clamp(lo, hi) - lo) * (span as i64 - 1) / (hi - lo)) as i32
}

// --- USB mass storage (Bulk-Only Transport, SCSI): one stick or disk, LUN 0 ---
#[repr(C, align(64))]
struct Small([u8; 64]);
#[repr(C, align(4096))]
struct Block([u8; 4096]);
static mut MSC_OUT_RING: Ring<NXFER> = Ring([TRB0; NXFER]);
static mut PMSCOUT: Prod = Prod { idx: 0, cycle: 1 };
static mut CBW: Small = Small([0; 64]);
static mut CSW: Small = Small([0; 64]);
static mut MSCBUF: Block = Block([0; 4096]);
static mut MSC_D: usize = usize::MAX;
static mut MSC_SLOT: u32 = 0;
static mut MSC_PORT: u32 = 0;
static mut MSC_IN: u32 = 0; // DCIs
static mut MSC_OUT: u32 = 0;
static mut MSC_EPI_ADDR: u8 = 0; // endpoint addresses, for CLEAR_FEATURE(ENDPOINT_HALT)
static mut MSC_EPO_ADDR: u8 = 0;
static mut MSC_IFACE: u8 = 0;
static mut MSC_RECOVERIES: u64 = 0;
static mut MSC_STALE: u64 = 0; // transfer events for an older TRB, set aside by wait_xfer
static mut CTL_STALE: u64 = 0; // endpoint 0 events for an older request, set aside by wait_control
static mut CTL_TOOK: u64 = 0; // the TRB whose event ended the last control transfer (lsusb late0)
static mut CTL_WANT: u64 = 0; // the status TRB of the last control transfer queued
static mut CTL_RESETS: u64 = 0; // endpoint 0 found halted after a failed request and reset
static mut CTL_QUIET: bool = false; // lsusb stall0: its refused request is not reported
static mut MSC_TAG: u32 = 1;
static mut MSC_BLOCKS: u64 = 0;
static mut MSC_BS: u32 = 512;
static mut MSC_USABLE: bool = false;
/// The stick answers sector reads and writes (its block size is one we use, block 0 reads, and it needs no 16-byte
/// commands): esp.rs may read and write its FAT32 (the boot stick: firmware files, `report save`). MSC_USABLE is
/// the narrower "holds the VFS or is blank" (14 Nov 2026: a FAT32 boot stick is never MSC_USABLE, so the FAT
/// code needs this one).
static mut MSC_RAW: bool = false;
static mut MSC_NAME: [u8; 24] = [b' '; 24];
/// Room for 8 KiB DMA buffers that each start on an 8 KiB line, so none crosses a 64 KiB line (one TRB each,
/// xHCI 4.11.7.1). `align(8192)` is not enough: the UEFI loader puts the image only on a 4 KiB line, so a buffer
/// aligned to 8 KiB when linked can sit 4 KiB off it when run (13 Nov 2026: with the fonts merge every such
/// buffer moved 4 KiB off, and the debug build's alignment check stopped the kernel at the camera's). The room is
/// 4 KiB longer than its buffers; they start at its first 8 KiB line (`at8k`). Never read as a typed reference.
#[repr(C, align(4096))]
struct Room8k<const N: usize>([u8; N]);
/// The bytes of a room for `n` buffers of 8 KiB.
const fn room8k(n: usize) -> usize {
    n * 8192 + 4096
}
/// Buffer `i` of the room at `p`: on an 8 KiB line, inside the room.
fn at8k(p: *const u8, i: usize) -> *mut u8 {
    let a = p as usize;
    (((a + 0x1FFF) & !0x1FFF) + i * 8192) as *mut u8
}
/// The stick's read-ahead and write-behind buffers (MSC_RA_BYTES each).
unsafe fn msc_ra() -> *mut u8 {
    at8k(addr_of!(MSC_RA) as *const u8, 0)
}
unsafe fn msc_wb() -> *mut u8 {
    at8k(addr_of!(MSC_WB) as *const u8, 0)
}

// Multi-block commands (9 Nov 2026): one READ(10) or WRITE(10) of up to 8 KiB (16 sectors of
// 512 B) instead of one command per device block. Reads: the blocks of the last such read are
// kept from MSC_RA_AT (MSC_RA_N valid) and the next reads served from them. Writes: the VFS
// saves its sectors in order, so a run of device blocks is gathered from MSC_WB_AT (MSC_WB_N
// of them; since 10 Nov 2026 on 1, 2 and 4 KiB-block disks too, each block read once and its
// sectors changed in place) and sent when it ends (its last sector, a gap, 8 KiB) or before
// any read. Each way is checked once against one block at a time before it is trusted.
const MSC_RA_BYTES: usize = 8192;
static mut MSC_RA: Room8k<{ room8k(1) }> = Room8k([0; room8k(1)]);
static mut MSC_RA_AT: u64 = 0;
static mut MSC_RA_N: u32 = 0;
static mut MSC_WB: Room8k<{ room8k(1) }> = Room8k([0; room8k(1)]);
static mut MSC_WB_AT: u64 = 0;
static mut MSC_WB_N: u32 = 0;
/// 0 = not checked, 1 = on, 2 = off; reads and writes apart.
static mut MSC_MULTI: u8 = 0;
static mut MSC_MULTIW: u8 = 0;
/// Disks of more than 2^32 blocks (over 2 TiB with 512-byte blocks): READ(10)/WRITE(10) carry a
/// 32-bit block address, so every read and write goes as READ(16)/WRITE(16), as Linux does once
/// READ CAPACITY(10) says 0xFFFFFFFF and READ CAPACITY(16) gives the real size (SBC-3 5.16).
static mut MSC_LONG: bool = false;
/// The 16-byte check at the last block: 0 = not run, 1 = passed, 2 = failed.
static mut MSC_LONGCHK: u8 = 0;
/// Reads: sectors asked for, served from the read-ahead, multi-block commands, single-block ones.
static mut MSC_RSTATS: (u64, u64, u64, u64) = (0, 0, 0, 0);
/// Writes: sectors written, multi-block commands, single-block ones.
static mut MSC_WSTATS: (u64, u64, u64) = (0, 0, 0);
/// Where the device being enumerated sits behind hubs: its route string (one 4-bit hub port
/// per tier) and, for a full/low-speed device behind a high-speed hub, that hub's slot and
/// port (the transaction translator). (0, 0, 0) for a device on a root port.
static mut PATH: (u32, u32, u32) = (0, 0, 0);
static mut HUBS: u32 = 0;
/// The last device enumerated without being used: (vendor, product, device class, the class
/// of its first interface), for the boot log, so a real machine's report says what it was.
static mut LAST_OTHER: (u16, u16, u8, u8) = (0, 0, 0, 0);
/// The same device's first mass storage interface that is not Bulk-Only SCSI, as (subclass,
/// protocol), when msc_setup saw one: a disk that speaks only UAS is then named as such.
static mut MSC_SEEN: (u8, u8) = (0, 0);
// UAS (USB Attached SCSI, 12 Nov 2026): a disk that offers only UAS, on a high-speed port, where
// UAS runs without streams (UAS r04, as Linux uas.c for such devices): the status pipe is armed,
// the command IU goes out on the command pipe, the device answers READ READY or WRITE READY on the
// status pipe before the data, and a SENSE IU ends the command. The four pipes, by the Pipe Usage
// descriptor after each endpoint: command (MSC_OUT_RING), status (UAS_ST_RING), data in (EPIN[d])
// and data out (UAS_DO_RING). On a USB 3 port UAS needs streams (UAS_STREAMS, below).
static mut MSC_UAS: bool = false;
static mut UAS_DCI: [u32; 4] = [0; 4]; // command, status, data in, data out
static mut UAS_ST_RING: Ring<NXFER> = Ring([TRB0; NXFER]);
static mut PUASST: Prod = Prod { idx: 0, cycle: 1 };
static mut UAS_DO_RING: Ring<NXFER> = Ring([TRB0; NXFER]);
static mut PUASDO: Prod = Prod { idx: 0, cycle: 1 };
static mut UAS_ST: Buf = Buf([0; 512]);
/// A transfer event of one of the four pipes that came while another was waited for, by DCI.
static mut UAS_EV: [Option<Trb>; 32] = [None; 32];
static mut UAS_TAG: u16 = 0;
// UAS on a USB 3 port (12 Nov 2026): streams, as UAS r04 and Linux uas.c. The status, data in and
// data out pipes each get a Stream Context Array of 4 (MaxPStreams 1, linear, streams 1 to 3) and
// streams 1 and 2 are used: the tag of a command is its stream on all three pipes (tags 1 and 2
// in turn, one command at a time). Status, data and command are queued at once and the device
// moves the data when it is ready (no READ READY or WRITE READY with streams). Rings by pipe and
// stream: UAS_SR[2 * (pipe - 1) + stream - 1], pipe 1 status, 2 data in, 3 data out.
static mut UAS_STREAMS: bool = false;
/// On a USB 3 port, the data pipes' bursts from their SuperSpeed Endpoint Companions (bMaxBurst
/// + 1 packets, in and out), set as each endpoint's Max Burst Size; (0, 0) below USB 3.
static mut MSC_BURST: (u32, u32) = (0, 0);
#[repr(C, align(64))]
struct Sca([[u32; 4]; 4]);
static mut UAS_SCA: [Sca; 3] = [const { Sca([[0; 4]; 4]) }; 3];
static mut UAS_SR: [Ring<NXFER>; 6] = [const { Ring([TRB0; NXFER]) }; 6];
static mut PUAS_SR: [Prod; 6] = [Prod { idx: 0, cycle: 1 }; 6];
/// Every device enumerated without being used since power-on, once per vendor:product (the same
/// four fields as LAST_OTHER), for `wifi`: a USB Wi-Fi stick is named from its vendor:product.
static mut OTHERS: [(u16, u16, u8, u8); 8] = [(0, 0, 0, 0); 8];
static mut NOTHERS: usize = 0;
unsafe fn note_other() {
    let (o, n, list) = (LAST_OTHER, NOTHERS, OTHERS);
    if (o.0, o.1) != (0, 0) && n < list.len() && !list[..n].iter().any(|x| (x.0, x.1) == (o.0, o.1)) {
        OTHERS[n] = o;
        NOTHERS = n + 1;
    }
}
/// The devices in use now, for `hwscan`: (vendor, product, what this driver does with it), and how many.
pub(crate) fn used() -> ([(u16, u16, &'static str); NDEV], usize) {
    let mut out = [(0, 0, ""); NDEV];
    let mut n = 0;
    // SAFETY: single core, shell context; plain copies of the device table.
    unsafe {
        for dv in (*addr_of!(DEVS)).iter() {
            if dv.slot == 0 {
                continue;
            }
            let what = match dv.kind {
                KIND_KBD => "keyboard (usb-keyboard)",
                KIND_MOUSE => "mouse (usb-mouse)",
                KIND_NET => "network adapter (usb-ethernet)",
                KIND_HUB => "hub (the USB host driver)",
                KIND_MSC => "storage (usb-storage)",
                KIND_TABLET => "touchscreen or absolute pointer (usb-tablet)",
                KIND_AUDIO => "headset or speaker (usb-audio)",
                KIND_SERIAL => "serial cable (usb-serial)",
                KIND_CAM => "camera (usb-video)",
                KIND_BT => "Bluetooth controller (bt)",
                _ => continue, // being set up, or not used
            };
            out[n] = (dv.vid, dv.pid, what);
            n += 1;
        }
    }
    (out, n)
}

/// The controllers this driver runs, for `hwscan`.
pub(crate) fn controllers() -> ([(u8, u8, u8); NCTL], usize) {
    let mut out = [(0, 0, 0); NCTL];
    // SAFETY: single core, shell context; `list` reads the controllers the same way.
    unsafe {
        if OP == 0 {
            return (out, 0);
        }
        let Some(home) = on(CUR) else { return (out, 0) };
        let n = NRUN.max(1).min(NCTL);
        for (c, o) in out.iter_mut().enumerate().take(n) {
            select(c);
            *o = *addr_of!(PCI_AT);
        }
        select(home);
        (out, n)
    }
}

/// (a USB controller was set up, the devices it found and does not use, how many).
pub(crate) fn others() -> (bool, [(u16, u16, u8, u8); 8], usize) {
    // SAFETY: single core, shell context; plain copies of boot/hot-plug bookkeeping.
    unsafe { (READY, OTHERS, NOTHERS) }
}
static mut HUB_PORTS: u32 = 0;

// --- USB Ethernet (CDC-NCM, CDC-ECM or RNDIS): one adapter, bulk IN and OUT ---
const NRXB: usize = 4; // receive transfers kept queued on the bulk-IN ring
const NTXB: usize = 8; // transmit buffers, used round robin
const NRXQ: usize = 16; // received frames waiting for the network stack
const FRAME: usize = 2048;
#[repr(C, align(2048))]
struct Frame([u8; FRAME]);
static mut EPOUT: Ring<NXFER> = Ring([TRB0; NXFER]);
static mut PEPOUT: Prod = Prod { idx: 0, cycle: 1 };
static mut NETRX: [Frame; NRXB] = [const { Frame([0; FRAME]) }; NRXB];
static mut NETTX: [Frame; NTXB] = [const { Frame([0; FRAME]) }; NTXB];
static mut RXMAP: [u64; NRXB] = [0; NRXB]; // the TRB each receive buffer was queued in
static mut RXQ: [Frame; NRXQ] = [const { Frame([0; FRAME]) }; NRXQ];
static mut RXQLEN: [usize; NRXQ] = [0; NRXQ];
static mut RXQ_HEAD: usize = 0; // next frame to hand out
static mut RXQ_TAIL: usize = 0; // next free place
static mut NET_D: usize = usize::MAX; // the device index of the adapter
static mut NET_SLOT: u32 = 0;
static mut NET_PORT: u32 = 0;
static mut NET_DIN: u32 = 0;
static mut NET_DOUT: u32 = 0;
static mut NET_MAC: [u8; 6] = [0; 6];
static mut NET_TXN: u64 = 0; // frames queued for sending
static mut NET_TXDONE: u64 = 0; // ... and completed
static mut NET_RXN: u64 = 0;
static mut NET_RXDROP: u64 = 0;
static mut NET_RNDIS: bool = false; // RNDIS (a phone's USB tethering): each frame in a 44-byte PACKET_MSG
static mut NET_MPSO: u32 = 64; // the bulk-OUT packet size (an RNDIS message never ends on a multiple of it)
const RNDIS_HDR: usize = 44;
/// CDC-NCM (13 Nov 2026): the received blocks can each carry many frames, so NCM has its own receive buffers,
/// 8 KiB each (up to 5 full-size frames per block). A buffer's address is its DMA address (identity mapped, as
/// every xHCI buffer here); each starts on an 8 KiB line (Room8k), so it never crosses a 64 KiB boundary (one TRB
/// each, xHCI 4.11.7.1).
const NCM_IN: usize = 8192;
static mut NCMRX: Room8k<{ room8k(NRXB) }> = Room8k([0; room8k(NRXB)]);
/// NCM receive buffer `b` (NCM_IN bytes).
unsafe fn ncm_rx(b: usize) -> *mut u8 {
    at8k(addr_of!(NCMRX) as *const u8, b)
}
/// Set when the adapter speaks NCM: the device's checked block parameters and the receive block size asked for.
static mut NET_NCM: Option<(crate::cdcncm::Params, usize)> = None;
static mut NCM_SEQ: u16 = 0;
#[derive(Clone, Copy)]
struct NcmStats {
    blocks: u64,    // blocks received
    most: usize,    // most frames in one block
    bad: u64,       // blocks dropped whole (cdcncm::Bad)
    last_bad: Option<crate::cdcncm::Bad>,
    skipped: u64,   // table entries skipped
    dropped: u64,   // frames the receive queue had no room for
    stopped: u32,   // receive buffers taken out after an error completion (never handed back to the controller)
    sent: u64,      // blocks sent
}
impl NcmStats {
    const ZERO: NcmStats = NcmStats { blocks: 0, most: 0, bad: 0, last_bad: None, skipped: 0, dropped: 0, stopped: 0, sent: 0 };
}
static mut NCM_STATS: NcmStats = NcmStats::ZERO;
/// One enumerated HID boot device: its slot, interrupt-IN DCI and kind.
#[derive(Clone, Copy)]
struct Dev {
    slot: u32,
    dci: u32,
    kind: u8,
    port: u32, // the root port it hangs off (directly or behind a hub)
    ctl: usize, // the controller it is on (slot and port numbers are that controller's)
    route: u32, // its route string behind hubs (0 on a root port)
    vid: u16, // its vendor and product ids, from the device descriptor (for `hwscan`)
    pid: u16,
}
const NO_DEV: Dev = Dev { slot: 0, dci: 0, kind: 0, port: 0, ctl: 0, route: 0, vid: 0, pid: 0 };
static mut DEVS: [Dev; NDEV] = [NO_DEV; NDEV];
/// Root ports whose connection changed (port-status-change events), for `service`, per controller.
static mut PENDING: [u32; NCTL] = [0; NCTL];
/// F2: each controller's power-on wait (`settle`): ms waited, ports with a device, steady.
static mut SETTLE: [(u32, u32, bool); NCTL] = [(0, 0, false); NCTL];
/// F2: each controller's reset (`reset`): ms from HCRST to ready, halted before it, Intel host.
static mut RESET: [(u32, bool, bool); NCTL] = [(0, false, false); NCTL];
/// Each controller's Supported Protocol capabilities (xHCI 7.2): (major, minor revision in BCD,
/// first port, port count, protocol speed ID count), as the controller names its root ports.
static mut PROTO: [[(u8, u8, u8, u8, u8); 6]; NCTL] = [[(0, 0, 0, 0, 0); 6]; NCTL];
static mut PROTO_N: [usize; NCTL] = [0; NCTL];
/// Each protocol's own speed ID table (xHCI 7.2.1, the PSI dwords after the capability's first
/// four), the first 8 of its PSIC: PSIV 3:0, PSIE 5:4 (b/s, Kb/s, Mb/s, Gb/s), PLT 7:6, PFD 8,
/// LP 15:14 (0 SuperSpeed, 1 SuperSpeedPlus), PSIM 31:16.
static mut PSI: [[[u32; 8]; 6]; NCTL] = [[[0; 8]; 6]; NCTL];
/// Devices whose speed was read from a controller's own speed ID table, and `usbaddr psi` (a test
/// table as Intel hosts publish it put on controller 1, so QEMU can show the table path).
static mut PSI_USED: u32 = 0;
static mut PSI_TEST: bool = false;
/// Root ports (bit n = port n), per controller, whose device was tried and is not used, or did
/// not answer after one retry: left alone until it is unplugged (field item F1, HP ENVY 8 Nov
/// 2026: a device that failed addressing was reset and tried again forever, each reset's own
/// port-change event starting the next try).
static mut LEFT: [u32; NCTL] = [0; NCTL];
/// Per hub (by device index): the ports its status-change endpoint reported (bit n = port n),
/// for `service`; and what its ports need: (speed, TT hub slot, TT port, number of ports).
static mut HUBCHG: [u16; NDEV] = [0; NDEV];
static mut HUBI: [(u32, u32, u32, u32); NDEV] = [(0, 0, 0, 0); NDEV];
static mut HUB_WATCHED: u32 = 0; // hubs whose status-change endpoint runs (hot-plug behind them)
/// `service` is enumerating: the timer poll stands aside (the waits serve events meanwhile).
static mut BUSY: bool = false;
static mut PLUGS: u64 = 0;
static mut UNPLUGS: u64 = 0;
/// A controller that stopped working: the USBSTS value the timer poll saw with Host System Error
/// (bit 2) or Host Controller Error (bit 12) set, 0 while it is fine. xHCI 4.24.1: after either
/// the controller does nothing more until it is reset, so every USB device on it (keyboard,
/// stick) was lost until a restart. `service` resets it and finds its devices again, as Linux's
/// xhci_hc_died and usb_hc_died lead to a reset of the host.
static mut DEAD: [u32; NCTL] = [0; NCTL];
/// The last DEAD came from a test switch (`xhcireset fault` or `xhcireset noanswer`), named here.
static mut DEAD_TEST: &str = "";
/// DEAD bit for a command that got no answer while USBSTS looked fine (USBSTS bit 31 is reserved,
/// so it never comes from the controller). xHCI 4.6.1.2: every command ends with an event; Linux
/// counts one that does not come in 5 s as a dead host (xhci_handle_command_timeout).
const NO_ANSWER: u32 = 1 << 31;
/// `xhcireset noanswer`: the next command is put on the ring without ringing the doorbell.
static mut NOANSWER_TEST: bool = false;
/// Each controller's resets after a fatal error or `xhcireset`, and whether it was given up on
/// (it did not come back, or it failed again after RESTART_MAX resets).
static mut RESTARTS: [u32; NCTL] = [0; NCTL];
static mut GIVEN_UP: [bool; NCTL] = [false; NCTL];
const RESTART_MAX: u32 = 3;
/// Field item F2 (HP ENVY, Intel Sunrise Point: Address Device and GET_DESCRIPTOR ending with
/// completion code 4, a USB transaction error). Two ways to give a new device its address:
/// plain (Address Device sends SET_ADDRESS at once, then the descriptor is read) and, as Linux's
/// "new scheme" and Windows do, descriptor first (Address Device with BSR sets up the slot
/// without SET_ADDRESS, the device descriptor is read (64 bytes asked) at the default
/// address, then a second Address Device gives the address). A root-port device that does not
/// answer (on a root port or a hub's port) is tried three times, the second try the other way.
/// `usbaddr bsr|plain` picks the first way (plain unless told).
static mut BSR_FIRST: bool = false;
/// The way `enumerate` uses now (set by `attach` for each try; hubs' devices use BSR_FIRST).
static mut BSR_NOW: bool = false;
static mut ADDR_PLAIN: u32 = 0; // devices whose descriptor was read, addressed plain
static mut ADDR_BSR: u32 = 0; // ... descriptor first
static mut ADDR_RETRIED: u32 = 0; // devices that answered only on a later try
static mut ADDR_NONE: u32 = 0; // devices that answered on no try
/// A test switch (`usbaddr fail N`): the next N addressing tries end at the first descriptor
/// read as if the device had sent nothing back (completion code 4, a USB transaction error),
/// so the retry ladder can be seen working where every device answers (QEMU).
static mut FAIL_NEXT: u32 = 0;
static mut FAILED_ON_PURPOSE: u32 = 0;
/// F2: after Address Device has sent SET_ADDRESS, the device has 2 ms to take its new address
/// (USB 2.0 9.2.6.3); a request sent sooner goes to an address nobody answers yet, which a real
/// xHCI ends with completion code 4. Linux waits 10 ms ("some device hardware wants it"), so do
/// we. QEMU's devices take the address at once, so this was never seen there.
const SET_ADDRESS_MS: u64 = 10;
/// F2: the first descriptor read before the address (BSR) asks for 64 bytes, as Windows and
/// Linux's new scheme do (some devices misbehave on an 8-byte read); a short answer is fine.
const FIRST_READ_BSR: u16 = 64;
static mut ADDR_WAITS: u32 = 0; // SET_ADDRESS recovery waits made
static mut LAST_MPS0: u32 = 0; // the EP0 packet size the last device's first read gave
/// The last completion code a command or control request ended with other than success (0:
/// none since boot, or a time-out), and which addressing step was running.
static mut LAST_CC: u32 = 0;
static mut ADDR_STAGE: &str = "";
/// The error that sent a root-port device to its next try (the last such), for `lsusb`.
static mut ADDR_ERR: (u32, &str) = (0, "");

/// A free device entry (never used, or freed by an unplug).
fn free_dev() -> Option<usize> {
    // SAFETY: single core; read-only look at the table.
    unsafe { (0..NDEV).find(|&d| DEVS[d].slot == 0) }
}
static mut NDEVS: usize = 0;
static mut KBD_SLOT: u32 = 0;
static mut KBD_PORT: u32 = 0;
/// Caps Lock (13 Nov 2026): a USB keyboard's Caps Lock key used to do nothing. It now turns
/// letters to capitals (Shift then gives small letters, as on other systems), and the keyboard's
/// own Caps Lock light is set with a HID SET_REPORT on its interface. The keyboard's lights are
/// the host's to set; the request is sent from `service` (the idle loop), never from the timer
/// interrupt. Counters for `lsusb`: lights set and requests the keyboard refused.
static mut KBD_IFACE: u16 = 0;
static mut CAPS_LOCK: bool = false;
static mut LED_WANT: bool = false;
static mut LED_SETS: u32 = 0;
static mut LED_FAILS: u32 = 0;
static mut MOUSE_SLOT: u32 = 0;
static mut MOUSE_PORT: u32 = 0;
static mut READY: bool = false;
// the pointer, in shared-framebuffer pixels (640x480), starting in the middle
static mut MX: i32 = (crate::framebuffer::WIDTH / 2) as i32;
static mut MY: i32 = (crate::framebuffer::HEIGHT / 2) as i32;
static mut MBUTTONS: u8 = 0;
static mut MREPORTS: u64 = 0;
static mut MCLICKS: u64 = 0;
/// Acceleration of the USB mouse (9 Nov 2026), through ps2::accelerate's curve: when the
/// report before came, sixteenths carried over, reports sped up. A mouse does not say its
/// resolution in its boot reports, so 400 dpi (16 counts/mm, the low end of mice sold) is
/// taken: a faster mouse reads as faster and is sped up a little sooner.
static mut M_LAST: u64 = 0;
static mut M_REM: (i32, i32) = (0, 0);
static mut M_SPED: u64 = 0;
const MOUSE_PER_MM: u32 = 16;
/// The Tonga window being dragged by its title bar (left button held), if any.
static mut DRAG: Option<usize> = None;
static mut DRAGS: u64 = 0;
static mut LAST_KEYS: [u8; 6] = [0; 6];
static mut LAST_MODS: u8 = 0;
static mut SUPER_COMBO: bool = false;

/// A key that is a desktop shortcut while the desktop is shown (HID usages): Alt+Tab,
/// Alt+F4, Super+arrows, Esc.
fn desk_key(k: u8, alt: bool, sup: bool) -> Option<u32> {
    use crate::desk::*;
    Some(match k {
        0x2B if alt => KEY_ALT_TAB,
        0x3D if alt => KEY_ALT_F4,
        0x4F if sup => KEY_SUPER_RIGHT,
        0x50 if sup => KEY_SUPER_LEFT,
        0x51 if sup => KEY_SUPER_DOWN,
        0x52 if sup => KEY_SUPER_UP,
        0x29 if !alt && !sup => KEY_ESC,
        // the keyboard walk (afriui.md 14): arrows and Enter while the Circle is open,
        // Super+1..4 for the Spine's apps
        0x52 if !alt && !sup && walking() => KEY_UP,
        0x51 if !alt && !sup && walking() => KEY_DOWN,
        0x50 if !alt && !sup && walking() => KEY_LEFT,
        0x4F if !alt && !sup && walking() => KEY_RIGHT,
        0x28 | 0x58 if !alt && !sup && walking() => KEY_ENTER,
        0x1E..=0x21 if sup => KEY_SUPER_1 + (k - 0x1E) as u32,
        _ => return None,
    })
}
/// Key repeat (a USB boot keyboard leaves it to the host): the key held, its character, and
/// the TSC time it next repeats at. 500 ms before the first repeat, then about 30 a second.
static mut HELD: Option<(u8, u8)> = None;
static mut REPEAT_AT: u64 = 0;
static mut REPEATS: u64 = 0;
static mut REPORTS: u64 = 0;
static mut KEYS: u64 = 0;

fn r32(a: u64) -> u32 {
    // SAFETY: an address inside the mapped xHCI BAR.
    unsafe { read_volatile(a as *const u32) }
}
fn w32(a: u64, v: u32) {
    // SAFETY: as r32.
    unsafe { write_volatile(a as *mut u32, v) }
}
fn w64(a: u64, v: u64) {
    w32(a, v as u32);
    w32(a + 4, (v >> 32) as u32);
}
fn op(off: u64) -> u64 {
    // SAFETY: scalar read of boot-time state.
    unsafe { OP + off }
}
fn portsc(p: u32) -> u64 {
    op(0x400 + 0x10 * (p as u64 - 1))
}
fn doorbell(slot: u32, target: u32) {
    fence(Ordering::SeqCst);
    // SAFETY: scalar read.
    w32(unsafe { DB } + 4 * slot as u64, target);
}

/// Poll `done` for up to `ms` milliseconds of real time. A count of polls ran from a fraction of
/// a second to seconds, depending on the CPU and on how slow the register reads were.
fn wait(ms: u64, mut done: impl FnMut() -> bool) -> bool {
    let t0 = crate::timer::cycles();
    loop {
        if done() {
            return true;
        }
        if past(t0, ms) {
            return false;
        }
        core::hint::spin_loop();
    }
}

/// True once `ms` milliseconds have gone by since the cycle count `t0`.
fn past(t0: u64, ms: u64) -> bool {
    crate::timer::cycles().wrapping_sub(t0) > crate::timer::hz().max(1).saturating_mul(ms) / 1000
}

/// The limits, in milliseconds. HANDOFF_MS: the firmware letting go of the controller (Linux
/// waits 1 s). START_MS: HCHalted clearing after Run (xHCI 5.4.2: 16 microframes, 2 ms).
/// PORT_RESET_MS: a port reset finishing (PRC; Linux gives a hub port 800 ms). CONTROL_MS: a
/// control transfer (USB 2.0 9.2.6.4: 5 s for a standard request). BULK_MS: one transfer of a
/// stick's SCSI command (30 s, the SCSI disk timeout Linux uses).
const HANDOFF_MS: u64 = 1_000;
const START_MS: u64 = 100;
const PORT_RESET_MS: u64 = 800;
const CONTROL_MS: u64 = 5_000;
const BULK_MS: u64 = 30_000;

/// Put one TRB on a producer ring, inserting the Link TRB (cycle toggle) at the end.
unsafe fn push(ring: *mut Trb, n: usize, p: &mut Prod, param: u64, status: u32, control: u32) {
    push_held(ring, n, p, param, status, control, false);
}

/// `push`, optionally writing the TRB with the WRONG cycle bit (`hold`), so the controller cannot
/// start a multi-TRB TD before the rest of it is written; `release` then flips it (xHCI 4.11.2.2).
/// Returns the TRB's address.
unsafe fn push_held(ring: *mut Trb, n: usize, p: &mut Prod, param: u64, status: u32, control: u32, hold: bool) -> *mut Trb {
    let at = ring.add(p.idx);
    write_volatile(at, Trb { param, status, control: control | (p.cycle ^ hold as u32) });
    p.idx += 1;
    if p.idx == n - 1 {
        write_volatile(ring.add(n - 1), Trb { param: ring as u64, status: 0, control: T_LINK << 10 | 1 << 1 | p.cycle });
        p.idx = 0;
        p.cycle ^= 1;
    }
    at
}

/// Hand a held TRB to the controller (flip its cycle bit to the valid value).
unsafe fn release(at: *mut Trb) {
    fence(Ordering::SeqCst);
    let c = read_volatile(addr_of!((*at).control));
    write_volatile(addr_of_mut!((*at).control), c ^ 1);
}

/// The next event, if the controller has written one (and the dequeue pointer moved on).
unsafe fn next_event() -> Option<Trb> {
    let ev = addr_of!(EVT[CUR].0) as *const Trb;
    // The cycle bit first, then the rest (14 Nov 2026, the release build's lost stick answer): the controller
    // writes an event's parameter and status before its control word, so reading the whole TRB in one go could take
    // a new control word (the cycle bit says "new") with the previous lap's parameter. wait_xfer then saw an event
    // naming another TRB, dropped it as stale, and waited 30 s for an answer that had come.
    let p = ev.add(EV_IDX);
    let control = read_volatile(addr_of!((*p).control));
    if control & 1 != EV_CYCLE {
        return None;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
    let t = Trb { param: read_volatile(addr_of!((*p).param)), status: read_volatile(addr_of!((*p).status)), control };
    EV_IDX += 1;
    if EV_IDX == NEV {
        EV_IDX = 0;
        EV_CYCLE ^= 1;
    }
    w64(RT + 0x20 + 0x18, ev.add(EV_IDX) as u64 | 1 << 3); // ERDP, clear EHB
    Some(t)
}

/// Wait up to 5 seconds of real time for an event of `ty` (others, such as port changes, are
/// passed over). A spin count ran from a fraction of a second to seconds, depending on the CPU,
/// and a command that runs out of it now costs a controller reset.
unsafe fn wait_event(ty: u32) -> Option<Trb> {
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    loop {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == ty {
                return Some(t);
            }
            handle(t); // another device's transfer: served, not dropped
        }
        if crate::timer::cycles().wrapping_sub(t0) / (hz / 1000).max(1) > 5000 {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// A command on the current controller got no completion event in 5 seconds. The controller is
/// marked as stopped, so the next idle moment resets it and finds its devices again, as after a
/// fatal error (`service`); before, every later command waited in vain too.
unsafe fn no_answer(control: u32) {
    let st = r32(op(0x04));
    crate::kprintln!("[xhci] controller {}: command type {} got no answer in 5 s (USBSTS {:#x})", CUR + 1, (control >> 10) & 0x3F, st);
    LAST_CC = 0;
    if DEAD[CUR] == 0 && !GIVEN_UP[CUR] {
        DEAD[CUR] = st | NO_ANSWER;
    }
}

/// Wait for the transfer event of endpoint 0 of `slot` for the request whose TRBs are `td`
/// (setup, data, status; the data one 0 when there is none); every other event is served as the
/// timer poll would serve it. Dropping them lost a running device's report (its transfer was
/// never re-queued) whenever a second device enumerated. An endpoint 0 event of the slot for
/// another TRB (the answer to a request whose wait gave up) is set aside and counted: before,
/// it ended this wait with that request's status.
unsafe fn wait_control(slot: u32, td: [u64; 3]) -> Option<Trb> {
    let t0 = crate::timer::cycles();
    while !past(t0, CONTROL_MS) {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == T_EV_TRANSFER && t.control >> 24 == slot && (t.control >> 16) & 0x1F == 1 {
                let at = t.param & !0xF;
                if at != 0 && td.contains(&at) {
                    CTL_TOOK = at;
                    return Some(t);
                }
                CTL_STALE += 1;
                continue;
            }
            handle(t);
        }
        core::hint::spin_loop();
    }
    None
}

/// One command on the command ring; its completion event, if it succeeded.
unsafe fn command(param: u64, control: u32) -> Option<Trb> {
    command_st(param, 0, control)
}

/// `command` with its status field set (Set TR Dequeue Pointer names a stream there).
unsafe fn command_st(param: u64, status: u32, control: u32) -> Option<Trb> {
    push(addr_of_mut!(CMD[CUR].0) as *mut Trb, NCMD, &mut *addr_of_mut!(PCMD), param, status, control);
    if NOANSWER_TEST {
        NOANSWER_TEST = false; // the test: the controller is never told about this command
    } else {
        doorbell(0, 0);
    }
    let Some(ev) = wait_event(T_EV_COMMAND) else {
        no_answer(control);
        return None;
    };
    if ev.status >> 24 != CC_SUCCESS {
        crate::kprintln!("[xhci] command type {} failed (completion code {})", (control >> 10) & 0x3F, ev.status >> 24);
        LAST_CC = ev.status >> 24;
        return None;
    }
    Some(ev)
}

/// A control transfer on device `d`'s endpoint 0 (`len` bytes into/out of DBUF).
unsafe fn control(d: usize, rtype: u8, req: u8, value: u16, index: u16, len: u16) -> bool {
    let slot = DEVS[d].slot;
    let ring = addr_of_mut!(EP0[d].0) as *mut Trb;
    let p = &mut (*addr_of_mut!(PEP0))[d];
    let input = rtype & 0x80 != 0;
    let setup = rtype as u64 | (req as u64) << 8 | (value as u64) << 16 | (index as u64) << 32 | (len as u64) << 48;
    let trt = if len == 0 { 0 } else if input { 3 } else { 2 };
    let first = push_held(ring, NXFER, p, setup, 8, T_SETUP << 10 | 1 << 6 | trt << 16, true);
    let mut data = 0;
    if len > 0 {
        data = push_held(ring, NXFER, p, addr_of!(DBUF) as u64, len as u32, T_DATA << 10 | (input as u32) << 16, false) as u64;
    }
    let status_in = len == 0 || !input;
    let status = push_held(ring, NXFER, p, 0, 0, T_STATUS << 10 | 1 << 5 | (status_in as u32) << 16, false) as u64;
    release(first); // the whole TD is written: only now may the controller see its first TRB
    doorbell(slot, 1);
    CTL_WANT = status;
    match wait_control(slot, [first as u64, data, status]) {
        Some(ev) if matches!(ev.status >> 24, CC_SUCCESS | CC_SHORT) => true,
        Some(ev) => {
            if !CTL_QUIET {
                crate::kprintln!("[xhci] control request {:#04x} failed (completion code {})", req, ev.status >> 24);
            }
            LAST_CC = ev.status >> 24;
            ep0_recover(d);
            false
        }
        None => {
            crate::kprintln!("[xhci] control request {:#04x} timed out", req);
            LAST_CC = 0;
            ep0_recover(d);
            false
        }
    }
}

/// After a failed control request: an endpoint 0 the controller halted (a STALL, babble or a
/// transaction error, xHCI 4.10.2.1) takes no more requests until it is reset. Reset Endpoint,
/// then its dequeue pointer is moved past the failed request to where the next one will be
/// queued (Set TR Dequeue Pointer, xHCI 4.6.10); the device itself clears a control STALL at the
/// next SETUP (USB 2.0 8.5.3.4). Before, every later request to the device timed out, so one
/// refused class or vendor request cost the whole device. A running endpoint 0 (a request that
/// timed out) is left as it is: its late answer is set aside by wait_control.
unsafe fn ep0_recover(d: usize) {
    let slot = DEVS[d].slot;
    if read_volatile(ctx(addr_of_mut!(OUTCTX[d].0) as *mut u8, 1)) & 7 != 2 {
        return;
    }
    CTL_RESETS += 1;
    command(0, T_RESET_EP << 10 | slot << 24 | 1 << 16);
    let p = *addr_of!(PEP0[d]);
    let deq = addr_of!(EP0[d].0) as u64 + 16 * p.idx as u64;
    command(deq | p.cycle as u64, T_SET_TR_DEQ << 10 | slot << 24 | 1 << 16);
}

fn ctx(base: *mut u8, i: usize) -> *mut u32 {
    // SAFETY: an index into one of the 4 KiB context buffers.
    unsafe { base.add(i * CSZ) as *mut u32 }
}

/// Asili v0 BINDS (pc/hwdb.rs): `init` takes the controllers this table matches.
pub(crate) const BINDS: &[crate::hwdb::Part] = &[crate::hwdb::pci_class(0x0C_03_30, 0xFF_FF_FF, "xHCI USB controller")];
/// The older USB 2 controllers (EHCI): not driven, only looked for by `intel_switch` (port routing).
const EHCI: &[crate::hwdb::Part] = &[crate::hwdb::pci_class(0x0C_03_20, 0xFF_FF_FF, "EHCI USB 2 controller")];

/// Every PCI function `parts` matches on buses 0-7, in bus order, with its vendor:device id; at most 4.
fn pci_find(parts: &'static [crate::hwdb::Part]) -> ([(u8, u8, u8, u32); 4], usize) {
    let mut out = [(0, 0, 0, 0); 4];
    let mut n = 0;
    pci::find_by(8, |b, d, f, id| {
        if crate::hwdb::pci_part(parts, b, d, f, id).is_some() && n < 4 {
            out[n] = (b, d, f, id);
            n += 1;
        }
        false // keep walking: every match, in bus order
    });
    (out, n)
}

/// BAR0 of the controller at b:d.f, mapped, with memory decoding on; None if it has none.
fn bar0(b: u8, d: u8, f: u8) -> Option<u64> {
    let bar = pci::bar_mem(b, d, f, 0);
    if bar == 0 || !mmu::map_mmio(bar, 0x10000) {
        return None;
    }
    pci::enable(b, d, f, 0x406); // memory decoding + bus mastering, INTx off (polled)
    Some(bar)
}

/// Take the controller from the firmware (xHCI 4.22.1): real PCs run a USB-keyboard emulation
/// in SMM that keeps the controller until the OS claims it.
fn handoff(bar: u64) {
    let hcc1 = r32(bar + 0x10);
    let mut xecp = ((hcc1 >> 16) & 0xFFFF) as u64 * 4;
    let mut guard = 0;
    while xecp != 0 && xecp < 0x10000 && guard < 64 {
        guard += 1;
        let at = bar + xecp;
        let v = r32(at);
        if v & 0xFF == 1 {
            if v & 1 << 16 != 0 {
                w32(at, v | 1 << 24); // OS owned
                if !wait(HANDOFF_MS, || r32(at) & 1 << 16 == 0) {
                    crate::kprintln!("[xhci] the firmware did not release the controller; taking it anyway");
                    w32(at, r32(at) & !(1 << 16));
                }
                crate::kprintln!("[xhci] controller taken over from the firmware (USB legacy support)");
            }
            // no more SMIs: clear the SMI enables, acknowledge the latched ones
            w32(at + 4, (r32(at + 4) & !0xE011) | 0xE000_0000);
        }
        let next = ((v >> 8) & 0xFF) as u64;
        if next == 0 {
            break;
        }
        xecp += next * 4;
    }
}

/// Read the controller's Supported Protocol capabilities (xHCI 7.2, extended capability ID 2):
/// which root ports are USB 2 and which USB 3, and whether the controller defines its own
/// protocol speed IDs (PSIC). Stored for `kinds_line`.
unsafe fn protocols(bar: u64, c: usize) {
    PROTO_N[c] = 0;
    let hcc1 = r32(bar + 0x10);
    let mut xecp = ((hcc1 >> 16) & 0xFFFF) as u64 * 4;
    let mut guard = 0;
    while xecp != 0 && xecp < 0x10000 && guard < 64 {
        guard += 1;
        let v = r32(bar + xecp);
        if v & 0xFF == 2 && PROTO_N[c] < 6 {
            let w2 = r32(bar + xecp + 8);
            PROTO[c][PROTO_N[c]] = ((v >> 24) as u8, (v >> 16) as u8, w2 as u8, (w2 >> 8) as u8, (w2 >> 28) as u8);
            for k in 0..((w2 >> 28) as usize).min(8) {
                PSI[c][PROTO_N[c]][k] = r32(bar + xecp + 16 + 4 * k as u64);
            }
            PROTO_N[c] += 1;
        }
        let next = ((v >> 8) & 0xFF) as u64;
        if next == 0 {
            break;
        }
        xecp += next * 4;
    }
}

/// One line naming controller `c`'s root ports by kind, e.g. "port kinds: USB 2.0 on ports 1-4,
/// USB 3.0 on ports 5-8", with any ports no capability names. On a real PC this says which
/// ports a laptop's built-in devices can be on (field item F2), and a USB 3.1/3.2 range means
/// SuperSpeedPlus devices (speed IDs 5 to 7) can appear there.
unsafe fn kinds_line(c: usize, prefix: &str) {
    let ports = if c == CUR { PORTS } else { BANKS[c].ports };
    crate::kprint!("{}port kinds:", prefix);
    let mut named = 0u32;
    for i in 0..PROTO_N[c] {
        let (maj, min, first, n, psic) = PROTO[c][i];
        let last = first as u32 + n as u32 - 1;
        crate::kprint!(
            "{} USB {}.{} on port{} {}",
            if i == 0 { "" } else { "," }, maj, min >> 4, if n == 1 { "" } else { "s" }, first
        );
        if n > 1 {
            crate::kprint!("-{}", last);
        }
        if psic > 0 {
            crate::kprint!(" ({} speed IDs of its own:", psic);
            for k in 0..(psic as usize).min(8) {
                let e = PSI[c][i][k];
                crate::kprint!(" {} {}", e & 0xF, speed_name(psi_class(maj, e)));
            }
            crate::kprint!(")");
        }
        named += n as u32;
    }
    if PROTO_N[c] == 0 {
        crate::kprint!(" not named by the controller");
    } else if named < ports {
        crate::kprint!(", {} of {} not named", ports - named, ports);
    }
    crate::kprintln!("");
}

/// One entry of a controller's own speed ID table (PSI dword) as the default speed code the
/// driver works with (xHCI 7.2.2.1.1): on a USB 2 port by its bit rate (1.5 Mb/s or less low,
/// 12 Mb/s or less full, else high); on a USB 3 port SuperSpeed (4), or SuperSpeedPlus (5) when
/// the link protocol says so or the rate is over 5 Gb/s. As Linux's xhci-hub reads PSI.
fn psi_class(major: u8, e: u32) -> u32 {
    let mut rate = (e >> 16) as u64;
    for _ in 0..(e >> 4) & 3 {
        rate *= 1000;
    }
    if major < 3 {
        if rate <= 1_500_000 { 2 } else if rate <= 12_000_000 { 1 } else { 3 }
    } else if (e >> 14) & 3 == 1 || rate > 5_000_000_000 {
        5
    } else {
        4
    }
}

/// The speed code of root port `port`'s device on controller `c` from the port's speed ID `id`:
/// when the port's protocol defines its own IDs (PSIC), the matching table entry says what the
/// ID means; otherwise (or with no match) the ID is a default one and taken as it is.
unsafe fn port_speed(c: usize, port: u32, id: u32) -> u32 {
    for i in 0..PROTO_N[c] {
        let (maj, _, first, n, psic) = PROTO[c][i];
        if port < first as u32 || port >= first as u32 + n as u32 {
            continue;
        }
        for k in 0..(psic as usize).min(8) {
            let e = PSI[c][i][k];
            if e & 0xF == id && id != 0 {
                PSI_USED += 1;
                return psi_class(maj, e);
            }
        }
    }
    id
}

/// `usbaddr psi`: controller 1 gets a speed ID table as Intel hosts publish it (USB 2: 1 full
/// 12 Mb/s, 2 low 1500 Kb/s, 3 high 480 Mb/s; USB 3: 4 SuperSpeed 5 Gb/s, 5 SuperSpeedPlus 10
/// Gb/s), each entry is checked against what it must mean, and devices plugged in after it take
/// their speed through the table.
unsafe fn psi_test() {
    const T2: [(u32, u32); 3] = [(0x000C_0021, 1), (0x05DC_0012, 2), (0x01E0_0023, 3)];
    const T3: [(u32, u32); 2] = [(0x0005_0134, 4), (0x000A_4135, 5)];
    let (mut ok, mut n) = (0u32, 0u32);
    for i in 0..PROTO_N[0] {
        let t: &[(u32, u32)] = if PROTO[0][i].0 < 3 { &T2 } else { &T3 };
        for (k, &(e, want)) in t.iter().enumerate() {
            PSI[0][i][k] = e;
            ok += (psi_class(PROTO[0][i].0, e) == want) as u32;
        }
        PROTO[0][i].4 = t.len() as u8;
        n += t.len() as u32;
    }
    PSI_TEST = true;
    crate::kprintln!(
        "usb: test: controller 1 now has its own speed ID table; {} of {} entries read as expected",
        ok, n
    );
    kinds_line(0, "  ");
}

/// The name of a USB speed as a port reports it (PORTSC Port Speed, xHCI 7.2.2.1.1 default
/// speed IDs): 1 full, 2 low, 3 high, 4 SuperSpeed (5 Gb/s); 5 to 7 are SuperSpeedPlus (USB 3.1
/// and 3.2, 10 or 20 Gb/s) on the USB-C ports of newer PCs.
fn speed_name(speed: u32) -> &'static str {
    match speed {
        1 => "full",
        2 => "low",
        3 => "high",
        4 => "super",
        5..=7 => "super plus",
        _ => "?",
    }
}

/// Intel 7/8/9-series chipsets (2012-2015) have an EHCI (USB 2) controller beside the xHCI, and
/// each port is wired to one of them by the XUSB2PR/USB3_PSSEN switches; the firmware may leave
/// them on EHCI, which this kernel does not drive, so a keyboard there would be invisible. As
/// Linux's usb_enable_intel_xhci_ports: on an Intel xHCI, when an Intel EHCI exists (not on a
/// Sony VAIO 104d:90a8, which cannot switch), every port the routing masks allow (USB3PRM 0xDC
/// -> USB3_PSSEN 0xD8, USB2PRM 0xD4 -> XUSB2PR 0xD0) is moved to the xHCI. Says what moved.
fn intel_switch(b: u8, d: u8, f: u8, say: bool) {
    if cfg32(b, d, f, 0) & 0xFFFF != 0x8086 || cfg32(b, d, f, 0x2C) == 0x90A8_104D {
        return;
    }
    let (ehci, n) = pci_find(EHCI);
    if !ehci[..n].iter().any(|e| e.3 & 0xFFFF == 0x8086) {
        return;
    }
    let (ss_before, hs_before) = (cfg32(b, d, f, 0xD8), cfg32(b, d, f, 0xD0));
    cfg_write32(b, d, f, 0xD8, cfg32(b, d, f, 0xDC));
    cfg_write32(b, d, f, 0xD0, cfg32(b, d, f, 0xD4));
    if say {
        crate::kprintln!(
            "[xhci] Intel port switch: USB 3 ports {:#x} -> {:#x}, USB 2 ports {:#x} -> {:#x} on this controller (were on the EHCI one)",
            ss_before, cfg32(b, d, f, 0xD8), hs_before, cfg32(b, d, f, 0xD0)
        );
    }
}

/// Stop and reset the controller whose operational registers start at `op`, as Linux's
/// xhci_halt + xhci_reset: Run/Stop cleared and up to 20 ms for HCHalted, then HCRST; 1 ms
/// before the next register read (Linux XHCI_INTEL_HOST: on Intel controllers a read right
/// after HCRST can, rarely, hang the whole PC; done on every host here, it costs 1 ms), then
/// HCRST and CNR clear within 10 s. Returns (ms from HCRST to ready, was halted), or None.
fn reset(op: u64) -> Option<(u32, bool)> {
    w32(op, r32(op) & !1);
    let mut halted = r32(op + 0x04) & 1 != 0;
    for _ in 0..20 {
        if halted {
            break;
        }
        crate::timer::busywait_ms(1);
        halted = r32(op + 0x04) & 1 != 0;
    }
    w32(op, 1 << 1);
    crate::timer::busywait_ms(1);
    let mut t = 1u32;
    while r32(op) & 1 << 1 != 0 || r32(op + 0x04) & 1 << 11 != 0 {
        if t >= 10_000 {
            return None;
        }
        crate::timer::busywait_ms(1);
        t += 1;
    }
    Some((t, halted))
}

/// A look at a controller that is not driven yet: taken from the firmware, ports switched (Intel),
/// reset, ports powered; returns how many ports have a device connected. Left halted.
fn probe(b: u8, d: u8, f: u8) -> u32 {
    let Some(bar) = bar0(b, d, f) else { return 0 };
    handoff(bar);
    intel_switch(b, d, f, false);
    let op = bar + (r32(bar) & 0xFF) as u64;
    if reset(op).is_none() {
        return 0;
    }
    let ports = r32(bar + 0x04) >> 24;
    let pc = |p: u32| op + 0x400 + 0x10 * (p as u64 - 1);
    for p in 1..=ports {
        let sc = r32(pc(p));
        if sc & 1 << 9 == 0 {
            w32(pc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | 1 << 9);
        }
    }
    crate::timer::busywait_ms(150); // power good, attach debounce
    (1..=ports).filter(|&p| r32(pc(p)) & 1 != 0).count() as u32
}

/// Bring up the controller(s) and the devices plugged into them. A PC with several xHCI
/// controllers (AMD boards; Intel laptops, whose USB-C/Thunderbolt controller at 00:0d.0 comes
/// before the one at 00:14.0 that has the USB-A ports) gets every one with a device plugged in
/// driven (a keyboard on one, a USB stick on another), each with its own rings. A controller
/// with nothing in it at power-on runs too, so a device plugged into ANY port later is seen
/// (hot-plug on every controller, as Linux does).
pub fn init() {
    let (all, n) = pci_find(BINDS);
    if n == 0 {
        return;
    }
    let drive = [true; 4];
    if n > 1 {
        let mut found = [0u32; 4];
        for i in 0..n {
            found[i] = probe(all[i].0, all[i].1, all[i].2);
        }
        for i in 0..n {
            let (b, d, f, id) = all[i];
            crate::kprintln!(
                "[xhci] {} of {}: {:02x}:{:02x}.{} ({:04x}:{:04x}), {} device(s) connected <- used{}",
                i + 1, n, b, d, f, id & 0xFFFF, id >> 16, found[i],
                if found[i] > 0 { "" } else { " (nothing yet: for hot-plug)" }
            );
        }
    }
    // SAFETY: boot, single core; BUSY keeps the timer poll away until every controller is up
    // (the waits inside serve the events meanwhile).
    unsafe {
        BUSY = true;
        for i in 0..n {
            if !drive[i] {
                continue;
            }
            select(NRUN);
            let (b, d, f, _) = all[i];
            if start(b, d, f) {
                NRUN += 1;
            } else {
                // not running: this bank is used again for the next one
                (PCMD, EV_IDX, EV_CYCLE, OP, CONNECTED) = (Prod { idx: 0, cycle: 1 }, 0, 1, 0, 0);
            }
        }
        if NRUN > 0 {
            select(0);
        }
        if NRUN > 1 {
            crate::kprintln!("[xhci] {} controllers driven", { NRUN });
        }
        if OP != 0 {
            if KBD_SLOT == 0 {
                crate::kprintln!("[xhci] {} device(s) connected, no USB keyboard", total_connected());
            }
            if MOUSE_SLOT == 0 {
                crate::kprintln!("[xhci] no USB mouse");
            }
        }
        BUSY = false;
    }
}

/// Devices connected on every controller driven.
unsafe fn total_connected() -> u32 {
    (0..NRUN).map(|c| if c == CUR { CONNECTED } else { BANKS[c].connected }).sum()
}

/// Run the controller at b:d.f as controller CUR: rings, ports, the devices on them. False if
/// it could not be started.
fn start(b: u8, d: u8, f: u8) -> bool {
    let Some(bar) = bar0(b, d, f) else {
        crate::kprintln!("[xhci] controller at {:02x}:{:02x}.{} has no usable BAR0", b, d, f);
        return false;
    };
    // SAFETY: boot, single core; init keeps the timer poll away (BUSY) while this runs.
    unsafe {
        let c = CUR;
        core::ptr::write_bytes(addr_of_mut!(CMD[c]) as *mut u8, 0, core::mem::size_of::<Ring<NCMD>>());
        core::ptr::write_bytes(addr_of_mut!(EVT[c]) as *mut u8, 0, core::mem::size_of::<Ring<NEV>>());
        core::ptr::write_bytes(addr_of_mut!(DCBAA[c]) as *mut u8, 0, core::mem::size_of::<Dcbaa>());
        PCI_AT = (b, d, f);
        let caplen = r32(bar) & 0xFF;
        let hcs1 = r32(bar + 0x04);
        let hcs2 = r32(bar + 0x08);
        let hcc1 = r32(bar + 0x10);
        OP = bar + caplen as u64;
        DB = bar + (r32(bar + 0x14) & !3) as u64;
        RT = bar + (r32(bar + 0x18) & !0x1F) as u64;
        CSZ = if hcc1 & 1 << 2 != 0 { 64 } else { 32 };
        let max_slots = (hcs1 & 0xFF).min(255);
        PORTS = hcs1 >> 24;
        handoff(bar);
        protocols(bar, c);
        intel_switch(b, d, f, true);
        // stop, reset, wait for the controller to be ready
        let intel = cfg32(b, d, f, 0) & 0xFFFF == 0x8086;
        let Some((ms, halted)) = reset(op(0x00)) else {
            crate::kprintln!("[xhci] controller did not come out of reset in 10 s");
            return false;
        };
        RESET[c] = (ms, halted, intel);
        crate::kprintln!(
            "[xhci] reset: {}ready {} ms after HCRST (first read after 1 ms, as Linux on Intel hosts{})",
            if halted { "halted, " } else { "did not halt in 20 ms, reset anyway; " },
            ms, if intel { "; this is one" } else { "" }
        );
        w32(op(0x38), max_slots); // CONFIG: slots enabled
        if r32(op(0x08)) & 1 == 0 {
            crate::kprintln!("[xhci] controller does not take 4 KiB pages (PAGESIZE {:#x}); not used", r32(op(0x08)));
            return false;
        }
        // scratchpad buffers the controller may ask for
        let nscratch = ((hcs2 >> 21) & 0x1F) << 5 | (hcs2 >> 27) & 0x1F;
        if nscratch > 0 {
            let arr = addr_of_mut!(SCRATCH_ARR[c].0) as *mut u64;
            for i in 0..(nscratch as usize).min(256) {
                // a reset controller (`restart`) keeps the pages it was given at boot
                let Some(page) = (if *arr.add(i) != 0 { Some(*arr.add(i) as usize) } else { crate::frame::alloc() }) else {
                    crate::kprintln!("[xhci] no frame for scratchpad buffer {}", i);
                    return false;
                };
                core::ptr::write_bytes(page as *mut u8, 0, 4096);
                *arr.add(i) = page as u64;
            }
            DCBAA[c].0[0] = arr as u64;
        }
        w64(op(0x30), addr_of!(DCBAA[c]) as u64);
        w64(op(0x18), addr_of!(CMD[c]) as u64 | 1); // CRCR, ring cycle state 1
        ERST[c].0 = [addr_of!(EVT[c]) as u64, NEV as u64];
        w32(RT + 0x20 + 0x08, 1); // ERSTSZ
        w64(RT + 0x20 + 0x18, addr_of!(EVT[c]) as u64); // ERDP
        w64(RT + 0x20 + 0x10, addr_of!(ERST[c]) as u64); // ERSTBA (last: it starts the ring)
        w32(op(0x00), 1); // run, interrupts off (polled)
        if !wait(START_MS, || r32(op(0x04)) & 1 == 0) {
            crate::kprintln!("[xhci] controller did not start");
            return false;
        }
        let vs = r32(bar) >> 16;
        crate::kprintln!(
            "[xhci] controller {:02x}:{:02x}.{} (xHCI {}.{}), {} ports, {} slots, {}-byte contexts",
            b, d, f, vs >> 8, (vs >> 4) & 0xF, { PORTS }, max_slots, { CSZ }
        );
        kinds_line(c, "[xhci] ");
        if c == 0 {
            dma_line("[xhci] ");
        }
        settle(c);
        // ports: a connected USB 2 device needs a reset to be enabled (USB 3 enables itself)
        for p in 1..=PORTS {
            let mut sc = r32(portsc(p));
            if sc & 1 << 9 == 0 {
                // port power off (controllers with port power control): on, then let a device appear
                w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | 1 << 9);
                crate::timer::busywait_ms(20);
                sc = r32(portsc(p));
            }
            if sc & 1 == 0 {
                continue;
            }
            CONNECTED += 1;
            if sc & 1 << 1 == 0 {
                let neutral = sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16);
                w32(portsc(p), neutral | 1 << 4); // PR
                wait(PORT_RESET_MS, || r32(portsc(p)) & 1 << 21 != 0); // PRC
                crate::timer::busywait_ms(20); // reset recovery (USB 2.0 TRSTRCY, 10 ms min)
            }
            let sc = r32(portsc(p));
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | (sc & 0x00FE_0000)); // clear changes
            if let (true, Some(fd)) = (sc & 1 << 1 != 0, free_dev()) {
                if !attach(fd, p, sc) {
                    left_alone(c, p, false);
                }
            }
        }
        READY = true; // port changes are served from now on, even with nothing plugged in
        PENDING[c] = 0; // the changes our own boot-time resets caused are not news
        true
    }
}

/// F2 (HP ENVY): right after the controller is reset and started, a real xHCI has not yet seen
/// the devices on its ports again (the HP said "0 devices connected" at boot, then found its
/// webcam, Bluetooth and fingerprint reader as hot-plugs a moment later, and failed to address
/// them). As Linux's hub driver does (power-on to power-good, then a port must stay connected
/// for 100 ms before it is used), every port is powered, then the ports are watched until the
/// set with a device has not changed for 100 ms, at least 150 ms and at most 1 s in all.
unsafe fn settle(c: usize) {
    let mut powered = false;
    for p in 1..=PORTS {
        let sc = r32(portsc(p));
        if sc & 1 << 9 == 0 {
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | 1 << 9);
            powered = true;
        }
    }
    if powered {
        crate::timer::busywait_ms(20); // power-on to power-good
    }
    let seen = || {
        let mut m = [0u64; 4];
        for p in 1..=PORTS.min(255) {
            if r32(portsc(p)) & 1 != 0 {
                m[(p / 64) as usize] |= 1 << (p % 64);
            }
        }
        m
    };
    let (mut last, mut steady, mut t) = (seen(), 0u32, 0u32);
    while (steady < 100 || t < 150) && t < 1000 {
        crate::timer::busywait_ms(10);
        t += 10;
        let m = seen();
        if m == last {
            steady += 10;
        } else {
            (last, steady) = (m, 0);
        }
    }
    let n: u32 = last.iter().map(|w| w.count_ones()).sum();
    SETTLE[c] = (t, n, steady >= 100);
    crate::kprintln!(
        "[xhci] ports {} after {} ms: {} of {} with a device",
        if steady >= 100 { "steady" } else { "still changing" }, t, n, { PORTS }
    );
}

/// Enumerate the device on port `port` (speed code `speed`) as device `d`; true if it is a
/// boot keyboard or boot mouse (one of each is kept) and its interrupt endpoint is now running.
/// A device that is neither gets its slot disabled again, so `d` stays free.
unsafe fn hid(d: usize, port: u32, speed: u32) -> bool {
    LAST_OTHER = (0, 0, 0, 0); // this try's own descriptor, never the last device's
    MSC_SEEN = (0, 0);
    let ev = match command(0, T_ENABLE_SLOT << 10) {
        Some(e) => e,
        None => return false,
    };
    let slot = ev.control >> 24;
    DEVS[d] = Dev { slot, dci: 0, kind: 0, port, ctl: CUR, route: PATH.0, vid: 0, pid: 0 };
    let ok = enumerate(d, port, speed);
    if !ok {
        command(0, T_DISABLE_SLOT << 10 | slot << 24);
        DCBAA[CUR].0[slot as usize] = 0;
        DEVS[d] = NO_DEV;
    }
    ok
}

/// The rest of `hid`: address, descriptors, the boot interface, its interrupt-IN endpoint.
unsafe fn enumerate(d: usize, port: u32, speed: u32) -> bool {
    let slot = DEVS[d].slot;
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    let outb = addr_of_mut!(OUTCTX[d].0) as *mut u8;
    *addr_of_mut!(PEP0[d]) = Prod { idx: 0, cycle: 1 };
    *addr_of_mut!(PEPIN[d]) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(EP0[d]) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    core::ptr::write_bytes(addr_of_mut!(EPIN[d]) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    core::ptr::write_bytes(inb, 0, 4096);
    core::ptr::write_bytes(outb, 0, 4096);
    DCBAA[CUR].0[slot as usize] = outb as u64;
    let mps0: u32 = match speed {
        3 => 64,                // high speed
        s if s >= 4 => 512,     // SuperSpeed, and SuperSpeedPlus (5 to 7: USB 3.1/3.2 ports)
        _ => 8,                 // full / low speed: 8 until the descriptor says otherwise
    };
    *ctx(inb, 0).add(1) = 0b11; // add slot + EP0
    let sl = ctx(inb, 1);
    *sl = speed << 20 | 1 << 27 | PATH.0; // speed, context entries = 1, route string
    *sl.add(1) = port << 16; // root hub port
    *sl.add(2) = PATH.1 | PATH.2 << 8; // TT hub slot and port (0 when none)
    let e0 = ctx(inb, 2);
    *e0.add(1) = 3 << 1 | 4 << 3 | mps0 << 16; // CErr 3, control, max packet
    let tr = addr_of!(EP0[d]) as u64;
    *e0.add(2) = tr as u32 | 1; // TR dequeue, DCS 1
    *e0.add(3) = (tr >> 32) as u32;
    *e0.add(4) = 8; // average TRB length
    let bsr = BSR_NOW; // F2: descriptor first (BSR) or plain, see BSR_FIRST
    ADDR_STAGE = if bsr { "Address Device (BSR, no SET_ADDRESS yet)" } else { "Address Device" };
    if command(inb as u64, T_ADDRESS_DEVICE << 10 | (bsr as u32) << 9 | slot << 24).is_none() {
        return false;
    }
    if !bsr {
        crate::timer::busywait_ms(SET_ADDRESS_MS); // let the device take its address (F2)
        ADDR_WAITS += 1;
    }
    // device descriptor: first the real EP0 packet size (8 bytes plain, 64 asked descriptor
    // first, as Windows does), then all 18
    ADDR_STAGE = if bsr { "first descriptor read at the default address" } else { "first descriptor read" };
    if !control(d, 0x80, 6, 0x0100, 0, if bsr { FIRST_READ_BSR } else { 8 }) {
        return false;
    }
    if FAIL_NEXT > 0 {
        // `usbaddr fail N`: this try is made to fail here, as the HP ENVY's did (F2)
        FAIL_NEXT -= 1;
        FAILED_ON_PURPOSE += 1;
        LAST_CC = 4;
        return false;
    }
    let real = DBUF.0[7] as u32;
    LAST_MPS0 = real;
    let mps = if speed < 3 && real != mps0 && real >= 8 { real } else { mps0 };
    if bsr {
        // now the address: the same slot and EP0, EP0 with the real packet size and its
        // dequeue pointer where the ring is now (the controller reads the context again)
        let p = &*addr_of!(PEP0[d]);
        let deq = tr + p.idx as u64 * core::mem::size_of::<Trb>() as u64;
        *ctx(inb, 0) = 0;
        *ctx(inb, 0).add(1) = 0b11; // slot + EP0
        *e0.add(1) = 3 << 1 | 4 << 3 | mps << 16;
        *e0.add(2) = deq as u32 | p.cycle;
        *e0.add(3) = (deq >> 32) as u32;
        ADDR_STAGE = "Address Device after the descriptor read";
        if command(inb as u64, T_ADDRESS_DEVICE << 10 | slot << 24).is_none() {
            return false;
        }
        crate::timer::busywait_ms(SET_ADDRESS_MS); // let the device take its address (F2)
        ADDR_WAITS += 1;
    } else if mps != mps0 {
        *ctx(inb, 0) = 0;
        *ctx(inb, 0).add(1) = 0b10; // evaluate EP0
        *e0.add(1) = 3 << 1 | 4 << 3 | real << 16;
        if command(inb as u64, T_EVALUATE_CTX << 10 | slot << 24).is_none() {
            return false;
        }
    }
    ADDR_STAGE = "device descriptor read";
    if !control(d, 0x80, 6, 0x0100, 0, 18) {
        return false;
    }
    if bsr {
        ADDR_BSR += 1;
    } else {
        ADDR_PLAIN += 1;
    }
    let (vid, pid) = (u16::from_le_bytes([DBUF.0[8], DBUF.0[9]]), u16::from_le_bytes([DBUF.0[10], DBUF.0[11]]));
    let num_configs = DBUF.0[17];
    (DEVS[d].vid, DEVS[d].pid) = (vid, pid);
    LAST_OTHER = (vid, pid, DBUF.0[4], 0);
    if DBUF.0[4] == 9 {
        let protocol = DBUF.0[6]; // 2 = a high-speed hub with one TT per port (multi-TT)
        return hub_setup(d, port, speed, vid, pid, protocol); // its own ports are enumerated next
    }
    // configuration descriptor (header, then the whole thing)
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let config_value = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    if total >= 9 + 9 && DBUF.0[9 + 1] == 4 {
        LAST_OTHER.3 = DBUF.0[9 + 5]; // the first interface's class
    }
    // find a HID boot keyboard (3/1/1) or boot mouse (3/1/2) interface (one of a kind not
    // already enumerated) and its interrupt-IN endpoint
    let (mut i, mut iface, mut in_kbd, mut ep, mut kind) = (0usize, 0u8, false, None, 0u8);
    // and the first other HID interface (not a keyboard), in case it is an absolute pointer:
    // (interface, report descriptor length, its interrupt-IN endpoint)
    let (mut in_tab, mut tab): (bool, Option<(u8, u16, Option<(u8, u32, u8)>)>) = (false, None);
    let mut mouse_rd = 0u16; // the boot mouse's report descriptor length, to look for a wheel
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 {
            break;
        }
        match DBUF.0[i + 1] {
            4 if i + 8 <= total => {
                let k = match (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]) {
                    (3, 1, 1) if KBD_SLOT == 0 => KIND_KBD,
                    (3, 1, 2) if MOUSE_SLOT == 0 => KIND_MOUSE,
                    _ => 0,
                };
                in_kbd = k != 0 && ep.is_none();
                if in_kbd {
                    iface = DBUF.0[i + 2];
                    kind = k;
                }
                in_tab = k == 0 && tab.is_none() && TABLET_SLOT == 0 && DBUF.0[i + 5] == 3 && (DBUF.0[i + 6], DBUF.0[i + 7]) != (1, 1);
                if in_tab {
                    tab = Some((DBUF.0[i + 2], 0, None));
                }
            }
            0x21 if in_kbd && kind == KIND_MOUSE && i + 9 <= total && DBUF.0[i + 6] == 0x22 => {
                mouse_rd = u16::from_le_bytes([DBUF.0[i + 7], DBUF.0[i + 8]]);
            }
            0x21 if in_tab && i + 9 <= total && DBUF.0[i + 6] == 0x22 => {
                if let Some(t) = tab.as_mut() {
                    t.1 = u16::from_le_bytes([DBUF.0[i + 7], DBUF.0[i + 8]]); // the report descriptor's length
                }
            }
            5 if (in_kbd && ep.is_none() || in_tab) && i + 7 <= total => {
                let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
                if addr & 0x80 != 0 && attr & 3 == 3 {
                    let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                    if in_kbd && ep.is_none() {
                        ep = Some((addr & 0xF, mps, DBUF.0[i + 6]));
                    } else if let Some(t) = tab.as_mut() {
                        t.2 = t.2.or(Some((addr & 0xF, mps, DBUF.0[i + 6])));
                    }
                }
            }
            _ => {}
        }
        i += len;
    }
    if ep.is_none() {
        if let Some((ti, tlen, Some(tep))) = tab {
            // configured first: a HID class descriptor belongs to an interface (HID 7.1)
            if tlen > 0 && control(d, 0x00, 9, config_value as u16, 0, 0) && control(d, 0x81, 6, 0x2200, ti as u16, tlen.min(512)) {
                if let Some(a) = parse_abs(&(&*addr_of!(DBUF)).0[..tlen.min(512) as usize]) {
                    TAB = a;
                    TAB_LEN = tep.1.clamp(8, 512);
                    (ep, kind, iface) = (Some(tep), KIND_TABLET, ti);
                }
            }
        }
    }
    let Some((epn, mps, binterval)) = ep else {
        // not a boot keyboard or mouse: perhaps a USB Ethernet adapter
        return (BT_SLOT == 0 && bt_setup(d, port, speed, vid, pid))
            || (SER_SLOT == 0 && serial_setup(d, port, speed, vid, pid))
            || (CAM_SLOT == 0 && video_setup(d, port, speed, vid, pid))
            || (AUD_SLOT == 0 && audio_setup(d, port, speed, vid, pid))
            || (NET_SLOT == 0 && net_setup(d, port, speed, num_configs, vid, pid))
            || (MSC_SLOT == 0 && msc_setup(d, port, speed, vid, pid));
    };
    if !control(d, 0x00, 9, config_value as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    // a mouse with a wheel in its report descriptor stays in the report protocol (a boot
    // report has no wheel); any other keyboard or mouse is switched to the boot protocol
    let mut rel = None;
    if kind == KIND_MOUSE && mouse_rd > 0 && control(d, 0x81, 6, 0x2200, iface as u16, mouse_rd.min(512)) {
        rel = parse_rel(&(&*addr_of!(DBUF)).0[..mouse_rd.min(512) as usize]);
    }
    if kind == KIND_MOUSE {
        MREL = rel;
        MREP_LEN = mps.clamp(8, 512);
    }
    if kind != KIND_TABLET {
        // SET_PROTOCOL(boot), or (report) for a wheel mouse; a refusal is survivable
        control(d, 0x21, 0x0B, rel.is_some() as u16, iface as u16, 0);
    }
    control(d, 0x21, 0x0A, 0, iface as u16, 0); // SET_IDLE(0): report only on change
    // configure the interrupt-IN endpoint: DCI = 2*ep + 1
    let dci = 2 * epn as u32 + 1;
    let interval = if speed >= 3 {
        (binterval as u32).clamp(1, 16) - 1
    } else {
        let frames = (binterval as u32).max(1) * 8; // in 125 us units
        (31 - frames.leading_zeros()).clamp(3, 10)
    };
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << dci; // slot + the endpoint
    let sl = ctx(inb, 1);
    *sl = speed << 20 | dci << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    let ec = ctx(inb, 1 + dci as usize);
    *ec = interval << 16;
    *ec.add(1) = 3 << 1 | 7 << 3 | mps << 16; // CErr 3, interrupt IN
    let tr = addr_of!(EPIN[d]) as u64;
    *ec.add(2) = tr as u32 | 1;
    *ec.add(3) = (tr >> 32) as u32;
    *ec.add(4) = 8 | mps << 16; // average TRB length, max ESIT payload
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].dci = dci;
    DEVS[d].kind = kind;
    NDEVS = NDEVS.max(d + 1);
    if kind == KIND_KBD {
        KBD_SLOT = slot;
        KBD_PORT = port;
        KBD_IFACE = iface as u16;
        LED_WANT = CAPS_LOCK; // a keyboard plugged in while Caps Lock is on gets its light too
    } else if kind == KIND_TABLET {
        TABLET_SLOT = slot;
        TABLET_PORT = port;
    } else {
        MOUSE_SLOT = slot;
        MOUSE_PORT = port;
    }
    crate::kprintln!(
        "[xhci] port {}: USB {} {:04x}:{:04x} ({} speed), slot {}, interrupt-IN endpoint {} every {} us",
        port, match kind { KIND_KBD => "keyboard", KIND_TABLET => "tablet", _ => "mouse" }, vid, pid,
        speed_name(speed),
        slot, epn, (1u32 << interval) * 125
    );
    if kind == KIND_TABLET {
        let a = TAB;
        crate::kprintln!(
            "[xhci] tablet: absolute pointer, report id {}, x {} bits at bit {} ({}..{}), y {} bits at bit {} ({}..{}), {}",
            a.id, a.x.size, a.x.bit, a.x.min, a.x.max, a.y.size, a.y.bit, a.y.min, a.y.max,
            if a.tip { "touch tip = left button" } else if a.nbtn > 0 { "buttons" } else { "no buttons" }
        );
    }
    if kind == KIND_MOUSE {
        if let Some(m) = rel {
            crate::kprintln!(
                "[xhci] mouse: wheel, report protocol: report id {}, {} buttons, x {} bits at bit {}, y {} bits at bit {}, wheel {} bits at bit {}",
                m.id, m.nbtn, m.x.size, m.x.bit, m.y.size, m.y.bit, m.wheel.size, m.wheel.bit
            );
        } else {
            crate::kprintln!("[xhci] mouse: no wheel in its report descriptor, boot protocol");
        }
    }
    queue_report(d);
    READY = true;
    true
}

/// Ask device `d` for its next report (8 bytes: a boot keyboard's whole report; a boot
/// mouse's is 3 or 4 and ends the transfer short; an absolute pointer's up to its packet size).
unsafe fn queue_report(d: usize) {
    push(
        addr_of_mut!(EPIN[d].0) as *mut Trb,
        NXFER,
        &mut (*addr_of_mut!(PEPIN))[d],
        addr_of!(REPORT[d]) as u64,
        match DEVS[d].kind {
            KIND_TABLET => TAB_LEN,
            KIND_MOUSE if (*addr_of!(MREL)).is_some() => MREP_LEN,
            _ => 8,
        },
        T_NORMAL << 10 | 1 << 5 | 1 << 2,
    );
    doorbell(DEVS[d].slot, DEVS[d].dci);
}

/// HID usage -> ASCII (unshifted, shifted), boot keyboard page.
fn ascii(usage: u8, shift: bool) -> Option<u8> {
    let c = match usage {
        0x04..=0x1D => {
            let l = b'a' + (usage - 0x04);
            return Some(if shift { l.to_ascii_uppercase() } else { l });
        }
        0x1E..=0x27 => {
            let (plain, shifted) = (b"1234567890", b"!@#$%^&*()");
            let i = (usage - 0x1E) as usize;
            return Some(if shift { shifted[i] } else { plain[i] });
        }
        0x28 => (b'\r', b'\r'),
        0x29 => (0x1B, 0x1B),
        0x2A => (0x7F, 0x7F),
        0x2B => (b'\t', b'\t'),
        0x2C => (b' ', b' '),
        0x2D => (b'-', b'_'),
        0x2E => (b'=', b'+'),
        0x2F => (b'[', b'{'),
        0x30 => (b']', b'}'),
        0x31 => (b'\\', b'|'),
        0x33 => (b';', b':'),
        0x34 => (b'\'', b'"'),
        0x35 => (b'`', b'~'),
        0x36 => (b',', b'<'),
        0x37 => (b'.', b'>'),
        0x38 => (b'/', b'?'),
        _ => return None,
    };
    Some(if shift { c.1 } else { c.0 })
}

/// Called from the local-APIC timer interrupt: every keyboard report that arrived becomes
/// console input (the keys newly pressed in it, in order); every mouse report moves the
/// pointer and counts the buttons newly pressed.
pub fn poll() {
    // SAFETY: single core; READY is only set once init is complete, and this runs with
    // interrupts off (inside the timer handler) so it never interleaves with itself.
    unsafe {
        if !READY || BUSY {
            return;
        }
        if let Some((_, c)) = HELD {
            let now = crate::timer::cycles();
            if now >= REPEAT_AT {
                crate::uart::inject(c);
                REPEATS += 1;
                REPEAT_AT = now + crate::timer::hz() / 30;
            }
        }
        if SWITCHING {
            return; // the mainline is half-way through a select
        }
        let home = CUR;
        for c in 0..NRUN.max(1) {
            select(c);
            if DEAD[c] == 0 && !GIVEN_UP[c] {
                let st = r32(op(0x04));
                if st & (1 << 2 | 1 << 12) != 0 {
                    DEAD[c] = st; // reset by `service`, outside the interrupt
                }
            }
            if DEAD[c] != 0 || GIVEN_UP[c] {
                continue; // its event ring is not read until it runs again
            }
            while let Some(ev) = next_event() {
                handle(ev);
            }
            if SER_SLOT != 0 && SER_CTL == c {
                ser_kick(); // the console text printed since the last tick, out to the serial cable
            }
        }
        select(home);
    }
}

/// One event from the ring: a device's finished transfer is served (keyboard keys, mouse
/// moves, network frames, the next transfer queued); anything else is ignored here.
unsafe fn handle(ev: Trb) {
    if (ev.control >> 10) & 0x3F == T_EV_PORT {
        let port = ((ev.param >> 24) & 0xFF) as u32;
        if (1..32).contains(&port) {
            PENDING[CUR] |= 1 << port; // enumerated or detached by `service`, outside the interrupt
        }
        return;
    }
    if (ev.control >> 10) & 0x3F != T_EV_TRANSFER {
        return;
    }
    let Some(d) = (0..NDEVS).find(|&d| DEVS[d].slot == ev.control >> 24 && DEVS[d].ctl == CUR) else { return };
    let cc = ev.status >> 24;
    if DEVS[d].kind == KIND_HUB && DEVS[d].dci != 0 {
        // the hub's status-change endpoint: a bitmap of its ports that changed (bit 0 = the hub)
        if cc == CC_SUCCESS || cc == CC_SHORT {
            let r = &mut (*addr_of_mut!(REPORT[d])).0;
            HUBCHG[d] |= u16::from_le_bytes([r[0], r[1]]) & !1;
            (r[0], r[1]) = (0, 0);
            queue_report(d); // after an error it stays quiet (no storm); root-port unplug still works
        }
        return;
    }
    if DEVS[d].kind == KIND_HUB || DEVS[d].kind == KIND_MSC || DEVS[d].kind == 0 {
        return; // a USB 3 hub has no transfers of ours; kind 0 is still being enumerated
    }
    if DEVS[d].kind == KIND_NET {
        net_event(&ev, cc);
        return;
    }
    if DEVS[d].kind == KIND_SERIAL {
        ser_event(d, &ev, cc);
        return;
    }
    if DEVS[d].kind == KIND_CAM {
        cam_event(&ev, cc);
        return;
    }
    if DEVS[d].kind == KIND_BT {
        bt_event(d, &ev, cc);
        return;
    }
    if DEVS[d].kind == KIND_AUDIO {
        // one isochronous packet played (or missed); ring under/overrun events name no packet
        if cc != 14 && cc != 15 {
            AUD_DONE += 1;
            if cc != CC_SUCCESS && cc != CC_SHORT {
                AUD_MISSED += 1;
            }
        }
        return;
    }
    if DEVS[d].kind == KIND_MOUSE {
        if cc == CC_SUCCESS || cc == CC_SHORT {
            let r = &(*addr_of!(REPORT[d])).0;
            if let Some(m) = MREL {
                let got = MREP_LEN.saturating_sub(ev.status & 0xFF_FFFF) as usize; // minus the residue
                mouse_rel_report(m, &r[..got.min(512)]);
            } else {
                MREPORTS += 1;
                let (dx, dy) = mouse_accel(r[1] as i8 as i32, r[2] as i8 as i32);
                mouse_report(r[0], dx, dy);
            }
        }
        queue_report(d);
        return;
    }
    if DEVS[d].kind == KIND_TABLET {
        if cc == CC_SUCCESS || cc == CC_SHORT {
            let got = TAB_LEN.saturating_sub(ev.status & 0xFF_FFFF) as usize; // minus the residue
            tablet_report(&(&*addr_of!(REPORT[d])).0[..got.min(512)]);
        }
        queue_report(d);
        return;
    }
    if cc == CC_SUCCESS || cc == CC_SHORT {
        REPORTS += 1;
        let r = (*addr_of!(REPORT[d])).0;
        let shift = r[0] & 0x22 != 0;
        let last = LAST_KEYS;
        // desktop shortcuts (desk.rs): Super alone toggles the launcher on its release
        let (alt, sup, desk) = (r[0] & 0x44 != 0, r[0] & 0x88 != 0, crate::desk::shown());
        if desk && LAST_MODS & 0x88 != 0 && !sup && !SUPER_COMBO {
            crate::desk::post_key(crate::desk::KEY_SUPER);
        }
        if !sup {
            SUPER_COMBO = false;
        }
        LAST_MODS = r[0];
        for &k in &r[2..8] {
            if k > 3 && !last.contains(&k) {
                if sup {
                    SUPER_COMBO = true; // Super+key is a shortcut, not a launcher press
                }
                if let Some(dk) = desk_key(k, alt, sup).filter(|_| desk) {
                    crate::desk::post_key(dk);
                    continue;
                }
                if k == 0x52 || k == 0x51 {
                    crate::uart::history_key(k == 0x52); // Up/Down: shell history (uart.rs)
                    continue;
                }
                if (k == 0x4b || k == 0x4e) && shift {
                    crate::uart::console_page(k == 0x4b); // Shift+PgUp/PgDn: scrollback (conback.rs)
                    continue;
                }
                if k == 0x39 {
                    CAPS_LOCK = !CAPS_LOCK; // Caps Lock: its light is set by `service`
                    LED_WANT = true;
                    continue;
                }
                // Caps Lock turns letters only; Shift with it gives small letters
                let up = if (0x04..=0x1D).contains(&k) { shift ^ CAPS_LOCK } else { shift };
                if let Some(c) = ascii(k, up) {
                    crate::uart::inject(c);
                    KEYS += 1;
                    // the newest key pressed is the one that repeats while held
                    HELD = Some((k, c));
                    REPEAT_AT = crate::timer::cycles() + crate::timer::hz() / 2;
                }
            }
        }
        if let Some((k, _)) = HELD {
            if !r[2..8].contains(&k) {
                HELD = None; // released
            }
        }
        (*addr_of_mut!(LAST_KEYS)).copy_from_slice(&r[2..8]);
    }
    queue_report(d);
}

/// The device just left alone in words, for the boot log: its class, or for a mass storage
/// device that msc_setup could not use, the transport it speaks (UAS only, CBI only, or Bulk-Only
/// with a command set other than SCSI), so a real PC's report says why the disk was not used.
unsafe fn other_name(c: u8) -> &'static str {
    match (c, MSC_SEEN) {
        (0x08, (0, 0)) => class_name(c),
        (0x08, (_, 0x62)) => "mass storage, UAS only: needs streams, which this port or controller lacks",
        (0x08, (_, 0x00 | 0x01)) => "mass storage, CBI only (an old USB floppy drive): not driven",
        (0x08, (_, 0x50)) => "mass storage, Bulk-Only but not SCSI commands: not driven",
        (0x08, _) => "mass storage of a kind not driven",
        _ => class_name(c),
    }
}

/// A USB class code in words, for the boot log and `hwscan`.
pub(crate) fn class_name(c: u8) -> &'static str {
    match c {
        0x01 => "audio",
        0x02 | 0x0A => "communications",
        0x03 => "HID, not a boot keyboard or mouse",
        0x06 => "camera / image",
        0x07 => "printer",
        0x08 => "mass storage",
        0x09 => "hub",
        0x0B => "smart card",
        0x0E => "video camera",
        0x0D => "content security",
        0xE0 => "wireless (Bluetooth)",
        0xEF => "miscellaneous",
        0xFF => "vendor-specific",
        _ => "unknown class",
    }
}

/// Hot-plug: called from the shell's idle loop (cpu::wfi), never from an interrupt. A root
/// port that gained a device gets it reset and enumerated (keyboard, mouse, hub, network
/// adapter); one that lost its device has every device behind it detached (slots disabled,
/// entries freed). While this runs the timer poll stands aside; the waits serve all events.
pub fn service() {
    // SAFETY: mainline, single core; BUSY keeps the timer poll off the event ring meanwhile.
    unsafe {
        if !READY
            || BUSY
            || ((*addr_of!(PENDING)).iter().all(|&p| p == 0)
                && (*addr_of!(HUBCHG)).iter().all(|&h| h == 0)
                && (*addr_of!(DEAD)).iter().all(|&s| s == 0)
                && !LED_WANT)
        {
            return;
        }
        BUSY = true;
        let home = CUR;
        for c in 0..NRUN.max(1) {
            if DEAD[c] != 0 {
                select(c);
                let st = DEAD[c] & !NO_ANSWER;
                let what = match (st & 1 << 2 != 0, st & 1 << 12 != 0) {
                    _ if DEAD[c] & NO_ANSWER != 0 => "a command got no answer",
                    (true, true) => "a host system error and a controller error",
                    (true, false) => "a host system error",
                    _ => "a controller error",
                };
                let test = DEAD_TEST;
                DEAD_TEST = "";
                if RESTARTS[c] >= RESTART_MAX {
                    DEAD[c] = 0;
                    GIVEN_UP[c] = true;
                    crate::kprintln!(
                        "[xhci] controller {} stopped working again ({}, USBSTS {:#x}{}) after {} resets: its USB devices are not used until the next start",
                        c + 1, what, st, test, RESTART_MAX
                    );
                    continue;
                }
                crate::kprintln!("[xhci] controller {} stopped working ({}, USBSTS {:#x}{})", c + 1, what, st, test);
                restart(c);
                PENDING[c] = 0;
            }
            if PENDING[c] != 0 && !GIVEN_UP[c] {
                select(c);
                service_ports(c);
            }
        }
        for d in 0..NDEV {
            if HUBCHG[d] != 0 {
                if DEVS[d].kind == KIND_HUB && DEVS[d].slot != 0 {
                    select(DEVS[d].ctl);
                    service_hub(d);
                } else {
                    HUBCHG[d] = 0; // the hub itself went meanwhile
                }
            }
        }
        if LED_WANT {
            LED_WANT = false;
            let kbd = (0..NDEVS).find(|&d| DEVS[d].kind == KIND_KBD && DEVS[d].slot == KBD_SLOT && KBD_SLOT != 0);
            if let Some(d) = kbd {
                select(DEVS[d].ctl);
                // SET_REPORT (output report 0): bit 1 is the Caps Lock light, the others off
                (*addr_of_mut!(DBUF)).0[0] = (CAPS_LOCK as u8) << 1;
                CTL_QUIET = true; // a keyboard without lights may refuse; that is counted, not logged
                if control(d, 0x21, 0x09, 0x0200, KBD_IFACE, 1) {
                    LED_SETS += 1;
                } else {
                    LED_FAILS += 1;
                }
                CTL_QUIET = false;
            }
        }
        select(home);
        BUSY = false;
    }
}

/// Free device `d` on controller `c` (the current one): its slot disabled, what it was used
/// for forgotten, the entry freed. A hub also takes its port count out of `lsusb`'s total.
unsafe fn detach(d: usize, c: usize) {
    command(0, T_DISABLE_SLOT << 10 | DEVS[d].slot << 24);
    forget(d, c);
    UNPLUGS += 1;
}

/// `detach` without the Disable Slot command (for a controller that is being reset): what
/// device `d` was used for forgotten, its entry freed.
unsafe fn forget(d: usize, c: usize) {
    let (slot, kind) = (DEVS[d].slot, DEVS[d].kind);
    DCBAA[c].0[slot as usize] = 0;
    match kind {
        KIND_KBD => {
            KBD_SLOT = 0;
            HELD = None;
        }
        KIND_MOUSE => (MOUSE_SLOT, MREL) = (0, None),
        KIND_TABLET => TABLET_SLOT = 0,
        KIND_NET => NET_SLOT = 0,
        KIND_AUDIO => AUD_SLOT = 0,
        KIND_CAM => {
            if CAM_RUN {
                (CAM_RUN, CAM_STOPPING, CAM_STOP_SAID) = (false, false, false);
                crate::uvc::stopped(); // the stream is gone with the device: frames zeroed, mark off
                crate::kprintln!("[camera] the camera went while on: stopped (the mark is off, the frames are cleared)");
            }
            (CAM_SLOT, CAM) = (0, None);
        }
        KIND_BT => {
            BT_SLOT = 0;
            crate::bt::gone();
        }
        KIND_SERIAL => {
            SER_SLOT = 0;
            SER_INFLIGHT = false;
        }
        KIND_MSC => {
            MSC_SLOT = 0;
            (MSC_USABLE, MSC_RAW) = (false, false);
            (MSC_UAS, UAS_STREAMS) = (false, false);
            (MSC_RA_N, MSC_WB_N) = (0, 0); // gathered writes are lost with the device
        }
        KIND_HUB if HUBI[d].3 != 0 => {
            HUBS = HUBS.saturating_sub(1);
            HUB_PORTS = HUB_PORTS.saturating_sub(HUBI[d].3);
            if DEVS[d].dci != 0 {
                HUB_WATCHED = HUB_WATCHED.saturating_sub(1);
            }
        }
        _ => {}
    }
    HUBCHG[d] = 0;
    HUBI[d] = (0, 0, 0, 0);
    DEVS[d] = NO_DEV;
}

/// Reset controller `c` (the current one; BUSY is set) and find its devices again: every device
/// on it is forgotten (the controller lost their slots), its rings start over and `start` runs
/// as at boot (reset, rings, ports, devices). Gathered USB stick writes not yet sent are lost
/// with the controller. True if it runs again.
unsafe fn restart(c: usize) -> bool {
    let (b, d, f) = *addr_of!(PCI_AT);
    let lost_writes = if MSC_SLOT != 0 && MSC_D < NDEV && DEVS[MSC_D].ctl == c { MSC_WB_N } else { 0 };
    let mut before = 0;
    for i in 0..NDEV {
        if DEVS[i].slot != 0 && DEVS[i].ctl == c {
            forget(i, c);
            before += 1;
        }
    }
    crate::kprintln!(
        "[xhci] controller {}: resetting it and finding its devices again ({} device(s) were in use{})",
        c + 1, before, if lost_writes > 0 { "; gathered USB stick writes not yet sent are lost" } else { "" }
    );
    (PCMD, EV_IDX, EV_CYCLE, CONNECTED) = (Prod { idx: 0, cycle: 1 }, 0, 1, 0);
    DEAD[c] = 0;
    RESTARTS[c] += 1;
    if start(b, d, f) {
        GIVEN_UP[c] = false;
        let after = (0..NDEV).filter(|&i| DEVS[i].slot != 0 && DEVS[i].ctl == c).count();
        crate::kprintln!("[xhci] controller {} running again after the reset: {} of {} device(s) found again", c + 1, after, before);
        true
    } else {
        (GIVEN_UP[c], DEAD[c]) = (true, 0); // a command lost during the reset does not start another
        crate::kprintln!("[xhci] controller {} did not come back after the reset: its USB devices are not used until the next start", c + 1);
        false
    }
}

/// `xhcireset [n]`: reset controller n (1 if not given) by hand, the path a controller that
/// stopped working takes, after sending any gathered USB stick writes. `xhcireset fault [n]`
/// (test switch) marks it as stopped with a controller error instead, so the timer poll's path
/// runs: the next idle moment finds it and resets it. `xhcireset noanswer [n]` (test switch)
/// sends a No Op command without ringing the doorbell, so the command gets no answer and the
/// no-answer path runs: 5 seconds, then the reset at the next idle moment.
pub fn reset_cmd(rest: &str) {
    let mut words = rest.split_whitespace();
    let mut w = words.next();
    let fault = w == Some("fault");
    let noanswer = w == Some("noanswer");
    if fault || noanswer {
        w = words.next();
    }
    // SAFETY: mainline, single core; BUSY keeps the timer poll off the rings meanwhile.
    unsafe {
        if !READY || OP == 0 {
            crate::kprintln!("xhcireset: no running xHCI controller");
            return;
        }
        let n = NRUN.max(1);
        let Some(k) = w.map_or(Some(1), |s| s.parse::<usize>().ok()).filter(|&k| (1..=n).contains(&k)) else {
            crate::kprintln!("xhcireset: give a controller from 1 to {} (lsusb lists them)", n);
            return;
        };
        let c = k - 1;
        if noanswer {
            (BUSY, NOANSWER_TEST) = (true, true);
            let home = CUR;
            select(c);
            crate::kprintln!("xhcireset: sending controller {} a No Op command without ringing its doorbell (test)", k);
            let answered = command(0, T_NOOP << 10).is_some();
            NOANSWER_TEST = false;
            if !answered && DEAD[c] & NO_ANSWER != 0 {
                DEAD_TEST = ", set by xhcireset noanswer";
            }
            select(home);
            BUSY = false;
            crate::kprintln!("xhcireset: {}", if answered { "the command was answered after all" } else { "no answer; the controller is reset at the next idle moment" });
            return;
        }
        if fault {
            DEAD_TEST = ", set by xhcireset fault";
            DEAD[c] = 1 << 12;
            crate::kprintln!("xhcireset: controller {} marked as stopped (test); it is reset at the next idle moment", k);
            return;
        }
        BUSY = true;
        let home = CUR;
        select(c);
        if MSC_SLOT != 0 && MSC_D < NDEV && DEVS[MSC_D].ctl == c {
            msc_flush();
        }
        let ok = restart(c);
        select(home);
        BUSY = false;
        crate::kprintln!("xhcireset: {}", if ok { "done" } else { "the controller did not come back" });
    }
}

/// How many hub tiers a route string goes through.
fn depth(route: u32) -> u32 {
    (0..5).take_while(|t| route >> (4 * t) & 0xF != 0).count() as u32
}

/// The route string of hub `d`'s port `hp`, and the mask that covers it (every device behind
/// that port, at any depth, has those low bits).
unsafe fn hub_port_route(d: usize, hp: u32) -> (u32, u32) {
    let dep = depth(DEVS[d].route);
    (DEVS[d].route | hp << (4 * dep), (1u32 << (4 * (dep + 1))) - 1)
}

/// `service` for hub `d` (its controller current): every port its status-change endpoint
/// reported is read; its change bits are acknowledged; a device gone is detached with
/// everything behind it, a new one reset and enumerated, as a root port's are.
unsafe fn service_hub(d: usize) {
    let bits = core::mem::replace(&mut HUBCHG[d], 0);
    let (c, nports) = (DEVS[d].ctl, HUBI[d].3);
    for hp in 1..=nports {
        if bits & 1 << hp == 0 || DEVS[d].kind != KIND_HUB {
            continue;
        }
        let Some((st, ch)) = hub_status(d, hp) else { continue };
        for (bit, feature) in [(0u16, 16u16), (1, 17), (2, 18), (3, 19), (4, 20)] {
            if ch & 1 << bit != 0 {
                control(d, 0x23, 1, feature, hp as u16, 0); // CLEAR_FEATURE(C_PORT_...)
            }
        }
        let (route, mask) = hub_port_route(d, hp);
        let behind = |x: usize| {
            x != d && DEVS[x].slot != 0 && DEVS[x].ctl == c && DEVS[x].port == DEVS[d].port && DEVS[x].route & mask == route
        };
        if st & 1 == 0 {
            let mut gone = 0;
            for x in 0..NDEV {
                if behind(x) {
                    detach(x, c);
                    gone += 1;
                }
            }
            if gone > 0 {
                crate::kprintln!("[xhci] hub port {}: device unplugged", hp);
            }
            continue;
        }
        if (0..NDEV).any(behind) {
            continue; // still there: the change was its reset or an enable/suspend bit
        }
        // plugged in: let the contacts settle, as on a root port
        crate::timer::busywait_ms(100);
        if !matches!(hub_status(d, hp), Some((s, _)) if s & 1 != 0) {
            continue;
        }
        let Some(fd) = free_dev() else {
            crate::kprintln!("[xhci] hub port {}: device plugged in, but no room for another device", hp);
            continue;
        };
        if hub_port(d, hp, fd) {
            PLUGS += 1;
            crate::kprintln!("[xhci] hub port {}: device plugged in and ready", hp);
        }
    }
}

/// GET_PORT_STATUS of hub `d`'s port `hp`: (wPortStatus, wPortChange).
unsafe fn hub_status(d: usize, hp: u32) -> Option<(u16, u16)> {
    if control(d, 0xA3, 0, 0, hp as u16, 4) {
        Some((u16::from_le_bytes([DBUF.0[0], DBUF.0[1]]), u16::from_le_bytes([DBUF.0[2], DBUF.0[3]])))
    } else {
        None
    }
}

/// Reset hub `d`'s port `hp` (a device is connected there) and enumerate the device as entry
/// `fd`, with its route string and transaction translator; true if this driver uses it. A
/// device that does not answer gets the root ports' three tries (F2): each after a new port
/// reset with a longer recovery, the second the other way of addressing.
unsafe fn hub_port(d: usize, hp: u32, fd: usize) -> bool {
    let (speed, tts, ttp, _) = HUBI[d];
    let (port, slot, saved) = (DEVS[d].port, DEVS[d].slot, PATH);
    let mut ok = false;
    let mut answered = false;
    let mut enabled = true;
    for (n, bsr, ms) in [(1u32, BSR_FIRST, 0u64), (2, !BSR_FIRST, 50), (3, BSR_FIRST, 100)] {
        if n > 1 {
            ADDR_ERR = (LAST_CC, ADDR_STAGE);
            crate::kprintln!(
                "[xhci] hub port {}: device did not answer (completion code {} at {}); port reset, try {} of 3: {}, {} ms recovery",
                hp, { LAST_CC }, { ADDR_STAGE }, n, scheme_name(bsr), ms
            );
        }
        control(d, 0x23, 3, 4, hp as u16, 0); // SET_FEATURE(PORT_RESET)
        let mut reset_done = false;
        for _ in 0..50 {
            crate::timer::busywait_ms(5);
            if let Some((_, ch)) = hub_status(d, hp) {
                if ch & 1 << 4 != 0 {
                    reset_done = true;
                    break;
                }
            }
        }
        control(d, 0x23, 1, 20, hp as u16, 0); // CLEAR_FEATURE(C_PORT_RESET)
        control(d, 0x23, 1, 16, hp as u16, 0); // CLEAR_FEATURE(C_PORT_CONNECTION)
        crate::timer::busywait_ms(20 + ms); // reset recovery, longer on each retry
        let Some((st, _)) = hub_status(d, hp) else {
            enabled = false;
            break;
        };
        if !reset_done || st & 2 == 0 {
            crate::kprintln!("[xhci] hub port {}: device did not enable after reset", hp);
            enabled = false;
            break;
        }
        let cspeed = if st & 1 << 9 != 0 { 2 } else if st & 1 << 10 != 0 { 3 } else { 1 };
        let tt = if speed == 3 && cspeed < 3 { (slot, hp) } else { (tts, ttp) };
        PATH = (hub_port_route(d, hp).0, tt.0, tt.1);
        BSR_NOW = bsr;
        LAST_CC = 0;
        ok = hid(fd, port, cspeed);
        BSR_NOW = BSR_FIRST;
        PATH = saved;
        answered = ok || LAST_OTHER.0 != 0 || LAST_OTHER.1 != 0;
        if answered && n > 1 {
            ADDR_RETRIED += 1;
            crate::kprintln!("[xhci] hub port {}: answered on try {} ({})", hp, n, scheme_name(bsr));
        }
        if answered {
            break;
        }
    }
    if !enabled {
        return false; // gone, or the port would not enable: not an addressing failure
    }
    if !answered {
        ADDR_ERR = (LAST_CC, ADDR_STAGE);
        ADDR_NONE += 1;
        crate::kprintln!("[xhci] hub port {}: device did not answer (descriptor not read), left alone", hp);
        return false;
    }
    if !ok {
        note_other();
        let (v, pr, dc, ic) = LAST_OTHER;
        crate::kprintln!(
            "[xhci] hub port {}: device {:04x}:{:04x} ({}) is not one this driver uses",
            hp, v, pr, other_name(if dc != 0 { dc } else { ic })
        );
    }
    PATH = saved;
    ok
}

/// Enumerate the device on root port `p` (PORTSC `sc`, enabled) as entry `fd`; true if this
/// driver uses it. A device that did not answer at all (no descriptor read) gets its port reset
/// and one more try; one that answered and is not ours is not tried again (field item F1).
unsafe fn attach(fd: usize, p: u32, sc: u32) -> bool {
    // F2: up to three tries, each after a port reset with a longer recovery, the second the
    // other way of addressing (as Linux alternates its old and new schemes)
    let mut sc = sc;
    for (n, bsr, ms) in [(1u32, BSR_FIRST, 0u64), (2, !BSR_FIRST, 50), (3, BSR_FIRST, 100)] {
        if n > 1 {
            ADDR_ERR = (LAST_CC, ADDR_STAGE);
            crate::kprintln!(
                "[xhci] port {}: device did not answer (completion code {} at {}); port reset, try {} of 3: {}, {} ms recovery",
                p, { LAST_CC }, { ADDR_STAGE }, n, scheme_name(bsr), ms
            );
            sc = r32(portsc(p));
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | 1 << 4); // PR
            wait(PORT_RESET_MS, || r32(portsc(p)) & 1 << 21 != 0); // PRC
            crate::timer::busywait_ms(ms); // reset recovery (USB 2.0 TRSTRCY is 10 ms at least)
            sc = r32(portsc(p));
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | (sc & 0x00FE_0000));
            if sc & 3 != 3 {
                break; // gone, or the port would not enable
            }
        }
        BSR_NOW = bsr;
        LAST_CC = 0;
        let ok = hid(fd, p, port_speed(CUR, p, (sc >> 10) & 0xF));
        BSR_NOW = BSR_FIRST;
        let answered = ok || LAST_OTHER.0 != 0 || LAST_OTHER.1 != 0;
        if answered && n > 1 {
            ADDR_RETRIED += 1;
            crate::kprintln!("[xhci] port {}: answered on try {} ({})", p, n, scheme_name(bsr));
        }
        if answered {
            return ok;
        }
    }
    ADDR_ERR = (LAST_CC, ADDR_STAGE);
    ADDR_NONE += 1;
    false
}

fn scheme_name(bsr: bool) -> &'static str {
    if bsr { "descriptor read before the address (BSR)" } else { "address first (plain)" }
}

/// `usbaddr` (F2): how new devices are given their address, and how that went since boot;
/// `usbaddr bsr` reads the first descriptor before the address, `usbaddr plain` the default.
pub fn addr_cmd(rest: &str) {
    // SAFETY: shell context, single core; plain flags and counters.
    unsafe {
        match rest.trim() {
            "" => {}
            "bsr" => (BSR_FIRST, BSR_NOW) = (true, true),
            "plain" => (BSR_FIRST, BSR_NOW) = (false, false),
            "psi" => {
                psi_test();
                return;
            }
            r if r.starts_with("fail") => match r[4..].trim().parse::<u32>() {
                Ok(n) if n <= 9 => {
                    FAIL_NEXT = n;
                    crate::kprintln!(
                        "usb: test: the next {} addressing tr{} will fail at the first descriptor read (completion code 4)",
                        n, if n == 1 { "y" } else { "ies" }
                    );
                    return;
                }
                _ => {
                    crate::kprintln!("usbaddr: fail takes a number from 0 to 9");
                    return;
                }
            },
            _ => {
                crate::kprintln!("usbaddr: say bsr, plain or psi (or nothing to see how devices were addressed)");
                return;
            }
        }
        crate::kprintln!(
            "usb: new devices are tried first with {}; then the other way, then this way again",
            scheme_name(BSR_FIRST)
        );
        for c in 0..NRUN {
            let (ms, halted, intel) = RESET[c];
            crate::kprintln!(
                "  reset: controller {} {}ready {} ms after HCRST, first read after 1 ms{}",
                c + 1, if halted { "halted, " } else { "not halted, " }, ms,
                if intel { " (Intel host)" } else { "" }
            );
            let (t, n, ok) = SETTLE[c];
            crate::kprintln!(
                "  power-on: controller {}'s ports {} after {} ms, {} with a device",
                c + 1, if ok { "steady" } else { "still changing" }, t, n
            );
        }
        crate::kprintln!(
            "  set address: {} ms for the device to take it, {} wait(s) made; first read {} bytes descriptor first, 8 plain; last EP0 packet size {}",
            SET_ADDRESS_MS, { ADDR_WAITS }, FIRST_READ_BSR, { LAST_MPS0 }
        );
        dma_line("  ");
        crate::kprintln!(
            "  speed IDs: {} device(s) by a controller's own table{}",
            { PSI_USED }, if PSI_TEST { " (test table on controller 1)" } else { "" }
        );
        addr_line();
    }
}

/// F2 (9 Nov 2026): xHCI 1.2 table 6-1 says where the controller's structures may sit: device
/// and input contexts, the device context array and the scratchpad array inside one 4 KiB page,
/// every ring segment and every transfer's data buffer inside one 64 KiB block. QEMU does not
/// care; a real controller may fetch a context that crosses a page from the wrong place. The
/// statics are placed by the linker, and UEFI loads the image at a page boundary, so a page
/// offset here is the same on every PC. Returns (structures checked, how many cross, the first).
unsafe fn dma_check() -> (u32, u32, &'static str) {
    const PG: u64 = 4096;
    const K64: u64 = 65536;
    let mut st = (0u32, 0u32, "");
    fn chk(st: &mut (u32, u32, &'static str), name: &'static str, a: u64, len: usize, bound: u64) {
        st.0 += 1;
        if a / bound != (a + len as u64 - 1) / bound {
            if st.1 == 0 {
                st.2 = name;
            }
            st.1 += 1;
        }
    }
    use core::mem::size_of;
    for c in 0..NCTL {
        chk(&mut st, "command ring", addr_of!(CMD[c]) as u64, size_of::<Ring<NCMD>>(), K64);
        chk(&mut st, "event ring", addr_of!(EVT[c]) as u64, size_of::<Ring<NEV>>(), K64);
        chk(&mut st, "device context array", addr_of!(DCBAA[c]) as u64, 256 * 8, PG);
        chk(&mut st, "scratchpad array", addr_of!(SCRATCH_ARR[c]) as u64, 256 * 8, PG);
    }
    chk(&mut st, "input context", addr_of!(INCTX) as u64, 33 * CSZ, PG);
    for d in 0..NDEV {
        chk(&mut st, "device context", addr_of!(OUTCTX[d]) as u64, 32 * CSZ, PG);
        chk(&mut st, "control ring", addr_of!(EP0[d]) as u64, size_of::<Ring<NXFER>>(), K64);
        chk(&mut st, "interrupt ring", addr_of!(EPIN[d]) as u64, size_of::<Ring<NXFER>>(), K64);
        chk(&mut st, "report buffer", addr_of!(REPORT[d]) as u64, 512, K64);
    }
    chk(&mut st, "control buffer", addr_of!(DBUF) as u64, 512, K64);
    chk(&mut st, "stick ring", addr_of!(MSC_OUT_RING) as u64, size_of::<Ring<NXFER>>(), K64);
    chk(&mut st, "stick command", addr_of!(CBW) as u64, 64, K64);
    chk(&mut st, "stick status", addr_of!(CSW) as u64, 64, K64);
    chk(&mut st, "stick block", addr_of!(MSCBUF) as u64, 4096, K64);
    chk(&mut st, "UAS status ring", addr_of!(UAS_ST_RING) as u64, size_of::<Ring<NXFER>>(), K64);
    chk(&mut st, "UAS data-out ring", addr_of!(UAS_DO_RING) as u64, size_of::<Ring<NXFER>>(), K64);
    chk(&mut st, "UAS status buffer", addr_of!(UAS_ST) as u64, 512, K64);
    for i in 0..3 {
        chk(&mut st, "UAS stream array", addr_of!(UAS_SCA[i]) as u64, size_of::<Sca>(), K64);
    }
    for i in 0..6 {
        chk(&mut st, "UAS stream ring", addr_of!(UAS_SR[i]) as u64, size_of::<Ring<NXFER>>(), K64);
    }
    chk(&mut st, "stick read-ahead", msc_ra() as u64, MSC_RA_BYTES, K64);
    chk(&mut st, "stick write-behind", msc_wb() as u64, MSC_RA_BYTES, K64);
    for i in 0..NRXB {
        chk(&mut st, "NCM receive buffer", ncm_rx(i) as u64, NCM_IN, K64);
    }
    chk(&mut st, "network ring", addr_of!(EPOUT) as u64, size_of::<Ring<NXFER>>(), K64);
    for i in 0..NRXB {
        chk(&mut st, "network receive buffer", addr_of!(NETRX[i]) as u64, FRAME, K64);
    }
    for i in 0..NTXB {
        chk(&mut st, "network send buffer", addr_of!(NETTX[i]) as u64, FRAME, K64);
    }
    chk(&mut st, "serial ring", addr_of!(SER_RING) as u64, size_of::<Ring<NXFER>>(), K64);
    chk(&mut st, "serial buffer", addr_of!(SER_TXBUF) as u64, 4096, K64);
    chk(&mut st, "camera ring", addr_of!(CAM_RING) as u64, size_of::<Ring<NCAM>>(), K64);
    for i in 0..NCAM - 1 {
        chk(&mut st, "camera packet", addr_of!(CAMBUF[i]) as u64, CAM_PKT, K64);
    }
    chk(&mut st, "camera configuration", cam_cfg() as u64, CAM_CFG, K64);
    chk(&mut st, "sound ring", addr_of!(AUD_RING) as u64, size_of::<Ring<NAUD>>(), K64);
    for i in 0..NAUD {
        chk(&mut st, "sound packet", addr_of!(AUDBUF[i]) as u64, AUD_PKT, K64);
    }
    st
}

/// One line on where the controller's structures sit (F2), at boot and in `usbaddr`.
unsafe fn dma_line(lead: &str) {
    let (n, bad, first) = dma_check();
    if bad == 0 {
        crate::kprintln!(
            "{}memory: all {} controller structures inside their xHCI limits (contexts in one 4 KiB page, rings and buffers in one 64 KiB block)",
            lead, n
        );
    } else {
        crate::kprintln!(
            "{}memory: {} of {} controller structures cross an xHCI limit (first: {})",
            lead, bad, n, first
        );
    }
}

/// One line on addressing (two after a retry), for `usbaddr` and `lsusb`.
unsafe fn addr_line() {
    crate::kprintln!(
        "  addressing: {} plain, {} descriptor first (BSR), {} answered on a later try, {} on no try",
        { ADDR_PLAIN }, { ADDR_BSR }, { ADDR_RETRIED }, { ADDR_NONE }
    );
    if ADDR_RETRIED + ADDR_NONE > 0 {
        let (cc, stage) = ADDR_ERR;
        crate::kprintln!("  last addressing error: completion code {} at {}", cc, stage);
    }
    if FAILED_ON_PURPOSE + FAIL_NEXT > 0 {
        crate::kprintln!(
            "  test: {} tr{} made to fail (usbaddr fail), {} still to come",
            { FAILED_ON_PURPOSE }, if FAILED_ON_PURPOSE == 1 { "y" } else { "ies" }, { FAIL_NEXT }
        );
    }
}

/// Root port `p` of controller `c` holds a device this driver does not use (or that did not
/// answer): one line for it, and it is left alone until it is unplugged.
unsafe fn left_alone(c: usize, p: u32, hot: bool) {
    if p < 32 {
        LEFT[c] |= 1 << p;
    }
    note_other();
    let (v, pr, dc, ic) = LAST_OTHER;
    if (v, pr) == (0, 0) {
        crate::kprintln!("[xhci] port {}: device did not answer (descriptor not read), left alone until unplugged", p);
    } else {
        crate::kprintln!(
            "[xhci] port {}: device {:04x}:{:04x} ({}) {}not one this driver uses",
            p, v, pr, other_name(if dc != 0 { dc } else { ic }), if hot { "plugged in, " } else { "is " }
        );
    }
}

/// `service` for the root ports of controller `c` (the current one).
unsafe fn service_ports(c: usize) {
    {
        let pending = core::mem::replace(&mut PENDING[c], 0);
        for p in 1..=PORTS.min(31) {
            if pending & 1 << p == 0 {
                continue;
            }
            let sc = r32(portsc(p));
            // acknowledge the change bits (RW1C), touching nothing else
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | (sc & 0x00FE_0000));
            let present = (0..NDEV).any(|d| DEVS[d].slot != 0 && DEVS[d].port == p && DEVS[d].ctl == c);
            if sc & 1 == 0 {
                // unplugged: everything that hung off this root port goes
                if LEFT[c] & 1 << p != 0 {
                    LEFT[c] &= !(1 << p); // a device plugged in here next is tried afresh
                    CONNECTED = CONNECTED.saturating_sub(1);
                }
                for d in 0..NDEV {
                    if DEVS[d].slot != 0 && DEVS[d].port == p && DEVS[d].ctl == c {
                        detach(d, c);
                        CONNECTED = CONNECTED.saturating_sub(1);
                    }
                }
                crate::kprintln!("[xhci] port {}: device unplugged", p);
                continue;
            }
            if present || LEFT[c] & 1 << p != 0 {
                continue; // ours already, or left alone: the change was a reset or an enable bit
            }
            // plugged in: let the contacts settle (a connector bounces as it goes in), and only
            // go on if it is still connected 100 ms later
            crate::timer::busywait_ms(100);
            if r32(portsc(p)) & 1 == 0 {
                continue;
            }
            let sc = r32(portsc(p));
            // a USB 2 device needs a port reset to be enabled
            let mut sc = sc;
            if sc & 1 << 1 == 0 {
                w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | 1 << 4);
                wait(PORT_RESET_MS, || r32(portsc(p)) & 1 << 21 != 0);
                crate::timer::busywait_ms(20);
                sc = r32(portsc(p));
                w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | (sc & 0x00FE_0000));
            }
            if sc & 1 << 1 == 0 {
                continue;
            }
            let Some(fd) = free_dev() else {
                crate::kprintln!("[xhci] port {}: device plugged in, but no room for another device", p);
                continue;
            };
            PATH = (0, 0, 0);
            CONNECTED += 1;
            if attach(fd, p, sc) {
                PLUGS += 1;
                crate::kprintln!("[xhci] port {}: device plugged in and ready", p);
            } else {
                left_alone(c, p, true);
            }
        }
    }
}

/// A USB hub (class 9): select its configuration, tell the controller it is a hub (slot
/// context Hub bit and port count), power its ports, and enumerate whatever is connected
/// to each, with the route string and, behind a high-speed hub, the transaction
/// translator the controller needs for full/low-speed devices. Then its status-change
/// endpoint is started: a device plugged into or pulled out of one of its ports later is
/// served by `service` (`service_hub`), as a root port's is.
unsafe fn hub_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16, protocol: u8) -> bool {
    let slot = DEVS[d].slot;
    if speed >= 4 {
        // a SuperSpeed (or SuperSpeedPlus) hub speaks another descriptor and port-status format (and wants
        // SET_HUB_DEPTH); every USB 3 hub is also a USB 2 hub on a USB 2 root port, and
        // keyboards, mice, sticks and USB 2 adapters behind it are reached through that half
        DEVS[d].kind = KIND_HUB;
        NDEVS = NDEVS.max(d + 1);
        crate::kprintln!("[xhci] port {}: USB 3 hub {:04x}:{:04x}: its USB 2 half is used", port, vid, pid);
        return true;
    }
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let cval = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    // its status-change endpoint: interrupt IN, one bit per port (bit 0 = the hub itself)
    let (mut i, mut ep) = (0usize, None);
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 {
            break;
        }
        if DBUF.0[i + 1] == 5 && i + 7 <= total && ep.is_none() {
            let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
            if addr & 0x80 != 0 && attr & 3 == 3 {
                let mps = (u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF).max(1);
                ep = Some((addr & 0xF, mps, DBUF.0[i + 6]));
            }
        }
        i += len;
    }
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    if !control(d, 0xA0, 6, 0x2900, 0, 9) {
        return false; // the hub descriptor
    }
    let nports = (DBUF.0[2] as u32).min(15);
    let power_good_ms = DBUF.0[5] as u64 * 2;
    let ttt = ((DBUF.0[3] >> 5) & 3) as u32; // TT think time (wHubCharacteristics bits 6:5)
    let mtt = speed == 3 && protocol == 2;
    if mtt {
        control(d, 0x01, 11, 1, 0, 0); // SET_INTERFACE alt 1: one TT per port
    }
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    let dci = ep.map_or(0, |(n, _, _)| 2 * n as u32 + 1);
    *ctx(inb, 0).add(1) = 1 | if dci != 0 { 1 << dci } else { 0 }; // the slot (+ the endpoint)
    let sl = ctx(inb, 1);
    *sl = speed << 20 | dci.max(1) << 27 | 1 << 26 | (mtt as u32) << 25 | PATH.0; // ... Hub = 1, MTT
    *sl.add(1) = port << 16 | nports << 24; // root port, number of ports
    *sl.add(2) = PATH.1 | PATH.2 << 8 | ttt << 16; // ... TT think time
    let mut every_us = 0;
    if let Some((_, mps, binterval)) = ep {
        let interval = if speed >= 3 {
            (binterval as u32).clamp(1, 16) - 1
        } else {
            ((binterval as u32).max(1) * 8).ilog2().clamp(3, 10) // ms -> 125 us units, log2
        };
        every_us = (1u32 << interval) * 125;
        let ec = ctx(inb, 1 + dci as usize);
        *ec = interval << 16;
        *ec.add(1) = 3 << 1 | 7 << 3 | mps << 16; // CErr 3, interrupt IN
        let tr = addr_of!(EPIN[d]) as u64;
        *ec.add(2) = tr as u32 | 1;
        *ec.add(3) = (tr >> 32) as u32;
        *ec.add(4) = mps | mps << 16; // average TRB length, max ESIT payload
    }
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].kind = KIND_HUB;
    NDEVS = NDEVS.max(d + 1);
    HUBS += 1;
    HUB_PORTS += nports;
    HUBI[d] = (speed, PATH.1, PATH.2, nports);
    crate::kprintln!(
        "[xhci] port {}: USB hub {:04x}:{:04x} ({} speed), slot {}, {} ports",
        port, vid, pid, speed_name(speed), slot, nports
    );
    for hp in 1..=nports {
        control(d, 0x23, 3, 8, hp as u16, 0); // SET_FEATURE(PORT_POWER)
    }
    crate::timer::busywait_ms(power_good_ms.max(20) + 20);
    for hp in 1..=nports {
        let Some(fd) = free_dev() else { break };
        if matches!(hub_status(d, hp), Some((st, _)) if st & 1 != 0) {
            hub_port(d, hp, fd);
        }
    }
    if dci != 0 {
        // from now on a device plugged into (or pulled out of) one of its ports is reported
        // here and served by `service`, as a root port's is
        DEVS[d].dci = dci;
        HUB_WATCHED += 1;
        crate::kprintln!("[xhci] hub slot {}: watching its {} ports for hot-plug (every {} us)", slot, nports, every_us);
        queue_report(d);
    } else {
        crate::kprintln!("[xhci] hub slot {}: no status-change endpoint; its ports are read once", slot);
    }
    true
}

/// A USB stick or disk: mass storage class (8), SCSI transparent (6), Bulk-Only (0x50), with
/// a bulk IN and a bulk OUT endpoint. Configure them, then INQUIRY, TEST UNIT READY and READ
/// CAPACITY(10); like every disk here it is used for storage only if sector 0 is blank or
/// already AFRIVFS1; the stick the PC booted from (FAT) is listed and never written.
unsafe fn msc_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let cval = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    let (mut i, mut ours, mut ein, mut eout) = (0usize, false, None, None);
    // the first UAS interface (protocol 0x62): its number and alternate setting, and each pipe's
    // bulk endpoint (address, max packet), named by the Pipe Usage descriptor (0x24) after it
    let (mut uas, mut uas_if, mut last_ep) = (false, None::<(u8, u8)>, None::<(u8, u32)>);
    let mut pipes: [Option<(u8, u32)>; 4] = [None; 4];
    // each pipe's MaxStreams from the SuperSpeed Endpoint Companion (0x30) after its endpoint:
    // 2^n streams (0 below USB 3)
    let (mut last_ss, mut pstreams) = (0u8, [0u8; 4]);
    // and each one's bMaxBurst (0 to 15: 1 to 16 packets per burst): the UAS pipes by Pipe Usage,
    // the Bulk-Only in and out endpoints by which one the companion follows (1 in, 2 out)
    let (mut last_burst, mut pburst, mut bot_last, mut bot_burst) = (0u8, [0u8; 4], 0u8, (0u8, 0u8));
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 {
            break;
        }
        match DBUF.0[i + 1] {
            4 if i + 9 <= total => {
                ours = (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]) == (8, 6, 0x50);
                uas = (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]) == (8, 6, 0x62) && uas_if.is_none();
                last_ep = None;
                if ours {
                    MSC_IFACE = DBUF.0[i + 2];
                } else if DBUF.0[i + 5] == 8 && MSC_SEEN == (0, 0) {
                    MSC_SEEN = (DBUF.0[i + 6], DBUF.0[i + 7]);
                }
                if uas {
                    uas_if = Some((DBUF.0[i + 2], DBUF.0[i + 3]));
                }
            }
            5 if (ours || uas) && i + 7 <= total => {
                let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
                let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                bot_last = 0;
                if attr & 3 == 2 && ours {
                    if addr & 0x80 != 0 && ein.is_none() {
                        ein = Some((addr & 0xF, mps));
                        bot_last = 1;
                    } else if addr & 0x80 == 0 && eout.is_none() {
                        eout = Some((addr & 0xF, mps));
                        bot_last = 2;
                    }
                }
                last_ep = if attr & 3 == 2 && uas { Some((addr, mps)) } else { None };
                (last_ss, last_burst) = (0, 0);
            }
            0x30 if (ours || uas) && i + 4 <= total => {
                let burst = DBUF.0[i + 2].min(15);
                match bot_last {
                    1 => bot_burst.0 = burst,
                    2 => bot_burst.1 = burst,
                    _ => {}
                }
                bot_last = 0;
                if uas {
                    (last_ss, last_burst) = (DBUF.0[i + 3] & 0x1F, burst);
                }
            }
            0x24 if uas && i + 3 <= total => {
                let id = DBUF.0[i + 2] as usize;
                if let (1..=4, Some(e)) = (id, last_ep.take()) {
                    if pipes[id - 1].is_none() {
                        pipes[id - 1] = Some(e);
                        pstreams[id - 1] = last_ss;
                        pburst[id - 1] = last_burst;
                    }
                }
            }
            _ => {}
        }
        i += len;
    }
    let bot_eps = match (ein, eout) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    };
    // UAS only when there is no Bulk-Only way in, with all four pipes found and each one pointing
    // the way its name says; on a high-speed port without streams, on a USB 3 port with streams
    // when the controller has them (HCCPARAMS1 MaxPSASize, 4 contexts or more) and the status and
    // data pipes take at least 2 streams each
    let psa = if speed >= 4 { bar0(PCI_AT.0, PCI_AT.1, PCI_AT.2).map_or(0, |b| (r32(b + 0x10) >> 12) & 0xF) } else { 0 };
    let streams = speed >= 4 && psa >= 1 && pstreams[1..].iter().all(|&m| m >= 1);
    let uas_ok = bot_eps.is_none() && (speed == 3 || streams) && uas_if.is_some()
        && pipes.iter().enumerate().all(|(k, p)| p.is_some_and(|(a, _)| (a & 0x80 != 0) == (k == 1 || k == 2)));
    if bot_eps.is_none() && !uas_ok {
        return false;
    }
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false;
    }
    // (DCI, transfer ring, endpoint type, max packet, max burst) of each pipe used; the burst only on
    // a USB 3 port (Max Burst Size is 0 below it)
    let ss = speed >= 4;
    let mut list = [(0u32, 0u64, 0u32, 0u32, 0u32); 4];
    let n;
    if let Some(((epi, mpsi), (epo, mpso))) = bot_eps {
        let (bi, bo) = if ss { (bot_burst.0 as u32, bot_burst.1 as u32) } else { (0, 0) };
        list[0] = (2 * epi as u32 + 1, addr_of!(EPIN[d]) as u64, 6, mpsi, bi);
        list[1] = (2 * epo as u32, addr_of!(MSC_OUT_RING) as u64, 2, mpso, bo);
        n = 2;
        (MSC_EPI_ADDR, MSC_EPO_ADDR) = (0x80 | epi, epo);
    } else {
        let (ifn, alt) = uas_if.unwrap_or((0, 0));
        if alt != 0 && !control(d, 0x01, 11, alt as u16, ifn as u16, 0) {
            return false; // SET_INTERFACE to the UAS setting
        }
        MSC_IFACE = ifn;
        let rings = [addr_of!(MSC_OUT_RING) as u64, addr_of!(UAS_ST_RING) as u64, addr_of!(EPIN[d]) as u64, addr_of!(UAS_DO_RING) as u64];
        for k in 0..4 {
            let (a, mps) = pipes[k].unwrap_or((0, 0));
            let input = a & 0x80 != 0;
            let burst = if ss { pburst[k] as u32 } else { 0 };
            list[k] = (2 * (a & 0xF) as u32 + input as u32, rings[k], if input { 6 } else { 2 }, mps, burst);
        }
        n = 4;
    }
    *addr_of_mut!(PMSCOUT) = Prod { idx: 0, cycle: 1 };
    *addr_of_mut!(PUASST) = Prod { idx: 0, cycle: 1 };
    *addr_of_mut!(PUASDO) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(MSC_OUT_RING) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    core::ptr::write_bytes(addr_of_mut!(UAS_ST_RING) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    core::ptr::write_bytes(addr_of_mut!(UAS_DO_RING) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    let streams = n == 4 && streams;
    if streams {
        core::ptr::write_bytes(addr_of_mut!(UAS_SR) as *mut u8, 0, core::mem::size_of::<[Ring<NXFER>; 6]>());
        core::ptr::write_bytes(addr_of_mut!(UAS_SCA) as *mut u8, 0, core::mem::size_of::<[Sca; 3]>());
        *addr_of_mut!(PUAS_SR) = [Prod { idx: 0, cycle: 1 }; 6];
        // stream 0 is reserved; streams 1 and 2: a primary transfer ring each (SCT 1), DCS 1
        for k in 0..3 {
            for s in 0..2 {
                let r = addr_of!(UAS_SR[2 * k + s]) as u64;
                (*addr_of_mut!(UAS_SCA[k])).0[s + 1] = [r as u32 | 1 << 1 | 1, (r >> 32) as u32, 0, 0];
            }
        }
    }
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    let (mut flags, mut top) = (1u32, 0u32);
    for &(dci, ..) in &list[..n] {
        flags |= 1 << dci;
        top = top.max(dci);
    }
    *ctx(inb, 0).add(1) = flags;
    let sl = ctx(inb, 1);
    *sl = speed << 20 | top << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    for (k, &(dci, tr, ty, mps, burst)) in list[..n].iter().enumerate() {
        let ec = ctx(inb, 1 + dci as usize);
        *ec.add(1) = 3 << 1 | ty << 3 | burst << 8 | mps << 16;
        *ec.add(2) = tr as u32 | 1;
        *ec.add(3) = (tr >> 32) as u32;
        *ec.add(4) = 1024;
        if streams && k >= 1 {
            // MaxPStreams 1 (4 stream contexts), LSA: the dequeue pointer is the stream array
            let a = addr_of!(UAS_SCA[k - 1]) as u64;
            *ec = 1 << 10 | 1 << 15;
            *ec.add(2) = a as u32;
            *ec.add(3) = (a >> 32) as u32;
        }
    }
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    let (din, dout) = if n == 4 { (list[2].0, list[3].0) } else { (list[0].0, list[1].0) };
    MSC_UAS = n == 4;
    UAS_STREAMS = streams;
    // data in, data out: list 0 and 1 (Bulk-Only) or 2 and 3 (UAS)
    MSC_BURST = match (ss, n == 4) {
        (false, _) => (0, 0),
        (true, true) => (list[2].4 + 1, list[3].4 + 1),
        (true, false) => (list[0].4 + 1, list[1].4 + 1),
    };
    UAS_DCI = [list[0].0, list[1].0, list[2].0, list[3].0];
    DEVS[d].dci = din;
    DEVS[d].kind = KIND_MSC;
    NDEVS = NDEVS.max(d + 1);
    MSC_D = d;
    MSC_SLOT = slot;
    MSC_PORT = port;
    MSC_IN = din;
    MSC_OUT = dout;
    READY = true;
    if UAS_STREAMS {
        crate::kprintln!(
            "[xhci] port {}: USB disk {:04x}:{:04x} speaks only UAS: driven over USB Attached SCSI ({}, four pipes, streams 1 and 2)",
            port, vid, pid, if speed == 4 { "SuperSpeed" } else { "SuperSpeed Plus" }
        );
    } else if MSC_UAS {
        crate::kprintln!(
            "[xhci] port {}: USB disk {:04x}:{:04x} speaks only UAS: driven over USB Attached SCSI (high speed, four pipes, no streams)",
            port, vid, pid
        );
    }
    // INQUIRY: vendor and product, for lsusb
    if bot(&[0x12, 0, 0, 0, 36, 0], 36, true) {
        core::ptr::copy_nonoverlapping(mscbuf()[8..].as_ptr(), addr_of_mut!(MSC_NAME) as *mut u8, 24);
    }
    let mut ready = false;
    for _ in 0..10 {
        if bot(&[0, 0, 0, 0, 0, 0], 0, false) {
            ready = true; // TEST UNIT READY
            break;
        }
        // (a failed command's sense data (after power-on, UNIT ATTENTION) is read by bot())
        crate::timer::busywait_ms(100);
    }
    // READ CAPACITY(10): last LBA and block length, big-endian
    if !ready || !bot(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true) {
        crate::kprintln!("[xhci] port {}: USB storage {:04x}:{:04x} did not report its capacity", port, vid, pid);
        return true; // kept (it is set up), but not used for storage
    }
    let b = &mscbuf();
    let last10 = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    MSC_BLOCKS = last10 as u64 + 1;
    MSC_BS = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    (MSC_LONG, MSC_LONGCHK) = (false, 0);
    if last10 == 0xFFFF_FFFF {
        // Too big for READ CAPACITY(10): SERVICE ACTION IN(16) / READ CAPACITY(16), 32 bytes back.
        let cdb = [0x9E, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 0, 0];
        if !bot(&cdb, 32, true) {
            crate::kprintln!("[xhci] port {}: USB storage {:04x}:{:04x} did not report its capacity (READ CAPACITY(16))", port, vid, pid);
            return true;
        }
        let b = &mscbuf();
        MSC_BLOCKS = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) + 1;
        MSC_BS = u32::from_be_bytes([b[8], b[9], b[10], b[11]]);
        MSC_LONG = true;
        crate::kprintln!(
            "[xhci] USB storage: {} blocks, past what READ(10) can address: READ(16)/WRITE(16) for every read and write",
            { MSC_BLOCKS }
        );
    }
    let raw = matches!(MSC_BS, 512 | 1024 | 2048 | 4096) && msc_io(0, false);
    let usable = raw && {
        let s0 = &mscbuf()[..512];
        &s0[..8] == b"AFRIVFS1" || s0.iter().all(|&x| x == 0)
    };
    MSC_USABLE = usable;
    // a stick past what 10-byte commands reach is only written after the 16-byte check, which runs for a VFS
    // stick; a FAT32 one that large is left alone
    MSC_RAW = raw && !MSC_LONG;
    if usable && MSC_LONG {
        msc_long_check();
        // a VFS stick past 2 TiB that passed the 16-byte check is read and written as before (14 Nov 2026:
        // report home's MSC_RAW left it off, so msc_read/msc_write refused it and pc-usbbig's VFS was lost)
        MSC_RAW = MSC_USABLE;
    }
    let name = core::str::from_utf8(&*addr_of!(MSC_NAME)).unwrap_or("?");
    crate::kprintln!(
        "[xhci] port {}: USB storage {:04x}:{:04x} \"{}\", {} blocks of {} B ({} MiB), {}",
        port, vid, pid, name.trim(),
        { MSC_BLOCKS }, { MSC_BS }, MSC_BLOCKS * MSC_BS as u64 / (1024 * 1024),
        if usable { "blank or ours: used for storage" } else { "holds other data: left alone" }
    );
    msc_multi_check();
    true
}

/// At start-up: READ(10) of the first blocks in one command must equal the same blocks read one
/// at a time, else multi-block reads stay off (and writes with them). One boot line says which.
unsafe fn msc_multi_check() {
    (MSC_MULTI, MSC_MULTIW, MSC_RA_N, MSC_WB_N) = (0, 0, 0, 0);
    MSC_RSTATS = (0, 0, 0, 0);
    MSC_WSTATS = (0, 0, 0);
    if !MSC_USABLE {
        return;
    }
    let k = (MSC_RA_BYTES as u64 / MSC_BS as u64).min(MSC_BLOCKS) as u32;
    if k < 2 {
        MSC_MULTI = 2;
        return;
    }
    let ra = msc_ra();
    core::ptr::write_bytes(ra, 0xA5, MSC_RA_BYTES);
    let mut same = msc_rw(0, k, false, ra as u64);
    let read_ok = same;
    for b in 0..k as u64 {
        if !same {
            break;
        }
        let off = b as usize * MSC_BS as usize;
        same = msc_io(b, false)
            && mscbuf()[..MSC_BS as usize] == core::slice::from_raw_parts(ra.add(off), MSC_BS as usize)[..];
    }
    if same {
        (MSC_MULTI, MSC_RA_AT, MSC_RA_N) = (1, 0, k);
        crate::kprintln!(
            "[xhci] USB storage: multi-block reads on ({} of up to {} blocks per command; the first {} equal one at a time)",
            if MSC_LONG { "READ(16)" } else { "READ(10)" }, k, k
        );
    } else {
        MSC_MULTI = 2;
        MSC_MULTIW = 2;
        crate::kprintln!(
            "[xhci] USB storage: multi-block reads off ({}): one block per command",
            if read_ok { "they differed from one block at a time" } else { "the device refused one" }
        );
    }
}

/// The command block for `k` device blocks from `lba`: READ(10)/WRITE(10), or READ(16)/WRITE(16)
/// on a disk past 2^32 blocks (MSC_LONG). Returns the block and its length (10 or 16).
unsafe fn msc_cdb(lba: u64, k: u32, write: bool) -> ([u8; 16], usize) {
    let mut c = [0u8; 16];
    if MSC_LONG {
        c[0] = if write { 0x8A } else { 0x88 };
        c[2..10].copy_from_slice(&lba.to_be_bytes());
        c[10..14].copy_from_slice(&k.to_be_bytes());
        (c, 16)
    } else {
        c[0] = if write { 0x2A } else { 0x28 };
        c[2..6].copy_from_slice(&(lba as u32).to_be_bytes());
        c[7..9].copy_from_slice(&(k as u16).to_be_bytes());
        (c, 10)
    }
}

/// READ(10) / WRITE(10) (or the 16-byte forms) of `k` device blocks from `lba`, the data at `buf`.
unsafe fn msc_rw(lba: u64, k: u32, write: bool, buf: u64) -> bool {
    let (cdb, n) = msc_cdb(lba, k, write);
    bot_at(&cdb[..n], k * MSC_BS, !write, buf)
}

/// At start-up on a disk past 2^32 blocks: the last block is read, a pattern holding its address
/// written there with WRITE(16) and read back with READ(16), the old contents put back, and the
/// block a 32-bit address would wrap to must not hold the pattern. One boot line says which.
unsafe fn msc_long_check() {
    let bs = MSC_BS as usize;
    let last = MSC_BLOCKS - 1;
    let wrap = last & 0xFFFF_FFFF;
    let mut old = [0u8; 4096];
    let mut pat = [0u8; 4096];
    for (i, x) in pat[..bs].iter_mut().enumerate() {
        *x = (i as u8) ^ 0xC3 ^ last.to_be_bytes()[i % 8];
    }
    let saved = msc_io(last, false) && {
        old[..bs].copy_from_slice(&mscbuf()[..bs]);
        true
    };
    mscbuf()[..bs].copy_from_slice(&pat[..bs]);
    let wrote = saved && msc_io(last, true);
    let back = wrote && msc_io(last, false) && mscbuf()[..bs] == pat[..bs];
    let apart = back && msc_io(wrap, false) && mscbuf()[..bs] != pat[..bs];
    mscbuf()[..bs].copy_from_slice(&old[..bs]);
    let restored = !wrote || (msc_io(last, true) && msc_io(last, false) && mscbuf()[..bs] == old[..bs]);
    let ok = back && apart && restored;
    MSC_LONGCHK = if ok { 1 } else { 2 };
    if ok {
        crate::kprintln!(
            "[xhci] USB storage: 16-byte commands checked at block {} (written, read back, put back; block {} untouched)",
            last, wrap
        );
    } else {
        let why = if !saved { "the last block could not be read" } else if !back { "the last block did not read back as written" }
            else if !apart { "the write landed on a 32-bit wrapped block too" } else { "the last block could not be put back" };
        crate::kprintln!("[xhci] USB storage: 16-byte command check did not pass ({}): disk left alone", why);
        (MSC_USABLE, MSC_RAW) = (false, false);
    }
}

/// Device block `b` into MSCBUF: from the read-ahead when it holds it, else a new multi-block
/// read of up to 8 KiB from it (once the boot check passed), else one block.
unsafe fn msc_fetch(b: u64) -> bool {
    MSC_RSTATS.0 += 1;
    let bs = MSC_BS as usize;
    if b >= MSC_RA_AT && b < MSC_RA_AT + MSC_RA_N as u64 {
        let off = (b - MSC_RA_AT) as usize * bs;
        mscbuf()[..bs].copy_from_slice(&core::slice::from_raw_parts(msc_ra(), MSC_RA_BYTES)[off..off + bs]);
        MSC_RSTATS.1 += 1;
        return true;
    }
    if MSC_MULTI == 1 {
        let k = (MSC_RA_BYTES as u64 / MSC_BS as u64).min(MSC_BLOCKS - b) as u32;
        if k > 1 {
            MSC_RA_N = 0;
            if msc_rw(b, k, false, msc_ra() as u64) {
                (MSC_RA_AT, MSC_RA_N) = (b, k);
                mscbuf()[..bs].copy_from_slice(&core::slice::from_raw_parts(msc_ra(), MSC_RA_BYTES)[..bs]);
                MSC_RSTATS.2 += 1;
                return true;
            }
            MSC_MULTI = 2;
            crate::kprintln!("[xhci] USB storage: a multi-block read at block {} failed: one block per command from now on", b);
        }
    }
    MSC_RSTATS.3 += 1;
    let ok = msc_io(b, false);
    if !ok {
        msc_note("read", b);
    }
    ok
}

/// Device block `b` was written from `data` (or the write failed): keep the read-ahead true.
unsafe fn msc_ra_update(b: u64, data: &[u8], ok: bool) {
    if b >= MSC_RA_AT && b < MSC_RA_AT + MSC_RA_N as u64 {
        if ok {
            let off = (b - MSC_RA_AT) as usize * MSC_BS as usize;
            core::slice::from_raw_parts_mut(msc_ra(), MSC_RA_BYTES)[off..off + data.len()].copy_from_slice(data);
        } else {
            MSC_RA_N = 0; // the device's copy is unknown now
        }
    }
}

/// Send the gathered writes: one WRITE(10) when multi-block writes are on (the first one is read
/// back block by block and must be equal, else they go off and the run is rewritten one block
/// at a time), else one WRITE(10) per block.
unsafe fn msc_flush() -> bool {
    let (at, k) = (MSC_WB_AT, MSC_WB_N);
    if k == 0 {
        return true;
    }
    MSC_WB_N = 0;
    let wb = msc_wb() as *const u8;
    let bs = MSC_BS as usize;
    let mut done = false;
    if k > 1 && MSC_MULTI == 1 && MSC_MULTIW != 2 {
        if msc_rw(at, k, true, wb as u64) {
            MSC_WSTATS.1 += 1;
            done = true;
            if MSC_MULTIW == 0 {
                for j in 0..k as u64 {
                    let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
                    if !(msc_io(at + j, false) && mscbuf()[..bs] == s[..]) {
                        done = false;
                        break;
                    }
                }
                MSC_MULTIW = if done { 1 } else { 2 };
                if done {
                    crate::kprintln!("[xhci] USB storage: multi-block writes on ({} of up to {} blocks per command; the first {} read back equal)", if MSC_LONG { "WRITE(16)" } else { "WRITE(10)" }, MSC_RA_BYTES / bs, k);
                } else {
                    crate::kprintln!("[xhci] USB storage: multi-block writes off (read back differently: rewritten one block per command)");
                }
            }
        } else {
            MSC_MULTIW = 2;
            crate::kprintln!("[xhci] USB storage: a multi-block write at block {} failed: one block per command from now on", at);
        }
    }
    let mut ok = true;
    for j in 0..k as u64 {
        let s = core::slice::from_raw_parts(wb.add(j as usize * bs), bs);
        let good = done || {
            mscbuf()[..bs].copy_from_slice(s);
            MSC_WSTATS.2 += 1;
            msc_io(at + j, true)
        };
        msc_ra_update(at + j, s, good);
        ok = ok && good;
    }
    ok
}

/// Wait for the transfer event of the TRB at `trb` on endpoint `dci` of `slot`, serving every
/// other event. An event of the stick for another TRB (an answer that came after its
/// wait gave up, or the "stopped" event of one that was cut off) is set aside and counted, not
/// taken as this transfer's: before, it ended this wait with the wrong length and status.
unsafe fn wait_xfer(slot: u32, dci: u32, trb: u64) -> Option<Trb> {
    let t0 = crate::timer::cycles();
    while !past(t0, BULK_MS) {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == T_EV_TRANSFER && t.control >> 24 == slot {
                if (t.control >> 16) & 0x1F == dci && t.param & !0xF == trb {
                    return Some(t);
                }
                MSC_STALE += 1; // the stick runs one transfer at a time: any other is an old one
                continue;
            }
            handle(t);
        }
        core::hint::spin_loop();
    }
    None
}

/// One Bulk-Only transaction: the command block wrapper out, `len` bytes of data in or out
/// of MSCBUF, the command status wrapper in. True if the device reports success.
unsafe fn bot(cdb: &[u8], len: u32, dir_in: bool) -> bool {
    bot_at(cdb, len, dir_in, addr_of!(MSCBUF) as u64)
}

/// As bot, with the data in or out of the buffer at `buf` (MSCBUF, MSC_RA or MSC_WB).
unsafe fn bot_at(cdb: &[u8], len: u32, dir_in: bool, buf: u64) -> bool {
    let was_busy = BUSY;
    BUSY = true; // the timer poll stands aside; wait_xfer serves the other devices
    let ok = bot_inner(cdb, len, dir_in, buf);
    BUSY = was_busy;
    ok
}

/// Why the last stick command failed, and the sense data the stick gave for it (key, ASC, ASCQ): every failed
/// read or write says so in one line (`msc_note`), so a failure is never silent (reliability engineer, 14 Nov).
static mut MSC_WHY: &str = "";
static mut MSC_SENSE: Option<(u8, u8, u8)> = None;
static mut MSC_FAILS: u64 = 0;

/// `op` ("read", "written") of device block `b` did not succeed: a line for the first three and every 64th after.
/// The line avoids the words the storage suites treat as a fault ("failed", "timed out"): it is a note, and a
/// suite that injects faults on purpose may see it.
unsafe fn msc_note(op: &str, b: u64) {
    MSC_FAILS += 1;
    if MSC_FAILS <= 3 || MSC_FAILS % 64 == 0 {
        match MSC_SENSE {
            Some((k, a, q)) => crate::kprintln!(
                "[xhci] USB storage: block {} could not be {}: {} (sense key {:#x}, ASC {:#04x}/{:#04x}); {} such time(s) so far",
                b, op, MSC_WHY, k, a, q, { MSC_FAILS }
            ),
            None => crate::kprintln!("[xhci] USB storage: block {} could not be {}: {}; {} such time(s) so far", b, op, MSC_WHY, { MSC_FAILS }),
        }
    }
}

unsafe fn bot_inner(cdb: &[u8], len: u32, dir_in: bool, buf: u64) -> bool {
    MSC_SENSE = None;
    if MSC_UAS {
        let ok = uas_cmd(cdb, len, dir_in, buf);
        if !ok {
            MSC_WHY = "the UAS command did not complete";
        }
        return ok;
    }
    let w = &mut (*addr_of_mut!(CBW)).0;
    w.fill(0);
    w[0..4].copy_from_slice(&0x4342_5355u32.to_le_bytes()); // "USBC"
    let tag = MSC_TAG;
    MSC_TAG = MSC_TAG.wrapping_add(1);
    w[4..8].copy_from_slice(&tag.to_le_bytes());
    w[8..12].copy_from_slice(&len.to_le_bytes());
    w[12] = if dir_in { 0x80 } else { 0 };
    w[14] = cdb.len() as u8;
    w[15..15 + cdb.len()].copy_from_slice(cdb);
    let cc_of = |t: Option<Trb>| t.map(|t| t.status >> 24);
    let good = |c: Option<u32>| matches!(c, Some(CC_SUCCESS) | Some(CC_SHORT));
    // the command block wrapper
    if !good(cc_of(msc_xfer(false, addr_of!(CBW) as u64, 31))) {
        MSC_WHY = "the stick did not take the command (it was reset)";
        bot_reset(); // the device would not even take the command: start over
        return false;
    }
    // the data, if any; a STALL here is the device saying "no"; clear it and read the status
    if len > 0 {
        let c = cc_of(msc_xfer(dir_in, buf, len));
        if c == Some(CC_STALL) {
            unwedge(dir_in);
        } else if !good(c) {
            MSC_WHY = if c.is_none() { "the data stage got no answer (the stick was reset)" } else { "the data stage got an error (the stick was reset)" };
            bot_reset();
            return false;
        }
    }
    // the command status wrapper (once more after clearing a stall)
    let mut c = cc_of(msc_xfer(true, addr_of!(CSW) as u64, 13));
    if c == Some(CC_STALL) {
        unwedge(true);
        c = cc_of(msc_xfer(true, addr_of!(CSW) as u64, 13));
    }
    let cs = &(*addr_of!(CSW)).0;
    let valid = good(c) && u32::from_le_bytes([cs[0], cs[1], cs[2], cs[3]]) == 0x5342_5355
        && u32::from_le_bytes([cs[4], cs[5], cs[6], cs[7]]) == tag;
    if !valid || cs[12] == 2 {
        MSC_WHY = if c.is_none() { "no status came back (the stick was reset)" } else { "a status that could not be trusted (the stick was reset)" };
        bot_reset(); // phase error, or a status we cannot trust: Bulk-Only reset recovery
        return false;
    }
    // the status is kept before REQUEST SENSE: its own wrapper lands in CSW and says "passed",
    // which must not turn this failed command into a good one (its data would be taken as read)
    let status = cs[12];
    if status != 0 && cdb[0] != 0x03 {
        // a failed command: read its sense data, or many sticks refuse the next one
        let saved = *mscbuf();
        let got = bot_inner(&[0x03, 0, 0, 0, 18, 0], 18, true, addr_of!(MSCBUF) as u64);
        let s = &mscbuf()[..18];
        let sense = if got && s[0] & 0x7E == 0x70 { Some((s[2] & 0xF, s[12], s[13])) } else { None };
        *mscbuf() = saved;
        MSC_WHY = "the stick refused the command";
        MSC_SENSE = sense;
    }
    status == 0
}

/// One transfer queued on UAS pipe `k` (0 command, 1 status, 2 data in, 3 data out).
unsafe fn uas_queue(k: usize, buf: u64, len: u32) {
    let d = MSC_D;
    let s = UAS_TAG as usize; // with streams, the stream this command's tag names
    let (ring, p): (*mut Trb, *mut Prod) = match k {
        0 => (addr_of_mut!(MSC_OUT_RING.0) as *mut Trb, addr_of_mut!(PMSCOUT)),
        _ if UAS_STREAMS => (addr_of_mut!(UAS_SR[2 * (k - 1) + s - 1].0) as *mut Trb, addr_of_mut!(PUAS_SR[2 * (k - 1) + s - 1])),
        1 => (addr_of_mut!(UAS_ST_RING.0) as *mut Trb, addr_of_mut!(PUASST)),
        2 => (addr_of_mut!(EPIN[d].0) as *mut Trb, addr_of_mut!(PEPIN[d])),
        _ => (addr_of_mut!(UAS_DO_RING.0) as *mut Trb, addr_of_mut!(PUASDO)),
    };
    let dci = UAS_DCI[k];
    let isp = if dci & 1 == 1 { 1 << 2 } else { 0 }; // IN: an event on a short packet too
    push(ring, NXFER, &mut *p, buf, len, T_NORMAL << 10 | 1 << 5 | isp);
    let sid = if UAS_STREAMS && k > 0 { (s as u32) << 16 } else { 0 };
    doorbell(MSC_SLOT, dci | sid);
}

/// After a UAS command over streams went wrong: each stream pipe still holding a transfer is
/// stopped (a halted one reset and its halt cleared), and both streams' dequeue pointers moved to
/// where the next transfer will be queued, so no old transfer waits on a stream for a later tag.
unsafe fn uas_flush() {
    let outb = addr_of_mut!(OUTCTX[MSC_D].0) as *mut u8;
    for k in 1..4 {
        let dci = UAS_DCI[k];
        match read_volatile(ctx(outb, dci as usize)) & 7 {
            1 => {
                command(0, T_STOP_EP << 10 | MSC_SLOT << 24 | dci << 16);
            }
            2 => {
                command(0, T_RESET_EP << 10 | MSC_SLOT << 24 | dci << 16);
                control(MSC_D, 0x02, 1, 0, (dci / 2 | (dci & 1) << 7) as u16, 0);
            }
            _ => {}
        }
        for s in 0..2 {
            let i = 2 * (k - 1) + s;
            let p = (*addr_of!(PUAS_SR))[i];
            let deq = addr_of!(UAS_SR[i]) as u64 + 16 * p.idx as u64;
            command_st(deq | 1 << 1 | p.cycle as u64, (s as u32 + 1) << 16, T_SET_TR_DEQ << 10 | MSC_SLOT << 24 | dci << 16);
        }
    }
    UAS_EV = [None; 32];
}

/// One SCSI command over UAS with streams: the status and the data queued on the command's
/// stream, then the command IU; the SENSE IU with its tag ends it (true when GOOD), and when it
/// carries data the data transfer has ended too.
unsafe fn uas_cmd_streams(len: u32, dir_in: bool, buf: u64) -> bool {
    let tag = UAS_TAG;
    let good = |c: Option<u32>| matches!(c, Some(CC_SUCCESS) | Some(CC_SHORT));
    let sb = addr_of!(UAS_ST) as u64;
    let k = if dir_in { 2 } else { 3 };
    uas_queue(1, sb, 512);
    if len > 0 {
        uas_queue(k, buf, len);
    }
    uas_queue(0, addr_of!(CBW) as u64, 32);
    let ok = 'run: {
        if !good(uas_wait(0)) {
            break 'run None;
        }
        for _ in 0..4 {
            if !good(uas_wait(1)) {
                break 'run None;
            }
            let (id, mine, status) = {
                let s = &(*addr_of!(UAS_ST)).0;
                (s[0], u16::from_be_bytes([s[2], s[3]]) == tag, s[6])
            };
            match id {
                // a SENSE IU that is not GOOD may end the command before its data moved
                3 if mine && status == 0 => break 'run if len == 0 || good(uas_wait(k)) { Some(true) } else { None },
                3 if mine => break 'run if len == 0 { Some(false) } else { None },
                3 => uas_queue(1, sb, 512), // an earlier command's status: wait on for ours
                _ => break 'run None,       // a RESPONSE IU (the command refused)
            }
        }
        None
    };
    match ok {
        Some(g) => g,
        None => {
            MSC_RECOVERIES += 1;
            uas_flush();
            false
        }
    }
}

/// The completion code of the next transfer on UAS pipe `k`. Events of the other three pipes
/// that come meanwhile are kept for their own wait; every other event is served.
unsafe fn uas_wait(k: usize) -> Option<u32> {
    let dcis = UAS_DCI;
    let dci = dcis[k];
    if let Some(t) = UAS_EV[dci as usize] {
        UAS_EV[dci as usize] = None;
        return Some(t.status >> 24);
    }
    let t0 = crate::timer::cycles();
    while !past(t0, BULK_MS) {
        if let Some(t) = next_event() {
            let e = (t.control >> 16) & 0x1F;
            if (t.control >> 10) & 0x3F == T_EV_TRANSFER && t.control >> 24 == MSC_SLOT && dcis.contains(&e) {
                if e == dci {
                    return Some(t.status >> 24);
                }
                UAS_EV[e as usize] = Some(t);
                continue;
            }
            handle(t);
        }
        core::hint::spin_loop();
    }
    None
}

/// One SCSI command over UAS without streams: the status pipe armed, the 32-byte command IU sent,
/// then READ READY / WRITE READY on the status pipe lets the data go (the status pipe armed again
/// beside it), and the SENSE IU with this command's tag ends it: true when its status is GOOD. A
/// failed command's sense data comes inside that IU, so no REQUEST SENSE follows.
unsafe fn uas_cmd(cdb: &[u8], len: u32, dir_in: bool, buf: u64) -> bool {
    UAS_TAG = if UAS_STREAMS { UAS_TAG % 2 + 1 } else { UAS_TAG % 0xFFFE + 1 };
    let tag = UAS_TAG;
    let iu = &mut (*addr_of_mut!(CBW)).0;
    iu.fill(0);
    iu[0] = 1; // COMMAND IU; task attribute SIMPLE, LUN 0
    iu[2..4].copy_from_slice(&tag.to_be_bytes());
    iu[16..16 + cdb.len().min(16)].copy_from_slice(&cdb[..cdb.len().min(16)]);
    UAS_EV = [None; 32];
    if UAS_STREAMS {
        return uas_cmd_streams(len, dir_in, buf);
    }
    let good = |c: Option<u32>| matches!(c, Some(CC_SUCCESS) | Some(CC_SHORT));
    let sb = addr_of!(UAS_ST) as u64;
    uas_queue(1, sb, 512);
    uas_queue(0, addr_of!(CBW) as u64, 32);
    if !good(uas_wait(0)) {
        MSC_RECOVERIES += 1;
        return false;
    }
    let mut data_done = len == 0;
    for _ in 0..4 {
        if !good(uas_wait(1)) {
            MSC_RECOVERIES += 1;
            return false;
        }
        let (id, mine, status) = {
            let s = &(*addr_of!(UAS_ST)).0;
            (s[0], u16::from_be_bytes([s[2], s[3]]) == tag, s[6])
        };
        match id {
            3 if mine => return status == 0, // SENSE IU
            6 | 7 if mine && !data_done && (id == 6) == dir_in => {
                // READ READY / WRITE READY: the data now, and the status pipe armed for the end
                data_done = true;
                let k = if dir_in { 2 } else { 3 };
                uas_queue(k, buf, len);
                uas_queue(1, sb, 512);
                if !good(uas_wait(k)) {
                    MSC_RECOVERIES += 1;
                    return false;
                }
            }
            3 => uas_queue(1, sb, 512), // an earlier command's status: wait on for ours
            _ => {
                MSC_RECOVERIES += 1;
                return false; // a RESPONSE IU (the command refused) or a READY out of turn
            }
        }
    }
    false
}

/// One bulk transfer on the stick's IN or OUT endpoint; its completion event.
unsafe fn msc_xfer(dir_in: bool, buf: u64, len: u32) -> Option<Trb> {
    let d = MSC_D;
    if dir_in {
        let at = push_held(addr_of_mut!(EPIN[d].0) as *mut Trb, NXFER, &mut (*addr_of_mut!(PEPIN))[d], buf, len, T_NORMAL << 10 | 1 << 5 | 1 << 2, false);
        doorbell(MSC_SLOT, MSC_IN);
        wait_xfer(MSC_SLOT, MSC_IN, at as u64)
    } else {
        let at = push_held(addr_of_mut!(MSC_OUT_RING.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PMSCOUT), buf, len, T_NORMAL << 10 | 1 << 5, false);
        doorbell(MSC_SLOT, MSC_OUT);
        wait_xfer(MSC_SLOT, MSC_OUT, at as u64)
    }
}

/// A halted (stalled) endpoint: xHCI Reset Endpoint, CLEAR_FEATURE(ENDPOINT_HALT) to the
/// device, then move the controller's dequeue pointer past the stalled transfer to where the
/// next one will be queued (Set TR Dequeue Pointer). An endpoint still running (a transfer that
/// timed out, or a Bulk-Only reset after a bad status) is stopped first, as uas_flush does: Reset
/// Endpoint and Set TR Dequeue are refused on a running endpoint (xHCI 4.6.8, 4.6.10), which left
/// the lost transfer queued for the stick to answer late.
unsafe fn unwedge(dir_in: bool) {
    let d = MSC_D;
    let (dci, addr) = if dir_in { (MSC_IN, MSC_EPI_ADDR) } else { (MSC_OUT, MSC_EPO_ADDR) };
    MSC_RECOVERIES += 1;
    match read_volatile(ctx(addr_of_mut!(OUTCTX[d].0) as *mut u8, dci as usize)) & 7 {
        1 => {
            command(0, T_STOP_EP << 10 | MSC_SLOT << 24 | dci << 16);
        }
        2 => {
            command(0, T_RESET_EP << 10 | MSC_SLOT << 24 | dci << 16);
        }
        _ => {}
    }
    control(d, 0x02, 1, 0, addr as u16, 0);
    let (ring, prod) = if dir_in {
        (addr_of!(EPIN[d].0) as u64, (*addr_of!(PEPIN))[d])
    } else {
        (addr_of!(MSC_OUT_RING.0) as u64, *addr_of!(PMSCOUT))
    };
    let deq = ring + 16 * prod.idx as u64;
    command(deq | prod.cycle as u64, T_SET_TR_DEQ << 10 | MSC_SLOT << 24 | dci << 16);
}

/// Bulk-Only Mass Storage Reset (class request 0xFF to the interface), then both endpoints
/// cleared: the way back after the host and the stick disagree about the phase.
unsafe fn bot_reset() {
    MSC_RECOVERIES += 1;
    control(MSC_D, 0x21, 0xFF, 0, MSC_IFACE as u16, 0);
    unwedge(true);
    unwedge(false);
}

/// The stick's 4 KiB transfer buffer.
fn mscbuf() -> &'static mut [u8; 4096] {
    // SAFETY: single core; only bot() and its callers use it, one transaction at a time.
    unsafe { &mut (*addr_of_mut!(MSCBUF)).0 }
}

/// `lsusb late`: a test switch for a Bulk-Only stick. Reads block 0, then sends a TEST UNIT READY
/// whose two answers (the command's and its status's) are left unread on the event ring, as when
/// a wait gives up and the stick answers after, then reads block 0 again. Right only if the second
/// read gives the same data with no recovery and both late events were set aside; the old
/// wait_xfer took them as the read's own and needed a Bulk-Only reset. Interrupts off.
pub fn late_check() {
    // SAFETY: mainline; BUSY and interrupts off keep the timer poll off the event ring.
    unsafe {
        if !msc_present() || MSC_UAS {
            crate::kprintln!("lsusb late: {}", if MSC_UAS { "the stick speaks UAS; this check is for Bulk-Only" } else { "no USB stick in use" });
            return;
        }
        let Some(home) = on(DEVS[MSC_D].ctl) else { return };
        crate::cpu::cli();
        let was_busy = BUSY;
        BUSY = true;
        let bs = MSC_BS as usize;
        let ok1 = msc_flush() && msc_io(0, false);
        let first = *mscbuf();
        let (stale0, rec0) = (MSC_STALE, MSC_RECOVERIES);
        // TEST UNIT READY: its wrapper out and its status in, queued but never waited for
        let w = &mut (*addr_of_mut!(CBW)).0;
        w.fill(0);
        w[0..4].copy_from_slice(&0x4342_5355u32.to_le_bytes()); // "USBC"
        w[4..8].copy_from_slice(&MSC_TAG.to_le_bytes());
        MSC_TAG = MSC_TAG.wrapping_add(1);
        w[14] = 6; // a 6-byte command block of zeros: TEST UNIT READY
        let d = MSC_D;
        push(addr_of_mut!(MSC_OUT_RING.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PMSCOUT), addr_of!(CBW) as u64, 31, T_NORMAL << 10 | 1 << 5);
        doorbell(MSC_SLOT, MSC_OUT);
        push(addr_of_mut!(EPIN[d].0) as *mut Trb, NXFER, &mut (*addr_of_mut!(PEPIN))[d], addr_of!(CSW) as u64, 13, T_NORMAL << 10 | 1 << 5 | 1 << 2);
        doorbell(MSC_SLOT, MSC_IN);
        let t0 = crate::timer::cycles();
        while !past(t0, 100) {
            core::hint::spin_loop(); // both answers come in and stay unread
        }
        let ok2 = msc_io(0, false);
        let same = mscbuf()[..bs] == first[..bs];
        let (stale, rec) = (MSC_STALE - stale0, MSC_RECOVERIES - rec0);
        BUSY = was_busy;
        crate::cpu::sti();
        select(home);
        let ok = ok1 && ok2 && same && stale == 2 && rec == 0;
        crate::kprintln!(
            "lsusb late: 2 answers were left unread; the next read of block 0 {} ({}), {} late event(s) set aside, {} recoveries: {}",
            if ok2 { "passed" } else { "FAILED" },
            if same { "same data" } else { "DIFFERENT data" },
            stale,
            rec,
            if ok { "OK" } else { "WRONG" }
        );
    }
}

/// `lsusb late0`: a test switch for endpoint 0. On the stick (or the first device), queues a
/// GET_STATUS whose answer is left unread on the event ring, as when a request's wait gives up
/// and the device answers after, then reads the device descriptor. Right only if the read
/// passed, took its own answer (not the GET_STATUS's, as the old wait_control did) and set one
/// late event aside. Interrupts off.
pub fn late0_check() {
    // SAFETY: mainline; BUSY and interrupts off keep the timer poll off the event ring.
    unsafe {
        let d = if msc_present() { MSC_D } else { match (0..NDEV).find(|&d| DEVS[d].slot != 0) { Some(d) => d, None => {
            crate::kprintln!("lsusb late0: no USB device");
            return;
        } } };
        let Some(home) = on(DEVS[d].ctl) else { return };
        crate::cpu::cli();
        let was_busy = BUSY;
        BUSY = true;
        let stale0 = CTL_STALE;
        // GET_STATUS (device, 2 bytes in): setup, data, status, rung but never waited for
        let ring = addr_of_mut!(EP0[d].0) as *mut Trb;
        let p = &mut (*addr_of_mut!(PEP0))[d];
        let first = push_held(ring, NXFER, p, 0x80 | 2 << 48, 8, T_SETUP << 10 | 1 << 6 | 3 << 16, true);
        push(ring, NXFER, p, addr_of!(DBUF) as u64, 2, T_DATA << 10 | 1 << 16);
        push(ring, NXFER, p, 0, 0, T_STATUS << 10 | 1 << 5);
        release(first);
        doorbell(DEVS[d].slot, 1);
        let t0 = crate::timer::cycles();
        while !past(t0, 100) {
            core::hint::spin_loop(); // its answer comes in and stays unread
        }
        (&mut (*addr_of_mut!(DBUF)).0)[..18].fill(0);
        let ok = control(d, 0x80, 6, 0x0100, 0, 18);
        let (len, ty) = (DBUF.0[0], DBUF.0[1]);
        let own = CTL_TOOK == CTL_WANT;
        let stale = CTL_STALE - stale0;
        BUSY = was_busy;
        crate::cpu::sti();
        select(home);
        let good = ok && len == 18 && ty == 1 && own && stale == 1;
        crate::kprintln!(
            "lsusb late0: an answer was left unread on endpoint 0; the next request {} (descriptor of {} bytes, type {}), took {} answer, {} late event(s) set aside: {}",
            if ok { "passed" } else { "FAILED" },
            len,
            ty,
            if own { "its own" } else { "ANOTHER request's" },
            stale,
            if good { "OK" } else { "WRONG" }
        );
    }
}

/// `lsusb stall0`: a test switch for endpoint 0. On the stick (or the first device), asks for a
/// descriptor of a type no device has (0x7F), which the device refuses with a STALL that halts
/// endpoint 0 in the controller, then reads the device descriptor. Right only if the refusal was
/// a STALL, endpoint 0 was reset once and the read passed; the old control() left endpoint 0
/// halted, so the read (and every later request to the device) timed out. Interrupts off.
pub fn stall0_check() {
    // SAFETY: mainline; BUSY and interrupts off keep the timer poll off the event ring.
    unsafe {
        let d = if msc_present() { MSC_D } else { match (0..NDEV).find(|&d| DEVS[d].slot != 0) { Some(d) => d, None => {
            crate::kprintln!("lsusb stall0: no USB device");
            return;
        } } };
        let Some(home) = on(DEVS[d].ctl) else { return };
        crate::cpu::cli();
        let was_busy = BUSY;
        BUSY = true;
        let r0 = CTL_RESETS;
        CTL_QUIET = true;
        let refused = !control(d, 0x80, 6, 0x7F00, 0, 18);
        CTL_QUIET = false;
        let cc = if refused { LAST_CC } else { CC_SUCCESS };
        (&mut (*addr_of_mut!(DBUF)).0)[..18].fill(0);
        let ok = control(d, 0x80, 6, 0x0100, 0, 18);
        let (len, ty) = (DBUF.0[0], DBUF.0[1]);
        let resets = CTL_RESETS - r0;
        BUSY = was_busy;
        crate::cpu::sti();
        select(home);
        let good = refused && cc == CC_STALL && resets == 1 && ok && len == 18 && ty == 1;
        crate::kprintln!(
            "lsusb stall0: a request the device refuses was {} (completion code {}); endpoint 0 was reset {} time(s); the next request {} (descriptor of {} bytes, type {}): {}",
            if refused { "refused" } else { "ANSWERED" },
            cc,
            resets,
            if ok { "passed" } else { "FAILED" },
            len,
            ty,
            if good { "OK" } else { "WRONG" }
        );
    }
}

/// `lsusb stallhub`: `stall0_check`'s request, but aimed at a HUB's own endpoint 0 (not a child
/// device's), then a real hub operation (GET_PORT_STATUS on port 1) to prove the hub still
/// manages its ports after recovery. `control()` and `ep0_recover()` are the same code
/// `hub_status`/`hub_port`/`service_hub` already call for every port request, so this needed no
/// new recovery path -- only a check that it also covers the hub's own control endpoint.
pub fn stallhub_check() {
    // SAFETY: mainline; BUSY and interrupts off keep the timer poll off the event ring.
    unsafe {
        let Some(d) = (0..NDEV).find(|&d| DEVS[d].slot != 0 && DEVS[d].kind == KIND_HUB) else {
            crate::kprintln!("lsusb stallhub: no USB hub in use");
            return;
        };
        let Some(home) = on(DEVS[d].ctl) else { return };
        crate::cpu::cli();
        let was_busy = BUSY;
        BUSY = true;
        let r0 = CTL_RESETS;
        CTL_QUIET = true;
        let refused = !control(d, 0x80, 6, 0x7F00, 0, 18);
        CTL_QUIET = false;
        let cc = if refused { LAST_CC } else { CC_SUCCESS };
        (&mut (*addr_of_mut!(DBUF)).0)[..18].fill(0);
        let ok = control(d, 0x80, 6, 0x0100, 0, 18);
        let (len, ty) = (DBUF.0[0], DBUF.0[1]);
        let port_ok = hub_status(d, 1).is_some();
        let resets = CTL_RESETS - r0;
        BUSY = was_busy;
        crate::cpu::sti();
        select(home);
        let good = refused && cc == CC_STALL && resets == 1 && ok && len == 18 && ty == 1 && port_ok;
        crate::kprintln!(
            "lsusb stallhub: a request the hub refuses was {} (completion code {}); its endpoint 0 was reset {} time(s); the next request {} (descriptor of {} bytes, type {}); its port 1 status {}: {}",
            if refused { "refused" } else { "ANSWERED" },
            cc,
            resets,
            if ok { "passed" } else { "FAILED" },
            len,
            ty,
            if port_ok { "read" } else { "FAILED" },
            if good { "OK" } else { "WRONG" }
        );
    }
}

/// READ(10) / WRITE(10) (or the 16-byte forms) of one device block at `lba` through MSCBUF.
unsafe fn msc_io(lba: u64, write: bool) -> bool {
    let (cdb, n) = msc_cdb(lba, 1, write);
    bot(&cdb[..n], MSC_BS, !write)
}

fn msc_per() -> u64 {
    // SAFETY: scalar read.
    (unsafe { MSC_BS } as u64 / 512).max(1)
}

/// True when a USB stick or disk holds (or may hold) the VFS.
pub fn msc_present() -> bool {
    // SAFETY: scalar reads.
    unsafe { MSC_SLOT != 0 && MSC_USABLE }
}

/// True when the stick answers sector reads and writes, whatever it holds (esp.rs: its FAT32). The VFS asks
/// msc_present.
pub fn msc_attached() -> bool {
    // SAFETY: scalar reads.
    unsafe { MSC_SLOT != 0 && MSC_RAW }
}

/// The stick's own block size in bytes (512 or 4096), for reading its partition table (esp.rs firmware files).
pub fn msc_block_size() -> u64 {
    // SAFETY: scalar read.
    (unsafe { MSC_BS } as u64).max(512)
}

/// Capacity in 512-byte sectors.
pub fn msc_capacity() -> u64 {
    // SAFETY: scalar read.
    (unsafe { MSC_BLOCKS }) * msc_per()
}

/// One 512-byte sector of the stick. The VFS (disk.rs) and the table backup (gpt.rs) check msc_present first;
/// esp.rs's FAT32 reader checks msc_attached.
pub fn msc_read(n: u64, out: &mut [u8; 512]) -> bool {
    if !msc_attached() || n >= msc_capacity() {
        return false;
    }
    // SAFETY: mainline (the VFS); bot() keeps the timer poll off the event ring meanwhile.
    unsafe {
        let Some(home) = on(DEVS[MSC_D].ctl) else {
            (MSC_WHY, MSC_SENSE) = ("the USB controller was being switched (try again)", None);
            msc_note("read", n / msc_per());
            return false;
        };
        // writes still gathered go to the device before anything is read
        let ok = msc_flush() && msc_fetch(n / msc_per());
        select(home);
        if !ok {
            return false;
        }
        let off = (n % msc_per()) as usize * 512;
        out.copy_from_slice(&mscbuf()[off..off + 512]);
    }
    true
}

/// READ ONLY, for `updatecheck` (AFR-0010, 13 Nov 2026): a USB stick that holds other data (a FAT32 stick
/// with an update file on it) is left alone for storage, so `msc_present` says no; it may still be READ.
/// There is deliberately no matching write. This door cannot write by construction: it refuses while any
/// write is gathered and never calls `msc_flush` (architect, 14 Nov 2026), so it does not lean on what
/// `msc_write` accepts. Capacity in 512-byte sectors, or 0 when no stick reported a block size we can read.
pub fn msc_read_only_capacity() -> u64 {
    // SAFETY: scalar reads.
    unsafe {
        if MSC_SLOT == 0 || !matches!(MSC_BS, 512 | 1024 | 2048 | 4096) {
            return 0;
        }
    }
    msc_capacity()
}

/// One 512-byte sector of any attached stick, ours or not (see `msc_read_only_capacity`). Reads only:
/// no path from here issues a WRITE command.
pub fn msc_read_only(n: u64, out: &mut [u8; 512]) -> bool {
    if n >= msc_read_only_capacity() {
        return false;
    }
    // SAFETY: as msc_read, but with no flush: while any write is gathered it refuses instead.
    unsafe {
        if MSC_WB_N != 0 {
            return false;
        }
        let Some(home) = on(DEVS[MSC_D].ctl) else { return false };
        let ok = msc_fetch(n / msc_per());
        select(home);
        if !ok {
            return false;
        }
        let off = (n % msc_per()) as usize * 512;
        out.copy_from_slice(&mscbuf()[off..off + 512]);
    }
    true
}

/// Select controller `c` for a device operation started from outside the driver; the controller
/// that was current, to `select` again afterwards. None while a select is half done (only an
/// interrupt can see that).
unsafe fn on(c: usize) -> Option<usize> {
    if SWITCHING {
        return None;
    }
    let home = CUR;
    select(c);
    Some(home)
}

/// Every write still gathered for the stick sent to it now (report save: what was written is on the stick).
pub fn msc_settle() -> bool {
    if !msc_attached() {
        return false;
    }
    // SAFETY: as msc_read.
    unsafe {
        let Some(home) = on(DEVS[MSC_D].ctl) else { return false };
        let ok = msc_flush();
        select(home);
        ok
    }
}

/// One 512-byte sector onto the stick. Its callers, and no others (architect, 14 Nov: a person's FAT32 stick is
/// written only by `report save`; scripts/pc-reportsave-test.sh greps this list):
/// - disk.rs, the VFS, after `msc_present` (a VFS or blank stick);
/// - gpt.rs, the partition-table backup, after `msc_present`;
/// - esp.rs, `BootStick`, made only in `report_save` after the boot-medium and boot-file checks.
pub fn msc_write(n: u64, data: &[u8; 512]) -> bool {
    if !msc_attached() || n >= msc_capacity() {
        return false;
    }
    // SAFETY: as msc_read.
    unsafe {
        let Some(home) = on(DEVS[MSC_D].ctl) else {
            (MSC_WHY, MSC_SENSE) = ("the USB controller was being switched (try again)", None);
            msc_note("written", n / msc_per());
            return false;
        };
        // the VFS saves its sectors in order, so a run of device blocks is gathered in MSC_WB
        // (up to 8 KiB; one block while multi-block writes are off). A block bigger than a
        // sector (1, 2 or 4 KiB) is read once when the run reaches it and its sectors changed
        // in place, so a 4 KiB disk is written once per block, not once per sector. The run is
        // sent when it is full and its last block complete, on a gap, before any read, or at the
        // VFS's last sector (or a lone write past the VFS area: the self-test, a table backup).
        let (per, bs) = (msc_per(), MSC_BS as usize);
        let (b, off) = (n / per, (n % per) as usize * 512);
        let cap = if MSC_MULTI == 1 && MSC_MULTIW != 2 { (MSC_RA_BYTES / bs) as u32 } else { 1 };
        let mut ok = true;
        let tail = MSC_WB_AT + MSC_WB_N as u64; // the block after the run
        if MSC_WB_N > 0 && b + 1 != tail && !(b == tail && MSC_WB_N < cap) {
            ok = msc_flush();
        }
        let new = MSC_WB_N == 0 || b == MSC_WB_AT + MSC_WB_N as u64;
        // a new block's other sectors as they are on the device (msc_fetch: the read-ahead,
        // kept equal to every block flushed, or the device)
        let got = !new || per == 1 || msc_fetch(b);
        if got {
            let wb = core::slice::from_raw_parts_mut(msc_wb(), MSC_RA_BYTES);
            if MSC_WB_N == 0 {
                MSC_WB_AT = b;
            }
            if new {
                if per > 1 {
                    let i = MSC_WB_N as usize * bs;
                    wb[i..i + bs].copy_from_slice(&mscbuf()[..bs]);
                }
                MSC_WB_N += 1;
            }
            let i = (b - MSC_WB_AT) as usize * bs + off;
            wb[i..i + 512].copy_from_slice(data);
            MSC_WSTATS.0 += 1;
            let last = n >= 2 * crate::vfs::MAX_FILES as u64;
            if last || (off + 512 == bs && MSC_WB_N >= cap) {
                ok = msc_flush() && ok;
            }
        }
        let ok = ok && got;
        if !ok {
            msc_note("written", n / msc_per());
        }
        select(home);
        ok
    }
}

/// A USB Ethernet adapter: CDC-NCM (newer phones' USB tethering, 2.5G dongles), CDC-ECM (most
/// USB Ethernet dongles and docks) or RNDIS (older Android tethering, dongles made for Windows).
/// Find a configuration with a communication interface of the kind tried and its data interface
/// (class 0x0A, named by the Union descriptor when there is one) with a bulk IN and a bulk OUT
/// endpoint (NCM and ECM: in an alternate setting >= 1; RNDIS: in setting 0). Tried in the order
/// NCM, ECM, RNDIS (architect's N1, 13 Nov 2026; Linux binds cdc_ncm first too): if any NCM step
/// fails (block parameters out of range, a refused SET_NTB_INPUT_SIZE, an alignment that does not
/// fit), one line, and the same device is set up as ECM or RNDIS, without a replug. QEMU's usb-net
/// offers RNDIS and ECM (with the CI's test patch and ncm=on: NCM and ECM); the test rig asks for
/// RNDIS first through fw_cfg `opt/afrisovera/usbnet-rndis`. NCM and ECM: the MAC is the string the Ethernet functional descriptor names;
/// NCM then reads and sets its transfer block sizes (cdcncm.rs); SET_INTERFACE selects the data
/// endpoints; SET_ETHERNET_PACKET_FILTER opens the filter. RNDIS: INITIALIZE, a query of the
/// permanent MAC and a SET of the packet filter go as encapsulated control messages (RNDIS 1.0).
/// Then both endpoints are configured and receive buffers queued.
unsafe fn net_setup(d: usize, port: u32, speed: u32, ncfg: u8, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    let want_rndis = crate::qemu::fw_cfg_file(b"opt/afrisovera/usbnet-rndis");
    let order = if want_rndis { [NetKind::Rndis, NetKind::Ncm, NetKind::Ecm] } else { [NetKind::Ncm, NetKind::Ecm, NetKind::Rndis] };
    for kind in order {
        let rndis = kind == NetKind::Rndis;
        let ncm = kind == NetKind::Ncm;
        for ci in 0..ncfg.clamp(1, 4) {
            if !control(d, 0x80, 6, 0x0200 | ci as u16, 0, 9) {
                continue;
            }
            let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
            let cval = DBUF.0[5];
            if !control(d, 0x80, 6, 0x0200 | ci as u16, 0, total as u16) {
                continue;
            }
            let (mut i, mut comm, mut mac_idx, mut ncm_caps, mut union) = (0usize, None, 0u8, 0u8, None);
            let (mut cur_if, mut cur_alt, mut cur_class) = (0u8, 0u8, (0u8, 0u8, 0u8));
            let (mut data_if, mut data_alt, mut ein, mut eout) = (0u8, 0u8, None, None);
            while i + 2 <= total {
                let len = DBUF.0[i] as usize;
                if len < 2 {
                    break;
                }
                match DBUF.0[i + 1] {
                    4 if i + 9 <= total => {
                        cur_if = DBUF.0[i + 2];
                        cur_alt = DBUF.0[i + 3];
                        cur_class = (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]);
                        // RNDIS: CDC ACM with vendor protocol 0xFF (Windows-class dongles), wireless
                        // controller 0xE0/1/3 (Android), or miscellaneous 0xEF/4/1
                        let is_comm = match kind {
                            NetKind::Rndis => matches!(cur_class, (2, 2, 0xFF) | (0xE0, 1, 3) | (0xEF, 4, 1)),
                            NetKind::Ecm => (cur_class.0, cur_class.1) == (2, 6),
                            NetKind::Ncm => (cur_class.0, cur_class.1) == (2, 0x0D),
                        };
                        if is_comm && comm.is_none() {
                            comm = Some(cur_if);
                        }
                    }
                    // CS_INTERFACE descriptors of the communication interface taken
                    0x24 if i + 4 <= total && comm == Some(cur_if) && cur_alt == 0 => match DBUF.0[i + 2] {
                        0x0F => mac_idx = DBUF.0[i + 3], // Ethernet Networking: iMACAddress
                        0x06 if i + 5 <= total && union.is_none() => union = Some(DBUF.0[i + 4]), // Union: its data interface
                        0x1A if i + 6 <= total => ncm_caps = DBUF.0[i + 5], // NCM: bmNetworkCapabilities
                        _ => {}
                    },
                    5 if i + 7 <= total
                        && cur_class.0 == 0x0A
                        && comm.is_some()
                        && union.map_or(true, |u| u == cur_if)
                        && (if rndis { cur_alt == 0 } else { cur_alt >= 1 }) =>
                    {
                        let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
                        let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                        // both bulk endpoints from one interface setting: the first one that has any
                        if attr & 3 == 2 && ((ein.is_none() && eout.is_none()) || (cur_if, cur_alt) == (data_if, data_alt)) {
                            data_if = cur_if;
                            data_alt = cur_alt;
                            if addr & 0x80 != 0 {
                                ein.get_or_insert((addr & 0xF, mps));
                            } else {
                                eout.get_or_insert((addr & 0xF, mps));
                            }
                        }
                    }
                    _ => {}
                }
                i += len;
            }
            let (Some(comm_if), Some((epi, mpsi)), Some((epo, mpso))) = (comm, ein, eout) else { continue };
            let mut mac = [0u8; 6];
            let mut ncm_p = None;
            if rndis {
                if !control(d, 0x00, 9, cval as u16, 0, 0) {
                    return false; // SET_CONFIGURATION
                }
                match rndis_init(d, comm_if) {
                    Some(m) => mac = m,
                    None => return false,
                }
            } else {
                if mac_idx == 0 || !control(d, 0x80, 6, 0x0300 | mac_idx as u16, 0x0409, 64) {
                    continue;
                }
                // the MAC is a string of 12 hex digits (UTF-16LE after the 2-byte header)
                let hex = |k: usize| -> Option<u8> { (DBUF.0[2 + 2 * k] as char).to_digit(16).map(|v| v as u8) };
                let mut ok = DBUF.0[0] as usize >= 2 + 24;
                for b in 0..6 {
                    match (hex(2 * b), hex(2 * b + 1)) {
                        (Some(h), Some(l)) if ok => mac[b] = h << 4 | l,
                        _ => ok = false,
                    }
                }
                if !ok {
                    continue;
                }
                if !control(d, 0x00, 9, cval as u16, 0, 0) {
                    return false; // SET_CONFIGURATION
                }
                if ncm {
                    match ncm_blocks(d, comm_if, ncm_caps) {
                        Ok(p) => ncm_p = Some(p),
                        Err(why) => {
                            crate::kprintln!("[xhci] port {}: CDC-NCM not used: {}; trying the device as CDC-ECM or RNDIS", port, why);
                            continue;
                        }
                    }
                }
                control(d, 0x01, 11, data_alt as u16, data_if as u16, 0); // SET_INTERFACE: the data endpoints
                control(d, 0x21, 0x43, 0x0E, comm_if as u16, 0); // SET_ETHERNET_PACKET_FILTER: directed, broadcast, all multicast
            }
            let (din, dout) = (2 * epi as u32 + 1, 2 * epo as u32);
            *addr_of_mut!(PEPOUT) = Prod { idx: 0, cycle: 1 };
            core::ptr::write_bytes(addr_of_mut!(EPOUT) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
            let inb = addr_of_mut!(INCTX.0) as *mut u8;
            core::ptr::write_bytes(inb, 0, 4096);
            *ctx(inb, 0).add(1) = 1 | 1 << din | 1 << dout;
            let sl = ctx(inb, 1);
            *sl = speed << 20 | din.max(dout) << 27 | PATH.0;
            *sl.add(1) = port << 16;
            *sl.add(2) = PATH.1 | PATH.2 << 8;
            for (dci, tr, ty, mps) in [(din, addr_of!(EPIN[d]) as u64, 6u32, mpsi), (dout, addr_of!(EPOUT) as u64, 2u32, mpso)] {
                let ec = ctx(inb, 1 + dci as usize);
                *ec.add(1) = 3 << 1 | ty << 3 | mps << 16; // CErr 3, bulk IN (6) / OUT (2)
                *ec.add(2) = tr as u32 | 1;
                *ec.add(3) = (tr >> 32) as u32;
                *ec.add(4) = 1024; // average TRB length
            }
            if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
                return false;
            }
            DEVS[d].dci = din;
            DEVS[d].kind = KIND_NET;
            NDEVS = NDEVS.max(d + 1);
            NET_D = d;
            NET_SLOT = slot;
            NET_PORT = port;
            NET_DIN = din;
            NET_DOUT = dout;
            NET_MAC = mac;
            NET_RNDIS = rndis;
            NET_NCM = ncm_p;
            NET_MPSO = mpso.max(8);
            NCM_STATS = NcmStats::ZERO;
            for b in 0..NRXB {
                queue_rx(b);
            }
            // RNDIS passes data only once the filter is set: after the receive buffers are queued
            let filtered = !rndis || rndis_filter(d, comm_if);
            READY = true;
            crate::kprintln!(
                "[xhci] port {}: USB Ethernet adapter {:04x}:{:04x} ({}, {} speed), slot {}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                port, vid, pid, kind.name(),
                speed_name(speed), slot,
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
            if let Some((p, insz)) = ncm_p {
                crate::kprintln!(
                    "[xhci] NCM: blocks up to {} KiB in (the device offers {} KiB), {} KiB out, table alignment {}, frame divisor {} remainder {}, 16-bit blocks",
                    insz / 1024, p.in_max / 1024, p.out_max / 1024, p.out_align, p.out_div, p.out_rem
                );
            }
            if rndis {
                let (per, max) = RNDIS_INFO;
                crate::kprintln!(
                    "[xhci] RNDIS: initialized (the device sends up to {} packet(s), {} bytes per transfer), packet filter {}",
                    per, max, if filtered { "set" } else { "REFUSED: nothing will be received" }
                );
            }
            return true;
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NetKind {
    Ncm,
    Ecm,
    Rndis,
}

impl NetKind {
    fn name(self) -> &'static str {
        match self {
            NetKind::Ncm => "CDC-NCM",
            NetKind::Ecm => "CDC-ECM",
            NetKind::Rndis => "RNDIS",
        }
    }
}

/// NCM's transfer blocks, while the data interface is still in setting 0 (NCM 1.0 6.2): read the
/// device's NTB parameters and check them (cdcncm::params), ask for 16-bit blocks when it also
/// offers 32-bit ones, and set the largest block it may send us to our receive buffer. Any refusal
/// is the reason the caller prints before it tries the device as ECM or RNDIS.
unsafe fn ncm_blocks(d: usize, comm: u8, caps: u8) -> Result<(crate::cdcncm::Params, usize), &'static str> {
    if !control(d, 0xA1, 0x80, 0, comm as u16, 28) {
        return Err("GET_NTB_PARAMETERS was refused");
    }
    let p = crate::cdcncm::params(&DBUF.0[..28], FRAME)?;
    if p.formats & 2 != 0 && !control(d, 0x21, 0x84, 0, comm as u16, 0) {
        return Err("SET_NTB_FORMAT (16-bit) was refused");
    }
    let insz = p.in_max.min(NCM_IN);
    DBUF.0[..8].fill(0);
    DBUF.0[..4].copy_from_slice(&(insz as u32).to_le_bytes());
    // bmNetworkCapabilities bit 5: the request carries 8 bytes (size, then most datagrams, 0 = any)
    let n = if caps & 1 << 5 != 0 { 8 } else { 4 };
    if !control(d, 0x21, 0x86, 0, comm as u16, n) {
        return Err("SET_NTB_INPUT_SIZE was refused");
    }
    Ok((p, insz))
}

/// What the RNDIS device said in INITIALIZE_CMPLT: packets and bytes per transfer at most.
static mut RNDIS_INFO: (u32, u32) = (0, 0);

/// RNDIS messages are built in, and read back from, DBUF: little-endian 32-bit fields.
unsafe fn dput(o: usize, v: u32) {
    (&mut (*addr_of_mut!(DBUF)).0)[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
unsafe fn dget(o: usize) -> u32 {
    let b = &(*addr_of!(DBUF)).0;
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Send the RNDIS control message built in DBUF (`len` bytes) with SEND_ENCAPSULATED_COMMAND
/// and read its completion with GET_ENCAPSULATED_RESPONSE, polled (the RESPONSE_AVAILABLE
/// notification on the interrupt endpoint is not waited for), skipping anything else until the
/// reply with this request id comes. True when it came with status SUCCESS; it is then in DBUF.
unsafe fn rndis_cmd(d: usize, comm: u8, len: u16) -> bool {
    let (ty, id) = (dget(0), dget(8));
    if !control(d, 0x21, 0x00, 0, comm as u16, len) {
        return false;
    }
    for _ in 0..40 {
        dput(0, 0);
        dput(8, 0);
        if control(d, 0xA1, 0x01, 0, comm as u16, 512) && dget(0) == ty | 0x8000_0000 && dget(8) == id {
            let st = dget(12);
            if st != 0 {
                crate::kprintln!("[xhci] RNDIS: message {} refused (status {:#010x})", ty, st);
            }
            return st == 0;
        }
        crate::timer::busywait_ms(5);
    }
    crate::kprintln!("[xhci] RNDIS: no reply to message {}", ty);
    false
}

/// RNDIS INITIALIZE (version 1.0; the device may send us at most FRAME bytes per transfer),
/// then the adapter's permanent MAC (OID_802_3_PERMANENT_ADDRESS). The medium must be 802.3.
unsafe fn rndis_init(d: usize, comm: u8) -> Option<[u8; 6]> {
    (&mut (*addr_of_mut!(DBUF)).0)[..64].fill(0);
    for (o, v) in [(0, 2u32), (4, 24), (8, 1), (12, 1), (16, 0), (20, FRAME as u32)] {
        dput(o, v);
    }
    if !rndis_cmd(d, comm, 24) {
        return None;
    }
    if dget(28) != 0 {
        crate::kprintln!("[xhci] RNDIS: medium {} is not Ethernet (802.3)", dget(28));
        return None;
    }
    RNDIS_INFO = (dget(32), dget(36));
    (&mut (*addr_of_mut!(DBUF)).0)[..64].fill(0);
    for (o, v) in [(0, 4u32), (4, 28), (8, 2), (12, 0x0101_0101), (16, 0), (20, 0), (24, 0)] {
        dput(o, v);
    }
    if !rndis_cmd(d, comm, 28) {
        return None;
    }
    let (n, off) = (dget(16) as usize, dget(20) as usize);
    if n < 6 || 8 + off + 6 > 512 {
        crate::kprintln!("[xhci] RNDIS: no MAC address in the reply");
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&(&(*addr_of!(DBUF)).0)[8 + off..8 + off + 6]);
    Some(mac)
}

/// RNDIS SET of OID_GEN_CURRENT_PACKET_FILTER: directed, all multicast, broadcast.
unsafe fn rndis_filter(d: usize, comm: u8) -> bool {
    (&mut (*addr_of_mut!(DBUF)).0)[..64].fill(0);
    for (o, v) in [(0, 5u32), (4, 32), (8, 3), (12, 0x0001_010E), (16, 4), (20, 20), (24, 0), (28, 0x0D)] {
        dput(o, v);
    }
    rndis_cmd(d, comm, 32)
}

/// Queue receive buffer `b` on the adapter's bulk-IN ring.
unsafe fn queue_rx(b: usize) {
    let d = NET_D;
    let (buf, len) = match NET_NCM {
        Some((_, insz)) => (ncm_rx(b) as u64, insz as u32),
        None => (addr_of!(NETRX[b]) as u64, FRAME as u32),
    };
    let at = push_held(
        addr_of_mut!(EPIN[d].0) as *mut Trb,
        NXFER,
        &mut (*addr_of_mut!(PEPIN))[d],
        buf,
        len,
        T_NORMAL << 10 | 1 << 5 | 1 << 2, // IOC, ISP: one event per frame, short ones too
        false,
    );
    RXMAP[b] = at as u64;
    doorbell(NET_SLOT, NET_DIN);
}

/// A transfer event for the adapter: a received frame (into the queue, buffer re-queued) or
/// a finished send.
unsafe fn net_event(ev: &Trb, cc: u32) {
    let ep = (ev.control >> 16) & 0x1F;
    if ep == NET_DOUT {
        NET_TXDONE += 1;
        return;
    }
    if ep != NET_DIN {
        return;
    }
    let Some(b) = (0..NRXB).find(|&b| RXMAP[b] == ev.param) else { return };
    if let Some((_, insz)) = NET_NCM {
        // architect's condition 4: this buffer's transfer has completed, so it is ours again. After an error
        // completion (a stall, babble, a transaction error) the endpoint is halted: the buffer is not handed
        // back to it; receiving stops with one line (a replug sets the adapter up again with all buffers).
        if cc != CC_SUCCESS && cc != CC_SHORT {
            RXMAP[b] = 0;
            NCM_STATS.stopped += 1;
            if NCM_STATS.stopped == 1 {
                crate::kprintln!("[xhci] NCM: a receive ended with completion code {}: that buffer is not reused (replug to receive again)", cc);
            }
            return;
        }
        let n = insz.saturating_sub((ev.status & 0xFF_FFFF) as usize);
        let blk = core::slice::from_raw_parts(ncm_rx(b) as *const u8, n.min(NCM_IN));
        let g = crate::cdcncm::parse(blk, |f| rxq_put(f.as_ptr(), f.len()));
        let st = &mut *addr_of_mut!(NCM_STATS);
        st.blocks += 1;
        st.most = st.most.max(g.frames + g.dropped);
        st.skipped += g.skipped as u64;
        st.dropped += g.dropped as u64;
        if g.bad.is_some() {
            st.bad += 1;
            st.last_bad = g.bad;
        }
        queue_rx(b);
        return;
    }
    if cc == CC_SUCCESS || cc == CC_SHORT {
        let n = FRAME.saturating_sub((ev.status & 0xFF_FFFF) as usize);
        let buf = addr_of!(NETRX[b]) as *const u8;
        if !NET_RNDIS {
            rxq_put(buf, n);
        } else {
            // one transfer may carry several RNDIS messages, each saying its own length
            let mut off = 0usize;
            while off + RNDIS_HDR <= n {
                let g = |k: usize| u32::from_le_bytes(core::ptr::read_unaligned(buf.add(off + k) as *const [u8; 4])) as usize;
                let (ty, ml, doff, dl) = (g(0), g(4), g(8), g(12));
                if ml < RNDIS_HDR || off + ml > n {
                    NET_RXDROP += 1;
                    break;
                }
                if ty == 1 {
                    // PACKET_MSG: the frame at DataOffset (counted from byte 8), DataLength long
                    if doff >= 36 && 8 + doff + dl <= ml {
                        rxq_put(buf.add(off + 8 + doff), dl);
                    } else {
                        NET_RXDROP += 1;
                    }
                }
                off += ml;
            }
        }
    }
    queue_rx(b);
}

/// One received Ethernet frame into the queue for the network stack (dropped if full); true if taken.
unsafe fn rxq_put(src: *const u8, n: usize) -> bool {
    let next = (RXQ_TAIL + 1) % NRXQ;
    if (14..=FRAME).contains(&n) && next != RXQ_HEAD {
        core::ptr::copy_nonoverlapping(src, addr_of_mut!(RXQ[RXQ_TAIL]) as *mut u8, n);
        RXQLEN[RXQ_TAIL] = n;
        core::sync::atomic::fence(Ordering::SeqCst);
        RXQ_TAIL = next;
        NET_RXN += 1;
        true
    } else {
        NET_RXDROP += 1;
        false
    }
}

/// True when a USB Ethernet adapter is configured.
pub fn net_present() -> bool {
    // SAFETY: scalar read.
    unsafe { NET_SLOT != 0 }
}

pub fn net_mac() -> [u8; 6] {
    // SAFETY: scalar read of boot-time state.
    unsafe { NET_MAC }
}

/// True when the adapter speaks CDC-NCM (newer phones' USB tethering, 2.5G dongles).
pub fn net_ncm() -> bool {
    // SAFETY: scalar read of enumeration-time state.
    unsafe { (*addr_of!(NET_NCM)).is_some() }
}

/// `usbnettest`: the NCM counters (blocks, frames per block, bad blocks), or `usbnettest ncm`: the block
/// parser's selftest (cdcncm.rs).
pub fn ncm_command(arg: &str) {
    if arg.trim() == "ncm" {
        crate::cdcncm::selftest();
        return;
    }
    // SAFETY: scalar reads of counters the timer interrupt updates; a torn value is only a stale count.
    let (on, st) = unsafe { ((*addr_of!(NET_NCM)).is_some(), *addr_of!(NCM_STATS)) };
    if !on {
        crate::kprintln!("usbnettest: no CDC-NCM adapter (`usbnettest ncm` runs the block parser's selftest)");
        return;
    }
    crate::kprintln!(
        "usbnettest: NCM {} block(s) in, most frames in one block {}, {} block(s) out, bad blocks {} (last: {}), skipped entries {}, frames with no room {}, receive buffers stopped {}",
        st.blocks,
        st.most,
        st.sent,
        st.bad,
        st.last_bad.map_or("none", |b| b.name()),
        st.skipped,
        st.dropped,
        st.stopped
    );
}

/// True when the adapter speaks RNDIS (a phone's USB tethering) rather than CDC-ECM.
pub fn net_rndis() -> bool {
    // SAFETY: scalar read of enumeration-time state.
    unsafe { NET_RNDIS }
}

/// Send one Ethernet frame through the adapter (CDC-ECM: the frame as it is, no header;
/// RNDIS: in a PACKET_MSG, with one pad byte inside it when the message would end exactly on
/// a packet boundary, so no zero-length packet is needed to end the transfer).
pub fn net_send(frame: &[u8]) -> bool {
    // SAFETY: the bulk-OUT ring and transmit buffers are only touched here (driver context);
    // the timer interrupt only counts their completions.
    unsafe {
        let hdr = if NET_RNDIS { RNDIS_HDR } else { 0 };
        if NET_SLOT == 0 || hdr + frame.len() + 1 > FRAME || NET_TXN - NET_TXDONE >= NTXB as u64 {
            return false;
        }
        if let Some((p, _)) = NET_NCM {
            let Some(home) = on(DEVS[NET_D].ctl) else { return false };
            let b = (NET_TXN % NTXB as u64) as usize;
            let tx = &mut (*addr_of_mut!(NETTX[b])).0;
            let Some(n) = crate::cdcncm::build(frame, NCM_SEQ, &p, NET_MPSO as usize, tx) else {
                select(home);
                return false;
            };
            NCM_SEQ = NCM_SEQ.wrapping_add(1);
            NCM_STATS.sent += 1;
            push(addr_of_mut!(EPOUT.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PEPOUT), tx.as_ptr() as u64, n as u32, T_NORMAL << 10 | 1 << 5);
            NET_TXN += 1;
            doorbell(NET_SLOT, NET_DOUT);
            select(home);
            return true;
        }
        let Some(home) = on(DEVS[NET_D].ctl) else { return false };
        let b = (NET_TXN % NTXB as u64) as usize;
        let tx = addr_of_mut!(NETTX[b]) as *mut u8;
        let mut n = frame.len();
        if NET_RNDIS {
            n += RNDIS_HDR;
            if n % NET_MPSO as usize == 0 {
                *tx.add(n) = 0;
                n += 1;
            }
            core::ptr::write_bytes(tx, 0, RNDIS_HDR);
            for (o, v) in [(0usize, 1u32), (4, n as u32), (8, 36), (12, frame.len() as u32)] {
                core::ptr::write_unaligned(tx.add(o) as *mut [u8; 4], v.to_le_bytes());
            }
        }
        core::ptr::copy_nonoverlapping(frame.as_ptr(), tx.add(hdr), frame.len());
        push(addr_of_mut!(EPOUT.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PEPOUT), tx as u64, n as u32, T_NORMAL << 10 | 1 << 5);
        NET_TXN += 1;
        doorbell(NET_SLOT, NET_DOUT);
        select(home);
        true
    }
}

/// True if a received frame is waiting.
pub fn net_rx_ready() -> bool {
    // SAFETY: scalar reads; the queue has one producer (the timer interrupt) and one consumer.
    unsafe { RXQ_HEAD != RXQ_TAIL }
}

/// Take the next received frame into `dst`.
pub fn net_recv(dst: &mut [u8]) -> Option<usize> {
    // SAFETY: as net_rx_ready; the slot at HEAD is complete once TAIL has moved past it.
    unsafe {
        if RXQ_HEAD == RXQ_TAIL {
            return None;
        }
        let n = RXQLEN[RXQ_HEAD].min(dst.len());
        core::ptr::copy_nonoverlapping(addr_of!(RXQ[RXQ_HEAD]) as *const u8, dst.as_mut_ptr(), n);
        RXQ_HEAD = (RXQ_HEAD + 1) % NRXQ;
        Some(n)
    }
}

// --- USB serial cables (FTDI FT232R/FT232H/FT2232/FT4232/FT-X): a console for PCs without COM1 ---
// No PC or laptop sold in years has a COM1 header, so on real hardware the boot log and the
// shell were on the screen only: nothing to copy from when bring-up goes wrong. A cheap
// USB-to-serial cable (most use an FTDI chip) plugged into the PC and another computer gives
// both: every console line is sent to it (as COM1's are) and what is typed on the far end
// reaches the shell (as a USB keyboard's keys do). 115200 baud, 8 data bits, no parity, one
// stop bit, as COM1. FTDI's bulk IN packets each start with two modem/line status bytes; its
// bulk OUT data is raw. (Linux drivers/usb/serial/ftdi_sio.c; FTDI AN232B-04/05.)
const SER_Q: usize = 16384;
static mut SER_RING: Ring<NXFER> = Ring([TRB0; NXFER]); // bulk OUT
static mut PSER: Prod = Prod { idx: 0, cycle: 1 };
static mut SER_TXBUF: Block = Block([0; 4096]);
static mut SER_QBUF: [u8; SER_Q] = [0; SER_Q];
// console text waiting to go out: written under the print lock, taken by the timer poll
static SER_QH: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static SER_QT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
static mut SER_SLOT: u32 = 0;
static mut SER_PORT: u32 = 0;
static mut SER_CTL: usize = 0;
static mut SER_D: usize = 0;
static mut SER_DIN: u32 = 0;
static mut SER_DOUT: u32 = 0;
static mut SER_MPSI: u32 = 64;
static mut SER_INFLIGHT: bool = false;
static mut SER_TX: u64 = 0;
static mut SER_RX: u64 = 0;
static mut SER_DROP: u64 = 0;
static mut SER_ERR: u64 = 0;

/// The chip's name, if `vid:pid` is an FTDI USB-serial chip.
fn ftdi_name(vid: u16, pid: u16) -> Option<&'static str> {
    if vid != 0x0403 {
        return None;
    }
    Some(match pid {
        0x6001 => "FT232R/FT232BM",
        0x6010 => "FT2232",
        0x6011 => "FT4232",
        0x6014 => "FT232H",
        0x6015 => "FT-X",
        _ => return None,
    })
}

/// An FTDI cable: its first interface's bulk endpoints configured, the line set to 115200 8N1
/// with DTR and RTS on and no flow control, a read queued, and the console mirrored to it.
unsafe fn serial_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16) -> bool {
    let Some(chip) = ftdi_name(vid, pid) else { return false };
    let slot = DEVS[d].slot;
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let cval = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    // the first interface (a multi-port chip's port A) and its bulk IN / OUT endpoints
    let (mut i, mut nif, mut ifn, mut ein, mut eout) = (0usize, 0u32, 0u8, None, None);
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 {
            break;
        }
        match DBUF.0[i + 1] {
            4 if i + 9 <= total => {
                nif += 1;
                if nif == 1 {
                    ifn = DBUF.0[i + 2];
                }
            }
            5 if nif == 1 && i + 7 <= total && DBUF.0[i + 3] & 3 == 2 => {
                let (addr, mps) = (DBUF.0[i + 2], u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF);
                if addr & 0x80 != 0 {
                    ein = ein.or(Some((addr & 0xF, mps)));
                } else {
                    eout = eout.or(Some((addr & 0xF, mps)));
                }
            }
            _ => {}
        }
        i += len;
    }
    let (Some((epi, mpsi)), Some((epo, mpso))) = (ein, eout) else {
        crate::kprintln!("[xhci] port {}: FTDI {} without bulk endpoints, not used", port, chip);
        return false;
    };
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    // vendor requests; a multi-port chip numbers its ports from 1, a single-port one takes 0
    let idx = if matches!(pid, 0x6010 | 0x6011) { ifn as u16 + 1 } else { 0 };
    let line = control(d, 0x40, 0, 0, idx, 0) // SIO_RESET
        && control(d, 0x40, 1, 0x0303, idx, 0) // SIO_SET_MODEM_CTRL: DTR and RTS on
        && control(d, 0x40, 2, 0, idx, 0) // SIO_SET_FLOW_CTRL: none
        && control(d, 0x40, 3, 26, idx, 0) // SIO_SET_BAUD_RATE: 3 MHz / 26 = 115200
        && control(d, 0x40, 4, 0x0008, idx, 0); // SIO_SET_DATA: 8 bits, no parity, 1 stop bit
    if !line {
        return false;
    }
    let (din, dout) = (2 * epi as u32 + 1, 2 * epo as u32);
    *addr_of_mut!(PSER) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(SER_RING) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << din | 1 << dout;
    let sl = ctx(inb, 1);
    *sl = speed << 20 | din.max(dout) << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    for (dci, tr, ty, mps) in [(din, addr_of!(EPIN[d]) as u64, 6u32, mpsi), (dout, addr_of!(SER_RING) as u64, 2u32, mpso)] {
        let ec = ctx(inb, 1 + dci as usize);
        *ec.add(1) = 3 << 1 | ty << 3 | mps << 16; // CErr 3, bulk IN (6) / OUT (2)
        *ec.add(2) = tr as u32 | 1;
        *ec.add(3) = (tr >> 32) as u32;
        *ec.add(4) = 512; // average TRB length
    }
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].dci = din;
    DEVS[d].kind = KIND_SERIAL;
    NDEVS = NDEVS.max(d + 1);
    (SER_D, SER_PORT, SER_CTL, SER_DIN, SER_DOUT) = (d, port, CUR, din, dout);
    SER_MPSI = mpsi.clamp(8, 512);
    (SER_TX, SER_RX, SER_DROP, SER_ERR, SER_INFLIGHT) = (0, 0, 0, 0, false);
    SER_QH.store(SER_QT.load(Ordering::Acquire), Ordering::Release); // nothing older than the cable
    ser_queue_rx();
    SER_SLOT = slot; // from here on every console line is copied to it (this one too)
    READY = true;
    crate::kprintln!(
        "[xhci] port {}: USB serial cable {:04x}:{:04x} (FTDI {}, {} speed), slot {}, 115200 baud 8N1, bulk IN {} / OUT {}: the console is mirrored to it",
        port, vid, pid, chip, speed_name(speed), slot, epi, epo
    );
    true
}

/// Queue one read (one packet) on the cable's bulk-IN ring.
unsafe fn ser_queue_rx() {
    let d = SER_D;
    push(
        addr_of_mut!(EPIN[d].0) as *mut Trb,
        NXFER,
        &mut (*addr_of_mut!(PEPIN))[d],
        addr_of!(REPORT[d]) as u64,
        SER_MPSI,
        T_NORMAL << 10 | 1 << 5 | 1 << 2, // IOC, ISP
    );
    doorbell(DEVS[d].slot, SER_DIN);
}

/// A transfer event for the cable: bytes typed on the far end (status bytes dropped, the rest
/// to the console input), or a finished send (the next one started).
unsafe fn ser_event(d: usize, ev: &Trb, cc: u32) {
    let ep = (ev.control >> 16) & 0x1F;
    let ok = cc == CC_SUCCESS || cc == CC_SHORT;
    if ep == SER_DOUT {
        SER_INFLIGHT = false;
        if ok {
            ser_kick();
        } else {
            SER_ERR += 1; // a halted endpoint: sending stops (the input side keeps working)
        }
        return;
    }
    if ep != SER_DIN {
        return;
    }
    if ok {
        let got = (SER_MPSI as usize).saturating_sub((ev.status & 0xFF_FFFF) as usize).min(512);
        let r = &(*addr_of!(REPORT[d])).0;
        for pkt in r[..got].chunks(SER_MPSI as usize) {
            for &b in pkt.iter().skip(2) {
                crate::uart::inject(b);
                SER_RX += 1;
            }
        }
        ser_queue_rx();
    } else {
        SER_ERR += 1;
    }
}

/// Start sending what the console printed, if no send is under way (its controller current).
unsafe fn ser_kick() {
    if SER_INFLIGHT || SER_SLOT == 0 || SER_ERR > 16 {
        return;
    }
    let (h, t) = (SER_QH.load(Ordering::Acquire), SER_QT.load(Ordering::Acquire));
    if h == t {
        return;
    }
    let n = ((t + SER_Q - h) % SER_Q).min(4096);
    let q = &*addr_of!(SER_QBUF);
    let buf = &mut (*addr_of_mut!(SER_TXBUF)).0;
    for k in 0..n {
        buf[k] = q[(h + k) % SER_Q];
    }
    SER_QH.store((h + n) % SER_Q, Ordering::Release);
    push(addr_of_mut!(SER_RING.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PSER), buf.as_ptr() as u64, n as u32, T_NORMAL << 10 | 1 << 5);
    SER_INFLIGHT = true;
    SER_TX += n as u64;
    doorbell(SER_SLOT, SER_DOUT);
}

/// Console text for the serial cable (from uart.rs, under its print lock): queued, "\n" as
/// "\r\n" as on COM1; sent by the timer poll. Dropped (and counted) when the queue is full.
pub fn serial_write(s: &str) {
    // SAFETY: SER_SLOT is a scalar read; the queue has one producer (under the print lock) and
    // one consumer (the poll), each moving only its own index.
    unsafe {
        if SER_SLOT == 0 {
            return;
        }
        let q = &mut *addr_of_mut!(SER_QBUF);
        let mut t = SER_QT.load(Ordering::Relaxed);
        for b in s.bytes() {
            for c in [b'\r', b] {
                if c == b'\r' && b != b'\n' {
                    continue;
                }
                let next = (t + 1) % SER_Q;
                if next == SER_QH.load(Ordering::Acquire) {
                    SER_DROP += 1;
                } else {
                    q[t] = c;
                    t = next;
                }
            }
        }
        SER_QT.store(t, Ordering::Release);
    }
}

/// True when a USB serial cable carries the console.
pub fn serial_present() -> bool {
    // SAFETY: scalar read.
    unsafe { SER_SLOT != 0 }
}

// --- USB audio: headsets, USB speakers, a monitor's or dock's sound output (Audio Class 1.0) ---
// The PC's sound used to be its HD Audio chip only; a USB headset (cheap, common, and the only
// sound output on many thin clients and desk setups) was "audio, not driven". One playback
// interface is driven: 16-bit PCM, one or two channels, one isochronous OUT endpoint. The
// shared sound calls (`audio tone`, `say`, Nia's `speak()`) go to it when it is there.
const NAUD: usize = 64; // isochronous TRB ring: 63 packets + the link
const AUD_AHEAD: u64 = 40; // packets queued ahead of the device (40 ms at full speed)
const AUD_PKT: usize = 1024; // the largest packet we build
const SRC_HZ: u32 = 44_100; // the shared sound calls' sample rate (mono)
#[repr(C, align(1024))]
struct Pkt([u8; AUD_PKT]);
static mut AUD_RING: Ring<NAUD> = Ring([TRB0; NAUD]);
static mut PAUD: Prod = Prod { idx: 0, cycle: 1 };
static mut AUDBUF: [Pkt; NAUD] = [const { Pkt([0; AUD_PKT]) }; NAUD];
static mut AUD_D: usize = usize::MAX;
static mut AUD_SLOT: u32 = 0;
static mut AUD_PORT: u32 = 0;
static mut AUD_DCI: u32 = 0;
static mut AUD_RATE: u32 = 0;
static mut AUD_CH: u32 = 2;
static mut AUD_US: u64 = 1000; // one packet's time (the endpoint's service interval)
static mut AUD_MPS: u32 = 0;
static mut AUD_ACC: u64 = 0; // rate x time not yet turned into frames
static mut AUD_RS: u32 = 0; // the resampler's position between input samples
static mut AUD_FILL: usize = 0; // bytes in the packet being built
static mut AUD_NEED: usize = 0; // ... and its size
static mut AUD_SENT: u64 = 0; // packets queued since the device came
static mut AUD_DONE: u64 = 0; // ... completed (played or missed)
static mut AUD_MISSED: u64 = 0;
static mut AUD_STREAMS: u64 = 0;
static mut AUD_SAMPLES: u64 = 0; // input samples of the stream now playing
static mut AUD_FRAMES: u64 = 0; // ... and the frames they became
static mut AUD_PKTS: u64 = 0; // ... in this many packets

/// One alternate setting of an AudioStreaming interface, as read from the descriptors.
#[derive(Clone, Copy)]
struct AsAlt {
    iface: u8,
    alt: u8,
    pcm: bool,
    ch: u8,
    sub: u8,
    bits: u8,
    rate: u32,
    ep: Option<(u8, u32, u8)>, // isochronous OUT: address, packet size, bInterval
    sfc: bool,                 // the endpoint takes a sampling-frequency control
}

impl AsAlt {
    fn score(&self) -> u32 {
        if !self.pcm || self.sub != 2 || self.bits != 16 || !(1..=2).contains(&self.ch) || self.rate == 0 || self.ep.is_none() {
            return 0;
        }
        1 + 4 * (self.ch == 2) as u32 + 2 * (self.rate == SRC_HZ) as u32 + (self.rate == 48_000) as u32
    }
}

/// The rate a format type I descriptor `f` offers that suits us best: 44.1 kHz (no
/// resampling), else 48 kHz, else the first listed between 8 and 96 kHz; 0 when none.
fn pick_rate(f: &[u8]) -> u32 {
    let n = f[7] as usize;
    let at = |k: usize| -> u32 {
        if 8 + 3 * k + 3 > f.len() { 0 } else { u32::from_le_bytes([f[8 + 3 * k], f[9 + 3 * k], f[10 + 3 * k], 0]) }
    };
    if n == 0 {
        // continuous: lower and upper bound
        let (lo, hi) = (at(0), at(1));
        return [SRC_HZ, 48_000].into_iter().find(|r| (lo..=hi).contains(r)).unwrap_or(0);
    }
    if (0..n).any(|k| at(k) == SRC_HZ) {
        SRC_HZ
    } else if (0..n).any(|k| at(k) == 48_000) {
        48_000
    } else {
        (0..n).map(at).find(|r| (8_000..=96_000).contains(r)).unwrap_or(0)
    }
}

/// Keep `c` in `best` when it scores higher.
fn keep_alt(c: Option<AsAlt>, best: &mut Option<AsAlt>) {
    if let Some(a) = c {
        if a.score() > best.map_or(0, |b| b.score()) {
            *best = Some(a);
        }
    }
}

/// A USB Audio Class 1.0 device with a playback interface: the best alternate setting
/// chosen (stereo before mono, 44.1 kHz before 48 kHz), the device configured, the
/// interface switched to it, the rate set when the endpoint takes one, the feature unit
/// unmuted, and the isochronous OUT endpoint configured. Audio Class 2.0 devices (many
/// USB DACs) are named and left alone.
unsafe fn audio_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let cval = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    let b = (*addr_of!(DBUF)).0;
    let (mut i, mut cls, mut ac_if, mut fu, mut uac2) = (0usize, (0u8, 0u8, 0u8), None, 0u8, false);
    let mut cur: Option<AsAlt> = None;
    let mut best: Option<AsAlt> = None;
    while i + 2 <= total {
        let len = b[i] as usize;
        if len < 2 || i + len > total {
            break;
        }
        match b[i + 1] {
            4 if len >= 9 => {
                keep_alt(cur.take(), &mut best);
                cls = (b[i + 5], b[i + 6], b[i + 7]);
                if cls.0 == 1 && cls.2 == 0x20 {
                    uac2 = true;
                }
                if (cls.0, cls.1) == (1, 1) && ac_if.is_none() {
                    ac_if = Some(b[i + 2]);
                }
                if (cls.0, cls.1) == (1, 2) && b[i + 3] > 0 {
                    cur = Some(AsAlt { iface: b[i + 2], alt: b[i + 3], pcm: false, ch: 0, sub: 0, bits: 0, rate: 0, ep: None, sfc: false });
                }
            }
            // class-specific AudioControl: the first feature unit (volume / mute)
            0x24 if (cls.0, cls.1) == (1, 1) && len >= 4 && b[i + 2] == 6 && fu == 0 => fu = b[i + 3],
            // class-specific AudioStreaming: AS_GENERAL (format tag 1 = PCM), FORMAT_TYPE I
            0x24 if cur.is_some() && len >= 7 && b[i + 2] == 1 => {
                if let Some(a) = cur.as_mut() {
                    a.pcm = u16::from_le_bytes([b[i + 5], b[i + 6]]) == 1;
                }
            }
            0x24 if cur.is_some() && len >= 8 && b[i + 2] == 2 && b[i + 3] == 1 => {
                if let Some(a) = cur.as_mut() {
                    (a.ch, a.sub, a.bits) = (b[i + 4], b[i + 5], b[i + 6]);
                    a.rate = pick_rate(&b[i..i + len]);
                }
            }
            5 if cur.is_some() && len >= 7 && b[i + 3] & 3 == 1 && b[i + 2] & 0x80 == 0 => {
                if let Some(a) = cur.as_mut() {
                    let mps = u16::from_le_bytes([b[i + 4], b[i + 5]]) as u32 & 0x7FF;
                    a.ep.get_or_insert((b[i + 2] & 0xF, mps, b[i + 6]));
                }
            }
            // class-specific endpoint: bit 0 = it takes a sampling-frequency control
            0x25 if cur.is_some() && len >= 4 && b[i + 2] == 1 => {
                if let Some(a) = cur.as_mut() {
                    a.sfc = b[i + 3] & 1 != 0;
                }
            }
            _ => {}
        }
        i += len;
    }
    keep_alt(cur.take(), &mut best);
    let Some(a) = best else {
        if uac2 {
            crate::kprintln!("[xhci] port {}: USB audio {:04x}:{:04x} is Audio Class 2.0, not driven yet", port, vid, pid);
        }
        return false;
    };
    let (epn, mps, binterval) = a.ep.unwrap_or((0, 0, 1));
    // one packet per service interval: full speed counts bInterval in 1 ms frames, high
    // speed in 125 us microframes (both 2^(bInterval-1)); the xHCI field counts microframes
    let interval = if speed >= 3 { (binterval as u32).clamp(1, 16) - 1 } else { (binterval as u32).clamp(1, 16) + 2 };
    let us = 125u64 << interval;
    let bytes_max = ((a.rate as u64 * us).div_ceil(1_000_000) * a.ch as u64 * 2) as u32;
    if bytes_max > mps || bytes_max as usize > AUD_PKT {
        crate::kprintln!("[xhci] port {}: USB audio {:04x}:{:04x}: {} bytes per packet do not fit its {}", port, vid, pid, bytes_max, mps);
        return false;
    }
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    if !control(d, 0x01, 11, a.alt as u16, a.iface as u16, 0) {
        return false; // SET_INTERFACE: the playback setting
    }
    if a.sfc {
        // SET_CUR SAMPLING_FREQ_CONTROL on the endpoint (3 bytes); a refusal leaves its default
        (&mut (*addr_of_mut!(DBUF)).0)[..3].copy_from_slice(&a.rate.to_le_bytes()[..3]);
        control(d, 0x22, 0x01, 0x0100, epn as u16, 3);
    }
    let unmuted = match ac_if {
        Some(ac) if fu != 0 => {
            DBUF.0[0] = 0; // SET_CUR MUTE_CONTROL, master channel: not muted
            control(d, 0x21, 0x01, 0x0100, (fu as u16) << 8 | ac as u16, 1)
        }
        _ => false,
    };
    let dci = 2 * epn as u32; // OUT
    *addr_of_mut!(PAUD) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(AUD_RING) as *mut u8, 0, core::mem::size_of::<Ring<NAUD>>());
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << dci;
    let sl = ctx(inb, 1);
    *sl = speed << 20 | dci << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    let ec = ctx(inb, 1 + dci as usize);
    *ec = interval << 16;
    *ec.add(1) = 1 << 3 | mps << 16; // CErr 0 (isochronous), isochronous OUT (1)
    let tr = addr_of!(AUD_RING) as u64;
    *ec.add(2) = tr as u32 | 1;
    *ec.add(3) = (tr >> 32) as u32;
    *ec.add(4) = bytes_max | bytes_max << 16; // average TRB length, max ESIT payload
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].dci = dci;
    DEVS[d].kind = KIND_AUDIO;
    NDEVS = NDEVS.max(d + 1);
    (AUD_D, AUD_SLOT, AUD_PORT, AUD_DCI) = (d, slot, port, dci);
    (AUD_RATE, AUD_CH, AUD_US, AUD_MPS) = (a.rate, a.ch as u32, us, mps);
    (AUD_SENT, AUD_DONE, AUD_MISSED, AUD_STREAMS, AUD_ACC) = (0, 0, 0, 0, 0);
    READY = true;
    crate::kprintln!(
        "[xhci] port {}: USB audio {:04x}:{:04x} ({} speed), slot {}, {} channel(s) 16-bit at {} Hz, isochronous OUT endpoint {} every {} us{}",
        port, vid, pid, speed_name(speed), slot,
        a.ch, a.rate, epn, us,
        if unmuted { ", unmuted" } else { "" }
    );
    true
}

/// True when a USB headset or speaker is ready to play.
pub fn audio_present() -> bool {
    // SAFETY: scalar read.
    unsafe { AUD_SLOT != 0 }
}

/// Bytes of the next packet: the frames its service interval holds at the device's rate.
unsafe fn next_packet() -> usize {
    AUD_ACC += AUD_RATE as u64 * AUD_US;
    let frames = AUD_ACC / 1_000_000;
    AUD_ACC %= 1_000_000;
    frames as usize * AUD_CH as usize * 2
}

/// Wait until fewer than `ahead` packets are queued; false if the device stops taking them.
unsafe fn audio_wait(ahead: u64) -> bool {
    let t0 = crate::timer::cycles();
    let (mut last, mut moved) = (AUD_DONE, t0);
    while AUD_SENT - AUD_DONE >= ahead.max(1) {
        if AUD_SLOT == 0 {
            return false; // unplugged
        }
        if !crate::cpu::interrupts_on() {
            poll(); // nobody else will take the completions off the event ring
        }
        let now = crate::timer::cycles();
        if AUD_DONE != last {
            (last, moved) = (AUD_DONE, now);
        } else if now.wrapping_sub(moved) > crate::timer::hz() * 2 {
            crate::kprintln!("[xhci] audio: the device stopped taking packets ({} sent, {} done)", { AUD_SENT }, { AUD_DONE });
            AUD_DONE = AUD_SENT; // let the next stream start afresh
            return false;
        }
        core::hint::spin_loop();
    }
    true
}

/// Queue the packet being built (AUD_FILL bytes) on the isochronous ring, as soon as possible.
unsafe fn audio_send() -> bool {
    if !audio_wait(AUD_AHEAD) {
        return false;
    }
    let Some(home) = on(DEVS[AUD_D].ctl) else { return false };
    let buf = addr_of!(AUDBUF[(AUD_SENT % NAUD as u64) as usize]) as u64;
    // Isoch TRB: SIA (start as soon as possible), IOC; TBC/TLBPC 0 (one packet)
    push(addr_of_mut!(AUD_RING.0) as *mut Trb, NAUD, &mut *addr_of_mut!(PAUD), buf, AUD_FILL as u32, 1 << 31 | T_ISOCH << 10 | 1 << 5);
    AUD_SENT += 1;
    AUD_PKTS += 1;
    doorbell(AUD_SLOT, AUD_DCI);
    select(home);
    AUD_FILL = 0;
    AUD_NEED = next_packet();
    true
}

/// Open a playback stream (pairs with audio_stream_write and audio_stream_end).
pub fn audio_stream_begin() -> bool {
    // SAFETY: mainline, single core; the packet buffers are only written here.
    unsafe {
        if AUD_SLOT == 0 || !audio_wait(1) {
            return false;
        }
        (AUD_FILL, AUD_RS, AUD_SAMPLES, AUD_FRAMES, AUD_PKTS) = (0, 0, 0, 0, 0);
        AUD_NEED = next_packet();
        AUD_STREAMS += 1;
        true
    }
}

/// Mono 44.1 kHz samples, any number: each becomes as many frames as the device's rate
/// asks (sample-and-hold), copied to every channel.
pub fn audio_stream_write(samples: &[i16]) -> bool {
    // SAFETY: as audio_stream_begin.
    unsafe {
        if AUD_SLOT == 0 {
            return false;
        }
        for &s in samples {
            AUD_RS += AUD_RATE;
            while AUD_RS >= SRC_HZ {
                AUD_RS -= SRC_HZ;
                let p = (addr_of_mut!(AUDBUF[(AUD_SENT % NAUD as u64) as usize]) as *mut u8).add(AUD_FILL);
                let v = s.to_le_bytes();
                for c in 0..AUD_CH as usize {
                    (*p.add(2 * c), *p.add(2 * c + 1)) = (v[0], v[1]);
                }
                AUD_FILL += 2 * AUD_CH as usize;
                AUD_FRAMES += 1;
                if AUD_FILL >= AUD_NEED && !audio_send() {
                    return false;
                }
            }
        }
        AUD_SAMPLES += samples.len() as u64;
        true
    }
}

/// Send what is left (the last packet filled with silence), wait until the device has
/// played every packet, and say what was played.
pub fn audio_stream_end() -> bool {
    // SAFETY: as audio_stream_begin.
    unsafe {
        if AUD_SLOT == 0 {
            return false;
        }
        if AUD_FILL > 0 {
            let p = addr_of_mut!(AUDBUF[(AUD_SENT % NAUD as u64) as usize]) as *mut u8;
            core::ptr::write_bytes(p.add(AUD_FILL), 0, AUD_NEED - AUD_FILL);
            AUD_FILL = AUD_NEED;
            if !audio_send() {
                return false;
            }
        }
        if !audio_wait(1) {
            return false;
        }
        crate::kprintln!(
            "[usb-audio] played {} samples (~{} ms) as {} frames at {} Hz in {} packets",
            { AUD_SAMPLES }, AUD_SAMPLES * 1000 / SRC_HZ as u64, { AUD_FRAMES }, { AUD_RATE }, { AUD_PKTS }
        );
        true
    }
}

/// A 441 Hz square tone, 16384 samples (~371 ms), as `audio tone` plays everywhere.
pub fn audio_tone() -> bool {
    if !audio_stream_begin() {
        return false;
    }
    let half = (SRC_HZ / 441 / 2) as usize;
    let mut buf = [0i16; 1024];
    let mut phase = 0usize;
    for _ in 0..16 {
        for s in buf.iter_mut() {
            *s = if (phase / half) & 1 == 0 { 8000 } else { -8000 };
            phase += 1;
        }
        if !audio_stream_write(&buf) {
            return false;
        }
    }
    audio_stream_end()
}

/// `audio` when the sound goes to a USB headset or speaker.
pub fn audio_cmd(rest: &str) {
    if rest == "tone" {
        crate::kprintln!("[audio] streaming a test tone through the USB audio device...");
        if audio_tone() {
            crate::kprintln!("[audio] done: played by the USB headset or speaker");
        }
    } else {
        // SAFETY: scalar reads of enumeration-time state.
        unsafe {
            crate::kprintln!(
                "[audio] USB audio online: slot {}, {} Hz {}-channel 16-bit, {} bytes per packet at most; 'audio tone' to play",
                { AUD_SLOT }, { AUD_RATE }, { AUD_CH }, { AUD_MPS }
            );
        }
    }
}

/// An absolute pointer's report: the pointer goes to the place it names (buttons as they
/// were, so a held button drags), then the buttons change there; a tap lands where touched.
unsafe fn tablet_report(r: &[u8]) {
    let a = TAB;
    if a.id != 0 && r.first() != Some(&a.id) {
        return; // another of the device's reports
    }
    if (r.len() as u32) * 8 < (a.x.bit + a.x.size).max(a.y.bit + a.y.size) {
        return;
    }
    TREPORTS += 1;
    let (w, h) = crate::screen::pointer_area(); // the desktop canvas while it is shown (afriui.md 13)
    let (x, y) = (scale(r, a.x, w), scale(r, a.y, h));
    let mut buttons = 0u8;
    for k in 0..a.nbtn.min(3) {
        buttons |= (bits(r, a.btn + k, 1) as u8) << k;
    }
    mouse_report(MBUTTONS, x - MX, y - MY);
    mouse_report(buttons, 0, 0);
}

/// A wheel mouse's report (report protocol, laid out by its descriptor): the move through
/// the acceleration curve as a boot report's, and wheel notches scroll the console (HID:
/// positive = rolled away from the user, which scrolls back), not on the desktop.
unsafe fn mouse_rel_report(m: Rel, r: &[u8]) {
    if m.id != 0 && r.first() != Some(&m.id) {
        return; // another of the device's reports
    }
    if (r.len() as u32) * 8 < (m.x.bit + m.x.size).max(m.y.bit + m.y.size).max(m.wheel.bit + m.wheel.size) {
        return;
    }
    MREPORTS += 1;
    let mut buttons = 0u8;
    for k in 0..m.nbtn.min(3) {
        buttons |= (bits(r, m.btn + k, 1) as u8) << k;
    }
    let z = signed(r, m.wheel).clamp(-127, 127);
    if z != 0 {
        MWHEEL += z.unsigned_abs() as u64;
        crate::uart::console_wheel(-z); // the console, or the desk's Terminal while the desk shows
    }
    let (dx, dy) = mouse_accel(signed(r, m.x), signed(r, m.y));
    mouse_report(buttons, dx, dy);
}

/// A USB mouse report's move through the acceleration curve: its speed is its counts over
/// the time since the report before (at least 1 ms, at most 100 ms; after a longer pause
/// the carried fractions are dropped).
unsafe fn mouse_accel(dx: i32, dy: i32) -> (i32, i32) {
    let now = crate::timer::cycles();
    let ms = (now.wrapping_sub(M_LAST) / (crate::timer::hz() / 1000).max(1)).clamp(1, 100) as u32;
    M_LAST = now;
    if ms == 100 {
        M_REM = (0, 0);
    }
    let (ax, ay, sped) = crate::ps2::accelerate(dx, dy, ms, MOUSE_PER_MM, &mut *addr_of_mut!(M_REM));
    M_SPED += sped as u64;
    (ax, ay)
}

/// `accel` and `lsusb`: the USB mouse's acceleration and how many reports it sped up.
pub fn accel_line() {
    // SAFETY: scalar read of the counter.
    let sped = unsafe { M_SPED };
    if crate::ps2::accel_on() {
        crate::kprintln!(
            "usb: mouse acceleration on, the same curve at 16 counts/mm (400 dpi); {} report(s) sped up",
            sped
        );
    } else {
        crate::kprintln!("usb: mouse acceleration off (1:1); {} report(s) sped up before", sped);
    }
}

/// One pointer report: buttons (bit 0 left, 1 right, 2 middle), then signed x and y moves.
unsafe fn mouse_report(buttons: u8, dx: i32, dy: i32) {
    MCLICKS += (buttons & !MBUTTONS & 7).count_ones() as u64;
    let left_down = buttons & 1 != 0 && MBUTTONS & 1 == 0;
    let released_left = buttons & 1 == 0 && MBUTTONS & 1 != 0;
    MBUTTONS = buttons & 7;
    // the window manager: a left press on a title bar picks the window up, moves carry it,
    // the release puts it down
    // the desktop (desk.rs) takes presses, releases and moves as events, drawn outside the interrupt
    if crate::desk::shown() {
        if left_down {
            crate::desk::post_down(MX, MY);
        } else if buttons & 1 == 0 && released_left {
            crate::desk::post_up(MX, MY);
        }
    }
    if buttons & 1 == 0 {
        DRAG = None;
    } else if left_down && crate::screen::wm_view() {
        DRAG = crate::tonga::win_title_at(MX as usize, MY as usize);
        if let Some(id) = DRAG {
            DRAGS += 1;
            crate::tonga::win_raise(id); // the window taken by its title comes to the front
            crate::screen::wm_moved();
        }
    }
    let (w, h) = crate::screen::pointer_area(); // the desktop canvas while it is shown (afriui.md 13)
    let (x, y) = ((MX + dx).clamp(0, w - 1), (MY + dy).clamp(0, h - 1));
    if (x, y) != (MX, MY) {
        if let Some(id) = DRAG {
            crate::tonga::win_move(id, x - MX, y - MY);
            crate::screen::wm_moved();
        }
        MX = x;
        MY = y;
        if crate::desk::shown() {
            crate::desk::post_move(x, y);
        }
        crate::screen::mark(); // the pointer is drawn over the console on the next redraw
    }
}

/// A PS/2 mouse or touchpad packet (ps2.rs): the same pointer, window drags and dock clicks
/// as a USB mouse report, the move cut into steps a boot report can carry.
pub(crate) fn pointer_input(buttons: u8, mut dx: i32, mut dy: i32) {
    // SAFETY: called from the timer interrupt (ps2::poll), as mouse_report is from xhci::poll.
    unsafe {
        loop {
            let (sx, sy) = (dx.clamp(-127, 127), dy.clamp(-127, 127));
            mouse_report(buttons, sx, sy); // lsusb counts the USB mouse's own reports only
            dx -= sx;
            dy -= sy;
            if dx == 0 && dy == 0 {
                break;
            }
        }
    }
}

/// `lsusb`: the controller, its ports and the keyboard.
pub fn list() {
    // SAFETY: scalar reads of boot-time state.
    unsafe {
        if OP == 0 {
            crate::kprintln!("usb: no xHCI controller");
            return;
        }
        let Some(home) = on(CUR) else { return };
        for c in 0..NRUN.max(1) {
            select(c);
            let (b, d, f) = *addr_of!(PCI_AT);
            crate::kprintln!("usb: xHCI controller {:02x}:{:02x}.{}, {} ports, {} connected", b, d, f, { PORTS }, { CONNECTED });
            kinds_line(c, "  ");
            if RESTARTS[c] > 0 {
                crate::kprintln!("  controller resets (it stopped working, or xhcireset): {}", { RESTARTS[c] });
            }
            if GIVEN_UP[c] {
                crate::kprintln!("  this controller stopped working: its devices are not used until the next start");
            }
        }
        select(home);
        if HUBS > 0 {
            crate::kprintln!("  hubs: {} with {} ports between them", { HUBS }, { HUB_PORTS });
            crate::kprintln!("  hubs: {} watched for hot-plug behind them", { HUB_WATCHED });
        }
        if MSC_SLOT != 0 {
            crate::kprintln!(
                "  storage: slot {}, {} blocks of {} B{}",
                { MSC_SLOT }, { MSC_BLOCKS }, { MSC_BS }, if MSC_USABLE { "  <- storage" } else { "  (other data, read-only to us)" }
            );
            if MSC_UAS {
                crate::kprintln!(
                    "  storage transport: UAS (USB Attached SCSI, four pipes, {}), {} recoveries",
                    if UAS_STREAMS { "streams 1 and 2" } else { "no streams" }, { MSC_RECOVERIES }
                );
            }
            if MSC_BURST.0 > 0 {
                crate::kprintln!(
                    "  storage bursts: {} packets in, {} out (SuperSpeed endpoint companion)",
                    { MSC_BURST.0 }, { MSC_BURST.1 }
                );
            }
            if MSC_LONG {
                let c = match MSC_LONGCHK { 1 => "checked at the last block", 2 => "check did not pass", _ => "not checked" };
                crate::kprintln!("  storage commands: READ(16)/WRITE(16), over 2^32 blocks ({})", c);
            }
            if MSC_USABLE {
                let (r, ra, rm, r1) = MSC_RSTATS;
                let (w, wm, w1) = MSC_WSTATS;
                let st = |m: u8| match m { 1 => "on", 2 => "off", _ => "not checked" };
                crate::kprintln!(
                    "  storage reads: {} sector(s), {} from the read-ahead, {} multi-block command(s), {} single ({})",
                    r, ra, rm, r1, st(MSC_MULTI)
                );
                crate::kprintln!(
                    "  storage writes: {} sector(s), {} multi-block command(s), {} single ({})",
                    w, wm, w1, st(MSC_MULTIW)
                );
            }
            if MSC_RECOVERIES > 0 {
                crate::kprintln!("  storage: {} stall/phase recoveries", { MSC_RECOVERIES });
            }
            if MSC_STALE > 0 {
                crate::kprintln!("  storage: {} late transfer event(s) set aside, not taken as a later transfer's", { MSC_STALE });
            }
        }
        if CTL_STALE > 0 {
            crate::kprintln!("  control: {} late endpoint 0 answer(s) set aside, not taken as a later request's", { CTL_STALE });
        }
        if CTL_RESETS > 0 {
            crate::kprintln!("  control: endpoint 0 reset {} time(s) after the controller halted it", { CTL_RESETS });
        }
        if PLUGS + UNPLUGS > 0 {
            crate::kprintln!("  hot-plug: {} plugged in, {} unplugged since boot", { PLUGS }, { UNPLUGS });
        }
        addr_line();
        let ctl_of = |kind: u8| (0..NDEV).find(|&d| DEVS[d].slot != 0 && DEVS[d].kind == kind).map(|d| DEVS[d].ctl);
        let (kctl, mctl, tctl, actl) = (ctl_of(KIND_KBD), ctl_of(KIND_MOUSE), ctl_of(KIND_TABLET), ctl_of(KIND_AUDIO));
        for c in 0..NRUN.max(1) {
            select(c);
            if NRUN > 1 {
                let (b, d, f) = *addr_of!(PCI_AT);
                crate::kprintln!("  controller {:02x}:{:02x}.{}:", b, d, f);
            }
            for p in 1..=PORTS {
                let sc = r32(portsc(p));
                if sc & 1 != 0 {
                    crate::kprintln!(
                        "  port {}: connected, {}, speed {}{}",
                        p,
                        if sc & 2 != 0 { "enabled" } else { "disabled" },
                        (sc >> 10) & 0xF,
                        if p == KBD_PORT && KBD_SLOT != 0 && kctl == Some(c) {
                            "  <- keyboard"
                        } else if p == MOUSE_PORT && MOUSE_SLOT != 0 && mctl == Some(c) {
                            "  <- mouse"
                        } else if p == TABLET_PORT && TABLET_SLOT != 0 && tctl == Some(c) {
                            "  <- tablet"
                        } else if p == AUD_PORT && AUD_SLOT != 0 && actl == Some(c) {
                            "  <- audio"
                        } else if p == SER_PORT && SER_SLOT != 0 && SER_CTL == c {
                            "  <- serial"
                        } else if (0..NDEV).any(|d| DEVS[d].slot != 0 && DEVS[d].kind == KIND_MSC && DEVS[d].ctl == c && DEVS[d].port == p) && NRUN > 1 {
                            "  <- storage"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
        select(home);
        if KBD_SLOT != 0 {
            crate::kprintln!("  keyboard: slot {}, {} reports, {} keys typed", { KBD_SLOT }, { REPORTS }, { KEYS });
            crate::kprintln!("  keyboard: {} repeats while a key was held", { REPEATS });
            crate::kprintln!(
                "  keyboard: caps lock {}, its light set {} times, {} refused",
                if CAPS_LOCK { "on" } else { "off" }, { LED_SETS }, { LED_FAILS }
            );
        }
        if MOUSE_SLOT != 0 {
            crate::kprintln!(
                "  mouse: slot {}, {} reports, pointer at {},{}, buttons {:03b}, {} clicks",
                { MOUSE_SLOT }, { MREPORTS }, { MX }, { MY }, { MBUTTONS }, { MCLICKS }
            );
            crate::kprintln!("  mouse: {} window drag(s)", { DRAGS });
            crate::kprintln!(
                "  mouse: acceleration {}, {} report(s) sped up",
                if crate::ps2::accel_on() { "on" } else { "off" },
                { M_SPED }
            );
            if (*addr_of!(MREL)).is_some() {
                crate::kprintln!("  mouse: wheel on (report protocol); {} notch(es), 3 console lines each", { MWHEEL });
            } else {
                crate::kprintln!("  mouse: no wheel (boot protocol)");
            }
            if crate::desk::dock_clicks() > 0 {
                crate::kprintln!("  mouse: {} dock click(s) on the desktop", crate::desk::dock_clicks());
            }
        }
        if TABLET_SLOT != 0 {
            crate::kprintln!(
                "  tablet: slot {}, {} reports, pointer at {},{}, buttons {:03b}, {} clicks",
                { TABLET_SLOT }, { TREPORTS }, { MX }, { MY }, { MBUTTONS }, { MCLICKS }
            );
            crate::kprintln!("  tablet: {} window drag(s), {} dock click(s)", { DRAGS }, crate::desk::dock_clicks());
        }
        if AUD_SLOT != 0 {
            crate::kprintln!(
                "  audio: slot {}, {} Hz {}-channel 16-bit, {} stream(s), {} packets sent, {} played, {} missed",
                { AUD_SLOT }, { AUD_RATE }, { AUD_CH }, { AUD_STREAMS }, { AUD_SENT }, { AUD_DONE }, { AUD_MISSED }
            );
        }
        if SER_SLOT != 0 {
            crate::kprintln!(
                "  serial: slot {}, 115200 8N1, {} bytes sent, {} received, {} dropped, {} errors",
                { SER_SLOT }, { SER_TX }, { SER_RX }, { SER_DROP }, { SER_ERR }
            );
        }
    }
}

/// True when a USB boot keyboard is enumerated and polled.
pub fn keyboard_present() -> bool {
    // SAFETY: scalar read.
    unsafe { KBD_SLOT != 0 }
}

/// True when a USB boot mouse is enumerated and polled.
pub fn mouse_present() -> bool {
    // SAFETY: scalar read.
    unsafe { MOUSE_SLOT != 0 }
}

/// True when a USB absolute pointer (touchscreen, tablet, remote console mouse) is polled.
pub fn tablet_present() -> bool {
    // SAFETY: scalar read.
    unsafe { TABLET_SLOT != 0 }
}

/// The mouse pointer in shared-framebuffer pixels, when there is a mouse (USB mouse or
/// absolute pointer, or a PS/2 mouse or touchpad that has been used).
pub fn pointer() -> Option<(usize, usize)> {
    // SAFETY: scalar reads (an i32 each; a torn pair only misplaces one redraw).
    unsafe {
        if MOUSE_SLOT != 0 || TABLET_SLOT != 0 || crate::ps2::mouse_used() {
            Some((MX as usize, MY as usize))
        } else {
            None
        }
    }
}

// --- USB video: webcams (USB Video Class 1.0 to 1.5), slice 1 (13 Nov 2026, uvc.rs, UVC_PLAN.md) ---
// Every laptop's built-in camera is a UVC device on an internal USB port. The device is configured when it
// comes, but nothing streams until `camera on` (uvc.rs: typed at the local keyboard or serial console only).
// Then: the probe and commit controls, the endpoint, the alternate setting that carries the camera's packet
// size, and a ring of isochronous IN packets, each handed to uvc::packet before its TRB is given back.

/// The isochronous IN ring: 39 packets and the link. Every packet asks for an event (its length is needed), so
/// at most 39 camera events wait at once, which leaves room in the 64-entry event ring for the other devices.
const NCAM: usize = 40;
/// The most one service interval brings (high speed: 3 x 1024).
const CAM_PKT: usize = 3072;
/// Each packet buffer on its own 4 KiB, so none crosses a 64 KiB block (the architect's N3 rule).
#[repr(C, align(4096))]
struct CamPkt([u8; 4096]);
const _: () = assert!(CAM_PKT <= core::mem::size_of::<CamPkt>() && core::mem::align_of::<CamPkt>() >= core::mem::size_of::<CamPkt>());
/// The camera's whole configuration descriptor (cameras' run past the 512 bytes of DBUF): 8 KiB on an 8 KiB line.
const CAM_CFG: usize = 8192;
static mut CAM_RING: Ring<NCAM> = Ring([TRB0; NCAM]);
static mut PCAM: Prod = Prod { idx: 0, cycle: 1 };
static mut CAMBUF: [CamPkt; NCAM - 1] = [const { CamPkt([0; 4096]) }; NCAM - 1];
static mut CAMCFG: Room8k<{ room8k(1) }> = Room8k([0; room8k(1)]);
unsafe fn cam_cfg() -> *mut u8 {
    at8k(addr_of!(CAMCFG) as *const u8, 0)
}
static mut CAM: Option<crate::uvc::Cam> = None;
static mut CAM_D: usize = usize::MAX;
static mut CAM_SLOT: u32 = 0;
static mut CAM_PORT: u32 = 0;
static mut CAM_SPEED: u32 = 0;
static mut CAM_ID: (u16, u16) = (0, 0);
static mut CAM_DCI: u32 = 0;
static mut CAM_REQ: u32 = 0; // bytes asked for in each packet
static mut CAM_TLBPC: u32 = 0; // high speed: the transactions in each packet, less one
static mut CAM_RUN: bool = false;
static mut CAM_HALT: bool = false; // an error ended the stream: no packet is given back until it is stopped
static mut CAM_NEXT: usize = 0; // the ring slot whose completion comes next
/// `camera off` (or an error) has begun but the stream is not yet known to have stopped: its events are ignored,
/// the mark stays, and the idle loop tries again (ARCHITECT_REVIEWS 13 Nov night, fix 1).
static mut CAM_STOPPING: bool = false;
static mut CAM_STOP_SAID: bool = false;
/// (events, packets taken, ring-empty events, packets lost by the controller, bad packets)
static mut CAM_ST: (u64, u64, u64, u64, u64) = (0, 0, 0, 0, 0);
/// The AfriSovera test camera (scripts/ci/qemu-uvc-test.patch): its frames carry a frame number and are summed.
const TEST_CAM: (u16, u16) = (0x46F4, 0xAF01);

/// A control transfer with its data stage in `buf` (a DMA buffer inside one 64 KiB block), returning the bytes
/// moved. The data stage asks for a Short Packet event (ISP), so a short IN answer says how much came back.
unsafe fn control_buf(d: usize, rtype: u8, req: u8, value: u16, index: u16, len: u16, buf: u64) -> Option<usize> {
    let slot = DEVS[d].slot;
    let ring = addr_of_mut!(EP0[d].0) as *mut Trb;
    let p = &mut (*addr_of_mut!(PEP0))[d];
    let input = rtype & 0x80 != 0;
    let setup = rtype as u64 | (req as u64) << 8 | (value as u64) << 16 | (index as u64) << 32 | (len as u64) << 48;
    let trt = if len == 0 { 0 } else if input { 3 } else { 2 };
    let first = push_held(ring, NXFER, p, setup, 8, T_SETUP << 10 | 1 << 6 | trt << 16, true);
    let mut data = 0;
    if len > 0 {
        data = push_held(ring, NXFER, p, buf, len as u32, T_DATA << 10 | (input as u32) << 16 | 1 << 2, false) as u64;
    }
    let status_in = len == 0 || !input;
    let status = push_held(ring, NXFER, p, 0, 0, T_STATUS << 10 | 1 << 5 | (status_in as u32) << 16, false) as u64;
    release(first);
    doorbell(slot, 1);
    CTL_WANT = status;
    let mut got = len as usize;
    for _ in 0..2 {
        match wait_control(slot, [first as u64, data, status]) {
            Some(ev) if ev.status >> 24 == CC_SHORT && data != 0 && ev.param & !0xF == data => {
                got = (len as u32).saturating_sub(ev.status & 0xFF_FFFF) as usize;
                // the status stage's own event follows
            }
            Some(ev) if matches!(ev.status >> 24, CC_SUCCESS | CC_SHORT) => return Some(got),
            Some(ev) => {
                crate::kprintln!("[xhci] control request {:#04x} failed (completion code {})", req, ev.status >> 24);
                LAST_CC = ev.status >> 24;
                ep0_recover(d);
                return None;
            }
            None => {
                crate::kprintln!("[xhci] control request {:#04x} timed out", req);
                LAST_CC = 0;
                ep0_recover(d);
                return None;
            }
        }
    }
    None
}

/// A USB camera: its whole configuration read and walked (uvc::descriptors), the device configured. Nothing
/// streams until `camera on`. A camera with no YUY2 size up to 640x480 (MJPEG only) is named and left alone.
unsafe fn video_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize;
    let cval = DBUF.0[5];
    let room = CAM_CFG;
    if total < 9 || total > room {
        if total > room {
            crate::kprintln!("[xhci] port {}: device {:04x}:{:04x}: its configuration ({} bytes) is larger than the camera driver reads", port, vid, pid, total);
        }
        return false;
    }
    let Some(got) = control_buf(d, 0x80, 6, 0x0200, 0, total as u16, cam_cfg() as u64) else { return false };
    let cfg = &core::slice::from_raw_parts(cam_cfg() as *const u8, CAM_CFG)[..got.min(total)];
    let cam = match crate::uvc::descriptors(cfg) {
        Ok(c) => c,
        Err(crate::uvc::Bad::NoVideo) => return false,
        Err(e) => {
            crate::kprintln!("[xhci] port {}: USB camera {:04x}:{:04x} refused: {}", port, vid, pid, e);
            return false;
        }
    };
    let Some(s) = crate::uvc::pick(&cam) else {
        crate::kprintln!(
            "[xhci] port {}: USB camera {:04x}:{:04x}: no YUY2 picture up to 640x480 ({}): not driven yet",
            port, vid, pid, if cam.mjpeg { "MJPEG only, slice 2" } else { "other formats" }
        );
        return false;
    };
    if cam.nalts == 0 {
        crate::kprintln!("[xhci] port {}: USB camera {:04x}:{:04x}: no isochronous endpoint (a bulk camera, slice 3): not driven yet", port, vid, pid);
        return false;
    }
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    DEVS[d].kind = KIND_CAM;
    DEVS[d].dci = 0;
    NDEVS = NDEVS.max(d + 1);
    (CAM, CAM_D, CAM_SLOT, CAM_PORT, CAM_SPEED, CAM_ID) = (Some(cam), d, slot, port, speed, (vid, pid));
    (CAM_RUN, CAM_HALT) = (false, false);
    READY = true;
    crate::kprintln!(
        "[xhci] port {}: USB camera {:04x}:{:04x} ({} speed), slot {}, UVC {:x}.{:02x}, {} YUY2 size(s){}: {}x{} at {} a second; off until `camera on`",
        port, vid, pid, speed_name(speed), slot, cam.uvc >> 8, cam.uvc & 0xFF, cam.nsizes,
        if cam.mjpeg { ", MJPEG too (slice 2)" } else { "" },
        s.w, s.h, if s.interval > 0 { 10_000_000 / s.interval } else { 0 }
    );
    true
}

/// `camera`: the camera, its sizes and what is shown.
pub fn cam_list() {
    // SAFETY: shell context; plain copies of enumeration-time state.
    unsafe {
        let Some(c) = CAM.filter(|_| CAM_SLOT != 0) else {
            crate::kprintln!("camera: no USB camera (video class, YUY2) is plugged in");
            return;
        };
        let (vid, pid) = CAM_ID;
        crate::kprintln!(
            "camera: {:04x}:{:04x} on port {}, UVC {:x}.{:02x}, video interfaces {} and {}, {} format(s){}",
            vid, pid, CAM_PORT, c.uvc >> 8, c.uvc & 0xFF, c.vc_if, c.vs_if, c.formats,
            if c.mjpeg { ", MJPEG not shown until slice 2" } else { "" }
        );
        crate::kprint!("camera: YUY2 sizes:");
        for z in &c.sizes[..c.nsizes] {
            crate::kprint!(" {}x{}", z.w, z.h);
        }
        crate::kprintln!();
        if let Some(s) = crate::uvc::pick(&c) {
            crate::kprintln!(
                "camera: shows {}x{} at {} a second; {}",
                s.w, s.h, if s.interval > 0 { 10_000_000 / s.interval } else { 0 },
                if CAM_RUN { "on" } else { "off (type `camera on` at this keyboard)" }
            );
        }
    }
}

/// True while the driver is in the middle of an operation (enumeration, a control transfer): the idle loop waits.
pub fn busy() -> bool {
    // SAFETY: scalar read.
    unsafe { BUSY }
}

/// True while the camera streams (from `camera on` until it has been stopped).
pub fn cam_running() -> bool {
    // SAFETY: scalar read.
    unsafe { CAM_RUN }
}

/// True for the AfriSovera test camera of pc-usbcam, whose frames are summed for the grader.
pub fn cam_is_test() -> bool {
    // SAFETY: scalar reads.
    unsafe { CAM_SLOT != 0 && CAM_ID == TEST_CAM }
}

/// `camera stats`, the ring's line.
pub fn cam_ring_stats() {
    // SAFETY: counters the interrupt updates; a torn read only skews a count.
    let (ev, taken, empty, lost, bad) = unsafe { CAM_ST };
    crate::kprintln!(
        "camera ring: {} events, {} packets taken, {} ring-empty events, {} packets lost by the controller, {} bad packets",
        ev, taken, empty, lost, bad
    );
}

/// `camera on` (uvc.rs has already checked that it was typed at the local console).
pub fn cam_start() -> Result<(), &'static str> {
    // SAFETY: shell context; BUSY keeps the timer poll off the event ring while the controls and commands run.
    unsafe {
        if CAM_SLOT == 0 || CAM_D >= NDEV {
            return Err("no USB camera is plugged in");
        }
        if CAM_STOPPING {
            return Err("the camera is still stopping; the mark stays until it has");
        }
        if CAM_RUN {
            return Err("already on");
        }
        let Some(cam) = CAM else { return Err("no USB camera is plugged in") };
        let Some(s) = crate::uvc::pick(&cam) else { return Err("the camera has no YUY2 picture up to 640x480") };
        let Some(home) = on(DEVS[CAM_D].ctl) else { return Err("the USB controller is busy; try again") };
        let was = BUSY;
        BUSY = true;
        let r = cam_open(CAM_D, &cam, s);
        BUSY = was;
        select(home);
        r
    }
}

/// Device `d`'s route string and TT hub fields, as enumerate gave them, read back from its output context.
unsafe fn slot_path(d: usize) -> (u32, u32) {
    let o = addr_of_mut!(OUTCTX[d].0) as *mut u8;
    (read_volatile(ctx(o, 0)) & 0xF_FFFF, read_volatile(ctx(o, 0).add(2)) & 0xFFFF)
}

/// Probe and commit `s`, then the endpoint, the alternate setting and the packets.
unsafe fn cam_open(d: usize, cam: &crate::uvc::Cam, s: crate::uvc::Size) -> Result<(), &'static str> {
    let slot = DEVS[d].slot;
    let vs = cam.vs_if as u16;
    // the probe control's length: 26 bytes for UVC 1.0, 34 for 1.1, 48 for 1.5
    let plen: u16 = if cam.uvc < 0x0110 { 26 } else if cam.uvc < 0x0150 { 34 } else { 48 };
    let dbuf = addr_of!(DBUF) as u64;
    {
        let b = &mut (*addr_of_mut!(DBUF)).0;
        b[..48].fill(0);
        b[0] = 1; // bmHint: keep the frame interval
        (b[2], b[3]) = (s.format, s.frame);
        b[4..8].copy_from_slice(&s.interval.to_le_bytes());
    }
    // VS_PROBE_CONTROL (selector 1): SET_CUR, then GET_CUR for what the camera makes of it
    if control_buf(d, 0x21, 0x01, 0x0100, vs, plen, dbuf).is_none() {
        return Err("the camera refused the probe");
    }
    let got = control_buf(d, 0xA1, 0x81, 0x0100, vs, plen, dbuf).ok_or("the camera did not answer the probe")?;
    // the answer's length is checked before any field of it is read
    if got < 26 {
        return Err("the camera's probe answer is shorter than 26 bytes");
    }
    let b = (*addr_of!(DBUF)).0;
    if (b[2], b[3]) != (s.format, s.frame) {
        return Err("the camera chose another format or size than asked");
    }
    let frame = u32::from_le_bytes([b[18], b[19], b[20], b[21]]);
    let payload = u32::from_le_bytes([b[22], b[23], b[24], b[25]]);
    if frame != s.bytes {
        return Err("the camera's frame size is not width x height x 2");
    }
    if payload == 0 {
        return Err("the camera asks for no packet size");
    }
    let Some(alt) = crate::uvc::alt_for(cam, payload) else { return Err("no alternate setting carries the camera's packet size") };
    if alt.bytes() as usize > CAM_PKT {
        return Err("the camera's packets are larger than 3072 bytes");
    }
    // VS_COMMIT_CONTROL (selector 2): what the probe gave back
    if control_buf(d, 0x21, 0x01, 0x0200, vs, got as u16, dbuf).is_none() {
        return Err("the camera refused the commit");
    }
    // the isochronous IN endpoint: DCI = 2 x number + 1
    let dci = 2 * (alt.ep & 0xF) as u32 + 1;
    let interval = if CAM_SPEED >= 3 { (alt.interval as u32).clamp(1, 16) - 1 } else { (alt.interval as u32).clamp(1, 16) + 2 };
    *addr_of_mut!(PCAM) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(CAM_RING) as *mut u8, 0, core::mem::size_of::<Ring<NCAM>>());
    let (route, tt) = slot_path(d);
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << dci;
    let sl = ctx(inb, 1);
    *sl = CAM_SPEED << 20 | dci << 27 | route;
    *sl.add(1) = CAM_PORT << 16;
    *sl.add(2) = tt;
    let ec = ctx(inb, 1 + dci as usize);
    *ec = interval << 16;
    // CErr 0 (isochronous), isochronous IN (5), Max Burst = transactions - 1 at high speed, the packet size
    let burst = if CAM_SPEED == 3 { alt.mult as u32 - 1 } else { 0 };
    *ec.add(1) = 5 << 3 | burst << 8 | (alt.mps as u32) << 16;
    let tr = addr_of!(CAM_RING) as u64;
    *ec.add(2) = tr as u32 | 1;
    *ec.add(3) = (tr >> 32) as u32;
    *ec.add(4) = alt.bytes() | alt.bytes() << 16; // average TRB length, max ESIT payload
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return Err("the USB controller refused the camera's endpoint");
    }
    if !control(d, 0x01, 11, alt.alt as u16, vs, 0) {
        cam_drop(d, dci);
        return Err("the camera refused its alternate setting");
    }
    DEVS[d].dci = dci;
    (CAM_DCI, CAM_REQ, CAM_NEXT) = (dci, alt.bytes(), 0);
    // high speed: the packet's transactions less one (TLBPC, as Linux sets it; TBC stays 0)
    CAM_TLBPC = if CAM_SPEED == 3 { alt.bytes().div_ceil(alt.mps as u32).max(1) - 1 } else { 0 };
    CAM_ST = (0, 0, 0, 0, 0);
    crate::uvc::begin(s.w as usize, s.h as usize);
    (CAM_RUN, CAM_HALT, CAM_STOPPING, CAM_STOP_SAID) = (true, false, false, false);
    for k in 0..NCAM - 1 {
        cam_queue(k);
    }
    doorbell(slot, dci);
    crate::kprintln!(
        "[camera] on: {}x{} YUY2 at {} a second, alternate setting {} ({} bytes every {} us), endpoint {:#04x}; the CAMERA ON mark shows until it stops",
        s.w, s.h, if s.interval > 0 { 10_000_000 / s.interval } else { 0 }, alt.alt, alt.bytes(), 125u32 << interval, alt.ep
    );
    Ok(())
}

/// Packet buffer `k` onto the ring: one Isoch TRB, started as soon as possible (SIA), an event for every packet
/// (IOC, and ISP for a short one).
unsafe fn cam_queue(k: usize) {
    push(
        addr_of_mut!(CAM_RING.0) as *mut Trb,
        NCAM,
        &mut *addr_of_mut!(PCAM),
        addr_of!(CAMBUF[k]) as u64,
        CAM_REQ,
        1 << 31 | CAM_TLBPC << 16 | T_ISOCH << 10 | 1 << 5 | 1 << 2,
    );
}

/// Take the endpoint away again (its bandwidth back to the controller): Configure Endpoint with its drop flag.
/// True when the controller took it.
unsafe fn cam_drop(d: usize, dci: u32) -> bool {
    let (route, tt) = slot_path(d);
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0) = 1 << dci; // drop
    *ctx(inb, 0).add(1) = 1; // the slot context
    let sl = ctx(inb, 1);
    *sl = CAM_SPEED << 20 | 1 << 27 | route;
    *sl.add(1) = CAM_PORT << 16;
    *sl.add(2) = tt;
    let ok = command(inb as u64, T_CONFIGURE_EP << 10 | DEVS[d].slot << 24).is_some();
    if ok {
        DEVS[d].dci = 0;
    }
    ok
}

/// One camera event (timer interrupt, or a wait in the shell): the packet it names handed to uvc.rs, then its
/// TRB given back. The bytes are the length asked for less the residue, never more than the buffer; a residue
/// larger than the request is a bad packet. Missed service and the like count as lost packets, not as errors (a
/// real camera gets them); only a halt or a transaction error ends the stream.
/// A real controller may skip packets without an event for each, and may send Missed Service with no packet
/// named: an event for a slot ahead of CAM_NEXT gives the slots in between back as lost (uvc::slot), and a Missed
/// Service with no pointer only counts. Only a slot already given back is a fault.
unsafe fn cam_event(ev: &Trb, cc: u32) {
    if !CAM_RUN || CAM_HALT || CAM_STOPPING || (ev.control >> 16) & 0x1F != CAM_DCI {
        return;
    }
    CAM_ST.0 += 1;
    if cc == 14 || cc == 15 {
        // ring underrun/overrun: the ring ran empty (no packet named); the doorbell starts it again
        CAM_ST.2 += 1;
        doorbell(CAM_SLOT, CAM_DCI);
        return;
    }
    let base = addr_of!(CAM_RING) as u64;
    let at = ev.param & !0xF;
    if cc == 23 && at == 0 {
        // missed service with no packet named: the packets it skipped are given back when the next event names a
        // later slot
        crate::uvc::lost_packet();
        return;
    }
    if at < base || at >= base + 16 * (NCAM as u64 - 1) {
        CAM_ST.4 += 1;
        return;
    }
    let k = ((at - base) / 16) as usize;
    match crate::uvc::slot(CAM_NEXT, k, NCAM - 1) {
        crate::uvc::Slot::Next => {}
        crate::uvc::Slot::Ahead(gap) => {
            // the controller passed these packets without an event: lost, given back in ring order
            for _ in 0..gap {
                let j = CAM_NEXT;
                CAM_ST.3 += 1;
                crate::uvc::lost_packet();
                CAM_NEXT = (j + 1) % (NCAM - 1);
                cam_queue(j);
            }
        }
        crate::uvc::Slot::Behind => {
            CAM_HALT = true;
            crate::uvc::fail("a packet's event named a packet already given back");
            return;
        }
    }
    match cc {
        CC_SUCCESS | CC_SHORT => {
            let res = ev.status & 0xFF_FFFF;
            if res > CAM_REQ {
                CAM_ST.4 += 1; // more left over than was asked for: a bad packet, not an underflow
                crate::uvc::lost_packet();
            } else {
                let n = ((CAM_REQ - res) as usize).min(CAM_PKT);
                CAM_ST.1 += 1;
                crate::uvc::packet(&(&*addr_of!(CAMBUF[k])).0[..n]);
            }
        }
        4 | 6 => {
            CAM_HALT = true;
            crate::uvc::fail(if cc == 4 { "a USB transaction error" } else { "the endpoint halted" });
            return;
        }
        23 => {
            CAM_ST.3 += 1; // missed service: the controller could not serve this interval
            crate::uvc::lost_packet();
        }
        _ => {
            CAM_ST.4 += 1;
            crate::uvc::lost_packet();
        }
    }
    CAM_NEXT = (k + 1) % (NCAM - 1);
    cam_queue(k);
    doorbell(CAM_SLOT, CAM_DCI);
}

/// `camera off`, a stream error, or the end: Stop Endpoint, the alternate setting back to 0 (no bandwidth), the
/// endpoint dropped. The frames are zeroed and the mark taken off (uvc::stopped) only when the stream has really
/// stopped: Stop Endpoint succeeded, or the drop did (a halted endpoint cannot take Stop Endpoint), or the camera
/// is gone. Otherwise the mark stays, events are ignored, and the idle loop tries again. True when stopped.
pub fn cam_stop() -> bool {
    // SAFETY: shell context (the idle loop or a command); BUSY keeps the timer poll off the event ring.
    unsafe {
        if !CAM_RUN {
            return true;
        }
        CAM_STOPPING = true;
        let d = CAM_D;
        let mut done = CAM_SLOT == 0 || d >= NDEV;
        if !done {
            if let Some(home) = on(DEVS[d].ctl) {
                let was = BUSY;
                BUSY = true;
                let stopped = command(0, T_STOP_EP << 10 | CAM_SLOT << 24 | CAM_DCI << 16).is_some();
                if let Some(c) = CAM {
                    CTL_QUIET = true;
                    control(d, 0x01, 11, 0, c.vs_if as u16, 0);
                    CTL_QUIET = false;
                }
                let dropped = cam_drop(d, CAM_DCI);
                BUSY = was;
                select(home);
                done = stopped || dropped;
            }
        }
        if !done {
            if !CAM_STOP_SAID {
                CAM_STOP_SAID = true;
                crate::kprintln!("[camera] could not stop the camera yet; the mark stays (trying again)");
            }
            return false;
        }
        (CAM_RUN, CAM_HALT, CAM_STOPPING, CAM_STOP_SAID) = (false, false, false, false);
        crate::uvc::stopped();
        true
    }
}

/// True while a stop has begun but the stream is not yet known to have stopped (the mark stays).
pub fn cam_stopping() -> bool {
    // SAFETY: scalar read.
    unsafe { CAM_STOPPING }
}

// --- Bluetooth over USB, slice 1 (14 Nov 2026, bt.rs, BLUETOOTH_PLAN.md) ---
// A laptop's Bluetooth is a USB device of class E0/01/01 on an internal port. Its first interface carries HCI:
// commands go out as class control requests (bmRequestType 0x20, bRequest 0, wIndex 0), events come in on an
// interrupt-IN endpoint, one packet of at most its wMaxPacketSize at a time (bt.rs puts them together). The bulk
// endpoints (ACL data) and the isochronous interface (voice) are not used in slice 1. Nothing is sent at boot.

static mut BT_SLOT: u32 = 0;
static mut BT_D: usize = usize::MAX;
static mut BT_ID: (u16, u16) = (0, 0);
static mut BT_DCI: u32 = 0;
static mut BT_MPS: u32 = 16;
static mut BT_ERR: u64 = 0;

/// A Bluetooth controller: interface 0 of class E0/01/01 (alternate setting 0) with an interrupt-IN endpoint.
unsafe fn bt_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    if !control(d, 0x80, 6, 0x0200, 0, 9) {
        return false;
    }
    let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
    let cval = DBUF.0[5];
    if !control(d, 0x80, 6, 0x0200, 0, total as u16) {
        return false;
    }
    let (mut i, mut in_hci, mut ep) = (0usize, false, None);
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 || i + len > total {
            break;
        }
        match DBUF.0[i + 1] {
            4 if len >= 9 => {
                let class = (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]);
                in_hci = class == (0xE0, 0x01, 0x01) && DBUF.0[i + 3] == 0 && ep.is_none();
            }
            5 if in_hci && len >= 7 && DBUF.0[i + 3] & 3 == 3 && DBUF.0[i + 2] & 0x80 != 0 => {
                let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                ep = ep.or(Some((DBUF.0[i + 2] & 0xF, mps, DBUF.0[i + 6])));
            }
            _ => {}
        }
        i += len;
    }
    let Some((epn, mps, binterval)) = ep else { return false };
    if mps == 0 {
        return false;
    }
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    let dci = 2 * epn as u32 + 1;
    let interval = if speed >= 3 {
        (binterval as u32).clamp(1, 16) - 1
    } else {
        ((binterval as u32).max(1) * 8).ilog2().clamp(3, 10)
    };
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << dci;
    let sl = ctx(inb, 1);
    *sl = speed << 20 | dci << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    let ec = ctx(inb, 1 + dci as usize);
    *ec = interval << 16;
    *ec.add(1) = 3 << 1 | 7 << 3 | mps << 16; // CErr 3, interrupt IN
    let tr = addr_of!(EPIN[d]) as u64;
    *ec.add(2) = tr as u32 | 1;
    *ec.add(3) = (tr >> 32) as u32;
    *ec.add(4) = mps | mps << 16; // average TRB length, max ESIT payload
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].dci = dci;
    DEVS[d].kind = KIND_BT;
    NDEVS = NDEVS.max(d + 1);
    (BT_D, BT_ID, BT_DCI, BT_MPS, BT_ERR) = (d, (vid, pid), dci, mps.clamp(8, 512), 0);
    BT_SLOT = slot;
    bt_queue(d);
    crate::kprintln!(
        "[xhci] port {}: Bluetooth controller {:04x}:{:04x} ({} speed), slot {}, events on endpoint {}; nothing is sent until `bt`",
        port, vid, pid, speed_name(speed), slot, epn
    );
    true
}

/// One event packet asked for on the interrupt-IN ring.
unsafe fn bt_queue(d: usize) {
    push(
        addr_of_mut!(EPIN[d].0) as *mut Trb,
        NXFER,
        &mut (*addr_of_mut!(PEPIN))[d],
        addr_of!(REPORT[d]) as u64,
        BT_MPS,
        T_NORMAL << 10 | 1 << 5 | 1 << 2, // IOC, ISP
    );
    doorbell(DEVS[d].slot, BT_DCI);
}

/// An event packet (timer interrupt, or a wait in the shell): to bt.rs, then the next one asked for.
unsafe fn bt_event(d: usize, ev: &Trb, cc: u32) {
    if (ev.control >> 16) & 0x1F != BT_DCI {
        return;
    }
    if cc == CC_SUCCESS || cc == CC_SHORT {
        let got = (BT_MPS as usize).saturating_sub((ev.status & 0xFF_FFFF) as usize).min(512);
        crate::bt::packet(&(&*addr_of!(REPORT[d])).0[..got]);
    } else {
        BT_ERR += 1;
        if BT_ERR > 16 {
            return; // a halted endpoint: no more asking (unplugging resets it)
        }
    }
    bt_queue(d);
}

/// True while a Bluetooth controller is plugged in.
pub fn bt_present() -> bool {
    // SAFETY: scalar read.
    unsafe { BT_SLOT != 0 }
}

/// The controller's USB vendor and product id.
pub fn bt_id() -> (u16, u16) {
    // SAFETY: scalar read.
    unsafe { BT_ID }
}

/// One HCI command packet (opcode, length, parameters) to the controller: a class request on endpoint 0.
pub fn bt_command(pkt: &[u8]) -> bool {
    if pkt.len() > 258 {
        return false;
    }
    // SAFETY: shell context; BUSY keeps the timer poll off the event ring while the request runs.
    unsafe {
        if BT_SLOT == 0 || BT_D >= NDEV {
            return false;
        }
        let d = BT_D;
        let Some(home) = on(DEVS[d].ctl) else { return false };
        let was = BUSY;
        BUSY = true;
        (&mut (*addr_of_mut!(DBUF)).0)[..pkt.len()].copy_from_slice(pkt);
        let ok = control(d, 0x20, 0, 0, 0, pkt.len() as u16);
        BUSY = was;
        select(home);
        ok
    }
}
