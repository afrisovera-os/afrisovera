//! xHCI (USB 3) host controller, a USB boot keyboard and a USB boot mouse on the PC: roadmap
//! PC step 3, "USB keyboard/mouse (xHCI)" (keyboard 5 Nov 2026, mouse 5 Nov 2026).
//!
//! Every PC since ~2012 has an xHCI controller (PCI class 0C:03:30), and a laptop's own
//! keyboard is often a USB device behind it. This driver brings the controller up (command
//! ring, one event ring), resets each connected port, gives the first HID boot keyboard a
//! slot and an address, reads its descriptors, configures its interrupt-IN endpoint, and
//! from then on the local-APIC timer interrupt turns each 8-byte boot report into
//! characters on the same console input ring COM1 fills -- so the shell can be typed on a
//! real USB keyboard. A HID boot mouse (3/1/2) on another port is enumerated the same way;
//! its 3-byte reports move a pointer over the 640x480 console (clamped, drawn by screen.rs)
//! and count button presses. One keyboard and one mouse, polled, no hot-plug yet.
//! xHCI 1.2: registers 5, rings/TRBs 4.9-4.11 and 6.4, contexts 6.2, enumeration 4.3.

use crate::mmu;
use core::arch::asm;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

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
const T_SET_TR_DEQ: u32 = 16;
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

#[repr(C, align(64))]
struct Ring<const N: usize>([Trb; N]);
#[repr(C, align(64))]
struct Erst([u64; 2]); // one segment: base, size
#[repr(C, align(64))]
struct Dcbaa([u64; 256]);
#[repr(C, align(64))]
struct Ctx([u8; 4096]);
#[repr(C, align(64))]
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
const NDEV: usize = 7; // a keyboard, a mouse, a USB Ethernet adapter, a hub, a USB stick, a headset, a serial cable
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

/// A HID report descriptor -> the first absolute X/Y pair (Generic Desktop X 0x30, Y 0x31,
/// input, variable, not relative) and, in the same report, the buttons (Button page 1..3) or
/// else a digitizer's tip switch (0x0D:0x42). None for a relative mouse, a keyboard, or
/// anything without absolute X and Y. Push/pop and long items are skipped.
fn parse_abs(r: &[u8]) -> Option<Abs> {
    let (mut page, mut lmin, mut lmax, mut lmax_u, mut rsize, mut rcount, mut rid) = (0u32, 0i32, 0i32, 0i32, 0u32, 0u32, 0u8);
    let (mut usages, mut nu, mut umin, mut umax, mut range) = ([0u32; 16], 0usize, 0u32, 0u32, false);
    let (mut offs, mut noffs) = ([(0u8, 0u32); 8], 0usize); // (report id, its next input bit)
    let mut x: Option<(u8, Field)> = None;
    let mut y: Option<Field> = None;
    let (mut btns, mut nb) = ([(0u8, 0u32, 0u32); 4], 0usize); // first buttons per report id
    let (mut tips, mut nt) = ([(0u8, 0u32); 4], 0usize); // first tip switch per report id
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
                        let f = Field { bit: base + k * rsize, size: rsize, min: lmin, max };
                        let absolute = u & 4 == 0 && rsize > 0 && rsize <= 32 && max > lmin;
                        match usage {
                            0x0001_0030 if absolute && x.is_none() => x = Some((rid, f)),
                            0x0001_0031 if absolute && y.is_none() && x.map(|x| x.0) == Some(rid) => y = Some(f),
                            0x0009_0001 if rsize == 1 && nb < btns.len() && !btns[..nb].iter().any(|e| e.0 == rid) => {
                                btns[nb] = (rid, f.bit, (rcount - k).min(3));
                                nb += 1;
                            }
                            0x000D_0042 if rsize == 1 && nt < tips.len() && !tips[..nt].iter().any(|e| e.0 == rid) => {
                                tips[nt] = (rid, f.bit);
                                nt += 1;
                            }
                            _ => {}
                        }
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
static mut MSC_TAG: u32 = 1;
static mut MSC_BLOCKS: u64 = 0;
static mut MSC_BS: u32 = 512;
static mut MSC_USABLE: bool = false;
static mut MSC_NAME: [u8; 24] = [b' '; 24];
/// Where the device being enumerated sits behind hubs: its route string (one 4-bit hub port
/// per tier) and, for a full/low-speed device behind a high-speed hub, that hub's slot and
/// port (the transaction translator). (0, 0, 0) for a device on a root port.
static mut PATH: (u32, u32, u32) = (0, 0, 0);
static mut HUBS: u32 = 0;
/// The last device enumerated without being used: (vendor, product, device class, the class
/// of its first interface) -- for the boot log, so a real machine's report says what it was.
static mut LAST_OTHER: (u16, u16, u8, u8) = (0, 0, 0, 0);
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
/// (a USB controller was set up, the devices it found and does not use, how many).
pub(crate) fn others() -> (bool, [(u16, u16, u8, u8); 8], usize) {
    // SAFETY: single core, shell context; plain copies of boot/hot-plug bookkeeping.
    unsafe { (READY, OTHERS, NOTHERS) }
}
static mut HUB_PORTS: u32 = 0;

// --- USB Ethernet (CDC-ECM or RNDIS): one adapter, bulk IN and OUT ---
const NRXB: usize = 4; // receive transfers kept queued on the bulk-IN ring
const NTXB: usize = 8; // transmit buffers, used round robin
const NRXQ: usize = 16; // received frames waiting for the network stack
const FRAME: usize = 2048;
#[repr(C, align(64))]
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
/// One enumerated HID boot device: its slot, interrupt-IN DCI and kind.
#[derive(Clone, Copy)]
struct Dev {
    slot: u32,
    dci: u32,
    kind: u8,
    port: u32, // the root port it hangs off (directly or behind a hub)
    ctl: usize, // the controller it is on (slot and port numbers are that controller's)
    route: u32, // its route string behind hubs (0 on a root port)
}
const NO_DEV: Dev = Dev { slot: 0, dci: 0, kind: 0, port: 0, ctl: 0, route: 0 };
static mut DEVS: [Dev; NDEV] = [NO_DEV; NDEV];
/// Root ports whose connection changed (port-status-change events), for `service`, per controller.
static mut PENDING: [u32; NCTL] = [0; NCTL];
/// Per hub (by device index): the ports its status-change endpoint reported (bit n = port n),
/// for `service`; and what its ports need -- (speed, TT hub slot, TT port, number of ports).
static mut HUBCHG: [u16; NDEV] = [0; NDEV];
static mut HUBI: [(u32, u32, u32, u32); NDEV] = [(0, 0, 0, 0); NDEV];
static mut HUB_WATCHED: u32 = 0; // hubs whose status-change endpoint runs (hot-plug behind them)
/// `service` is enumerating: the timer poll stands aside (the waits serve events meanwhile).
static mut BUSY: bool = false;
static mut PLUGS: u64 = 0;
static mut UNPLUGS: u64 = 0;

/// A free device entry (never used, or freed by an unplug).
fn free_dev() -> Option<usize> {
    // SAFETY: single core; read-only look at the table.
    unsafe { (0..NDEV).find(|&d| DEVS[d].slot == 0) }
}
static mut NDEVS: usize = 0;
static mut KBD_SLOT: u32 = 0;
static mut KBD_PORT: u32 = 0;
static mut MOUSE_SLOT: u32 = 0;
static mut MOUSE_PORT: u32 = 0;
static mut READY: bool = false;
// the pointer, in shared-framebuffer pixels (640x480), starting in the middle
static mut MX: i32 = (crate::framebuffer::WIDTH / 2) as i32;
static mut MY: i32 = (crate::framebuffer::HEIGHT / 2) as i32;
static mut MBUTTONS: u8 = 0;
static mut MREPORTS: u64 = 0;
static mut MCLICKS: u64 = 0;
/// The Tonga window being dragged by its title bar (left button held), if any.
static mut DRAG: Option<usize> = None;
static mut DRAGS: u64 = 0;
static mut DOCK_CLICKS: u64 = 0;
static mut LAST_KEYS: [u8; 6] = [0; 6];
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

fn wait(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..30_000_000u64 {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

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
    let t = read_volatile(ev.add(EV_IDX));
    if t.control & 1 != EV_CYCLE {
        return None;
    }
    EV_IDX += 1;
    if EV_IDX == NEV {
        EV_IDX = 0;
        EV_CYCLE ^= 1;
    }
    w64(RT + 0x20 + 0x18, ev.add(EV_IDX) as u64 | 1 << 3); // ERDP, clear EHB
    Some(t)
}

/// Wait for an event of `ty` (others -- port changes -- are passed over).
unsafe fn wait_event(ty: u32) -> Option<Trb> {
    for _ in 0..30_000_000u64 {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == ty {
                return Some(t);
            }
            handle(t); // another device's transfer: served, not dropped
        }
        core::hint::spin_loop();
    }
    None
}

/// Wait for the transfer event of endpoint 0 of `slot` (a control transfer); every other
/// event is served as the timer poll would serve it. Dropping them lost a running device's
/// report (its transfer was never re-queued) whenever a second device enumerated.
unsafe fn wait_control(slot: u32) -> Option<Trb> {
    for _ in 0..30_000_000u64 {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == T_EV_TRANSFER && t.control >> 24 == slot && (t.control >> 16) & 0x1F == 1 {
                return Some(t);
            }
            handle(t);
        }
        core::hint::spin_loop();
    }
    None
}

/// One command on the command ring; its completion event, if it succeeded.
unsafe fn command(param: u64, control: u32) -> Option<Trb> {
    push(addr_of_mut!(CMD[CUR].0) as *mut Trb, NCMD, &mut *addr_of_mut!(PCMD), param, 0, control);
    doorbell(0, 0);
    let ev = wait_event(T_EV_COMMAND)?;
    if ev.status >> 24 != CC_SUCCESS {
        crate::kprintln!("[xhci] command type {} failed (completion code {})", (control >> 10) & 0x3F, ev.status >> 24);
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
    if len > 0 {
        push(ring, NXFER, p, addr_of!(DBUF) as u64, len as u32, T_DATA << 10 | (input as u32) << 16);
    }
    let status_in = len == 0 || !input;
    push(ring, NXFER, p, 0, 0, T_STATUS << 10 | 1 << 5 | (status_in as u32) << 16);
    release(first); // the whole TD is written: only now may the controller see its first TRB
    doorbell(slot, 1);
    match wait_control(slot) {
        Some(ev) if matches!(ev.status >> 24, CC_SUCCESS | CC_SHORT) => true,
        Some(ev) => {
            crate::kprintln!("[xhci] control request {:#04x} failed (completion code {})", req, ev.status >> 24);
            false
        }
        None => {
            crate::kprintln!("[xhci] control request {:#04x} timed out", req);
            false
        }
    }
}

fn ctx(base: *mut u8, i: usize) -> *mut u32 {
    // SAFETY: an index into one of the 4 KiB context buffers.
    unsafe { base.add(i * CSZ) as *mut u32 }
}

/// Every PCI function of class `class` (24 bits: class, subclass, programming interface) on
/// buses 0-7, in bus order, with its vendor:device id; at most 4.
fn pci_find(class: u32) -> ([(u8, u8, u8, u32); 4], usize) {
    let mut out = [(0, 0, 0, 0); 4];
    let mut n = 0;
    for b in 0..8u8 {
        for d in 0..32u8 {
            for f in 0..8u8 {
                let id = cfg32(b, d, f, 0);
                if id & 0xFFFF == 0xFFFF {
                    if f == 0 {
                        break;
                    }
                    continue;
                }
                if cfg32(b, d, f, 8) >> 8 == class && n < 4 {
                    out[n] = (b, d, f, id);
                    n += 1;
                }
            }
        }
    }
    (out, n)
}

/// BAR0 of the controller at b:d.f, mapped, with memory decoding on; None if it has none.
fn bar0(b: u8, d: u8, f: u8) -> Option<u64> {
    let lo = cfg32(b, d, f, 0x10);
    let bar = (lo & !0xF) as u64 | if lo & 0x6 == 0x4 { (cfg32(b, d, f, 0x14) as u64) << 32 } else { 0 };
    if bar == 0 || !mmu::map_mmio(bar, 0x10000) {
        return None;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory decoding + bus mastering, INTx off (polled)
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
                if !wait(|| r32(at) & 1 << 16 == 0) {
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
    let (ehci, n) = pci_find(0x0C_03_20);
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

/// A look at a controller that is not driven yet: taken from the firmware, ports switched (Intel),
/// reset, ports powered; returns how many ports have a device connected. Left halted.
fn probe(b: u8, d: u8, f: u8) -> u32 {
    let Some(bar) = bar0(b, d, f) else { return 0 };
    handoff(bar);
    intel_switch(b, d, f, false);
    let op = bar + (r32(bar) & 0xFF) as u64;
    w32(op, r32(op) & !1);
    wait(|| r32(op + 0x04) & 1 != 0);
    w32(op, 1 << 1);
    if !wait(|| r32(op) & 1 << 1 == 0 && r32(op + 0x04) & 1 << 11 == 0) {
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
    let (all, n) = pci_find(0x0C_03_30);
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
        intel_switch(b, d, f, true);
        // stop, reset, wait for the controller to be ready
        w32(op(0x00), r32(op(0x00)) & !1);
        wait(|| r32(op(0x04)) & 1 != 0);
        w32(op(0x00), 1 << 1);
        if !wait(|| r32(op(0x00)) & 1 << 1 == 0 && r32(op(0x04)) & 1 << 11 == 0) {
            crate::kprintln!("[xhci] controller did not come out of reset");
            return false;
        }
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
                let Some(page) = crate::frame::alloc() else {
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
        if !wait(|| r32(op(0x04)) & 1 == 0) {
            crate::kprintln!("[xhci] controller did not start");
            return false;
        }
        let vs = r32(bar) >> 16;
        crate::kprintln!(
            "[xhci] controller {:02x}:{:02x}.{} (xHCI {}.{}), {} ports, {} slots, {}-byte contexts",
            b, d, f, vs >> 8, (vs >> 4) & 0xF, { PORTS }, max_slots, { CSZ }
        );
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
                wait(|| r32(portsc(p)) & 1 << 21 != 0); // PRC
                crate::timer::busywait_ms(20); // reset recovery (USB 2.0 TRSTRCY, 10 ms min)
            }
            let sc = r32(portsc(p));
            w32(portsc(p), (sc & !(1 << 1 | 0x00FE_0000 | 1 << 4 | 1 << 31 | 1 << 16)) | (sc & 0x00FE_0000)); // clear changes
            if let (true, Some(fd)) = (sc & 1 << 1 != 0, free_dev()) {
                if !hid(fd, p, (sc >> 10) & 0xF) {
                    note_other();
                    let (v, pr, dc, ic) = LAST_OTHER;
                    crate::kprintln!(
                        "[xhci] port {}: device {:04x}:{:04x} ({}) is not one this driver uses",
                        p, v, pr, class_name(if dc != 0 { dc } else { ic })
                    );
                }
            }
        }
        READY = true; // port changes are served from now on, even with nothing plugged in
        PENDING[c] = 0; // the changes our own boot-time resets caused are not news
        true
    }
}

/// Enumerate the device on port `port` (speed code `speed`) as device `d`; true if it is a
/// boot keyboard or boot mouse (one of each is kept) and its interrupt endpoint is now running.
/// A device that is neither gets its slot disabled again, so `d` stays free.
unsafe fn hid(d: usize, port: u32, speed: u32) -> bool {
    let ev = match command(0, T_ENABLE_SLOT << 10) {
        Some(e) => e,
        None => return false,
    };
    let slot = ev.control >> 24;
    DEVS[d] = Dev { slot, dci: 0, kind: 0, port, ctl: CUR, route: PATH.0 };
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
        3 => 64,  // high speed
        4 => 512, // super speed
        _ => 8,   // full / low speed: 8 until the descriptor says otherwise
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
    if command(inb as u64, T_ADDRESS_DEVICE << 10 | slot << 24).is_none() {
        return false;
    }
    // device descriptor: first 8 bytes for the real EP0 packet size, then all 18
    if !control(d, 0x80, 6, 0x0100, 0, 8) {
        return false;
    }
    let real = DBUF.0[7] as u32;
    if speed < 3 && real != mps0 && real >= 8 {
        *ctx(inb, 0) = 0;
        *ctx(inb, 0).add(1) = 0b10; // evaluate EP0
        *e0.add(1) = 3 << 1 | 4 << 3 | real << 16;
        if command(inb as u64, T_EVALUATE_CTX << 10 | slot << 24).is_none() {
            return false;
        }
    }
    if !control(d, 0x80, 6, 0x0100, 0, 18) {
        return false;
    }
    let (vid, pid) = (u16::from_le_bytes([DBUF.0[8], DBUF.0[9]]), u16::from_le_bytes([DBUF.0[10], DBUF.0[11]]));
    let num_configs = DBUF.0[17];
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
    // find a HID boot keyboard (3/1/1) or boot mouse (3/1/2) interface -- one of a kind not
    // already enumerated -- and its interrupt-IN endpoint
    let (mut i, mut iface, mut in_kbd, mut ep, mut kind) = (0usize, 0u8, false, None, 0u8);
    // and the first other HID interface (not a keyboard), in case it is an absolute pointer:
    // (interface, report descriptor length, its interrupt-IN endpoint)
    let (mut in_tab, mut tab): (bool, Option<(u8, u16, Option<(u8, u32, u8)>)>) = (false, None);
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
        return (SER_SLOT == 0 && serial_setup(d, port, speed, vid, pid))
            || (AUD_SLOT == 0 && audio_setup(d, port, speed, vid, pid))
            || (NET_SLOT == 0 && net_setup(d, port, speed, num_configs, vid, pid))
            || (MSC_SLOT == 0 && msc_setup(d, port, speed, vid, pid));
    };
    if !control(d, 0x00, 9, config_value as u16, 0, 0) {
        return false; // SET_CONFIGURATION
    }
    if kind != KIND_TABLET {
        control(d, 0x21, 0x0B, 0, iface as u16, 0); // SET_PROTOCOL(boot); a refusal is survivable
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
        match speed { 1 => "full", 2 => "low", 3 => "high", 4 => "super", _ => "?" },
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
        if DEVS[d].kind == KIND_TABLET { TAB_LEN } else { 8 },
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
            MREPORTS += 1;
            mouse_report(r[0], r[1] as i8 as i32, r[2] as i8 as i32);
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
        for &k in &r[2..8] {
            if k > 3 && !last.contains(&k) {
                if let Some(c) = ascii(k, shift) {
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

/// A USB class code in words, for the boot log.
fn class_name(c: u8) -> &'static str {
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
        if !READY || BUSY || ((*addr_of!(PENDING)).iter().all(|&p| p == 0) && (*addr_of!(HUBCHG)).iter().all(|&h| h == 0)) {
            return;
        }
        BUSY = true;
        let home = CUR;
        for c in 0..NRUN.max(1) {
            if PENDING[c] != 0 {
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
        select(home);
        BUSY = false;
    }
}

/// Free device `d` on controller `c` (the current one): its slot disabled, what it was used
/// for forgotten, the entry freed. A hub also takes its port count out of `lsusb`'s total.
unsafe fn detach(d: usize, c: usize) {
    let (slot, kind) = (DEVS[d].slot, DEVS[d].kind);
    command(0, T_DISABLE_SLOT << 10 | slot << 24);
    DCBAA[c].0[slot as usize] = 0;
    match kind {
        KIND_KBD => {
            KBD_SLOT = 0;
            HELD = None;
        }
        KIND_MOUSE => MOUSE_SLOT = 0,
        KIND_TABLET => TABLET_SLOT = 0,
        KIND_NET => NET_SLOT = 0,
        KIND_AUDIO => AUD_SLOT = 0,
        KIND_SERIAL => {
            SER_SLOT = 0;
            SER_INFLIGHT = false;
        }
        KIND_MSC => {
            MSC_SLOT = 0;
            MSC_USABLE = false;
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
    UNPLUGS += 1;
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
/// everything behind it, a new one reset and enumerated -- as a root port's are.
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
/// `fd`, with its route string and transaction translator; true if this driver uses it.
unsafe fn hub_port(d: usize, hp: u32, fd: usize) -> bool {
    let (speed, tts, ttp, _) = HUBI[d];
    let (port, slot, saved) = (DEVS[d].port, DEVS[d].slot, PATH);
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
    crate::timer::busywait_ms(20); // reset recovery
    let Some((st, _)) = hub_status(d, hp) else { return false };
    if !reset_done || st & 2 == 0 {
        crate::kprintln!("[xhci] hub port {}: device did not enable after reset", hp);
        return false;
    }
    let cspeed = if st & 1 << 9 != 0 { 2 } else if st & 1 << 10 != 0 { 3 } else { 1 };
    let tt = if speed == 3 && cspeed < 3 { (slot, hp) } else { (tts, ttp) };
    PATH = (hub_port_route(d, hp).0, tt.0, tt.1);
    let ok = hid(fd, port, cspeed);
    if !ok {
        note_other();
        let (v, pr, dc, ic) = LAST_OTHER;
        crate::kprintln!(
            "[xhci] hub port {}: device {:04x}:{:04x} ({}) is not one this driver uses",
            hp, v, pr, class_name(if dc != 0 { dc } else { ic })
        );
    }
    PATH = saved;
    ok
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
                for d in 0..NDEV {
                    if DEVS[d].slot != 0 && DEVS[d].port == p && DEVS[d].ctl == c {
                        detach(d, c);
                        CONNECTED = CONNECTED.saturating_sub(1);
                    }
                }
                crate::kprintln!("[xhci] port {}: device unplugged", p);
                continue;
            }
            if present {
                continue;
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
                wait(|| r32(portsc(p)) & 1 << 21 != 0);
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
            if hid(fd, p, (sc >> 10) & 0xF) {
                PLUGS += 1;
                crate::kprintln!("[xhci] port {}: device plugged in and ready", p);
            } else {
                note_other();
                let (v, pr, dc, ic) = LAST_OTHER;
                crate::kprintln!(
                    "[xhci] port {}: device {:04x}:{:04x} ({}) plugged in, not one this driver uses",
                    p, v, pr, class_name(if dc != 0 { dc } else { ic })
                );
            }
        }
    }
}

/// A USB hub (class 9): select its configuration, tell the controller it is a hub (slot
/// context Hub bit and port count), power its ports, and enumerate whatever is connected
/// to each -- with the route string and, behind a high-speed hub, the transaction
/// translator the controller needs for full/low-speed devices. Then its status-change
/// endpoint is started: a device plugged into or pulled out of one of its ports later is
/// served by `service` (`service_hub`), as a root port's is.
unsafe fn hub_setup(d: usize, port: u32, speed: u32, vid: u16, pid: u16, protocol: u8) -> bool {
    let slot = DEVS[d].slot;
    if speed == 4 {
        // a SuperSpeed hub speaks another descriptor and port-status format (and wants
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
        port, vid, pid, match speed { 1 => "full", 2 => "low", 3 => "high", 4 => "super", _ => "?" }, slot, nports
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
/// already AFRIVFS1 -- the stick the PC booted from (FAT) is listed and never written.
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
    while i + 2 <= total {
        let len = DBUF.0[i] as usize;
        if len < 2 {
            break;
        }
        match DBUF.0[i + 1] {
            4 if i + 9 <= total => {
                ours = (DBUF.0[i + 5], DBUF.0[i + 6], DBUF.0[i + 7]) == (8, 6, 0x50);
                if ours {
                    MSC_IFACE = DBUF.0[i + 2];
                }
            }
            5 if ours && i + 7 <= total => {
                let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
                let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                if attr & 3 == 2 {
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
    let (Some((epi, mpsi)), Some((epo, mpso))) = (ein, eout) else { return false };
    if !control(d, 0x00, 9, cval as u16, 0, 0) {
        return false;
    }
    let (din, dout) = (2 * epi as u32 + 1, 2 * epo as u32);
    *addr_of_mut!(PMSCOUT) = Prod { idx: 0, cycle: 1 };
    core::ptr::write_bytes(addr_of_mut!(MSC_OUT_RING) as *mut u8, 0, core::mem::size_of::<Ring<NXFER>>());
    let inb = addr_of_mut!(INCTX.0) as *mut u8;
    core::ptr::write_bytes(inb, 0, 4096);
    *ctx(inb, 0).add(1) = 1 | 1 << din | 1 << dout;
    let sl = ctx(inb, 1);
    *sl = speed << 20 | din.max(dout) << 27 | PATH.0;
    *sl.add(1) = port << 16;
    *sl.add(2) = PATH.1 | PATH.2 << 8;
    for (dci, tr, ty, mps) in [(din, addr_of!(EPIN[d]) as u64, 6u32, mpsi), (dout, addr_of!(MSC_OUT_RING) as u64, 2u32, mpso)] {
        let ec = ctx(inb, 1 + dci as usize);
        *ec.add(1) = 3 << 1 | ty << 3 | mps << 16;
        *ec.add(2) = tr as u32 | 1;
        *ec.add(3) = (tr >> 32) as u32;
        *ec.add(4) = 1024;
    }
    if command(inb as u64, T_CONFIGURE_EP << 10 | slot << 24).is_none() {
        return false;
    }
    DEVS[d].dci = din;
    DEVS[d].kind = KIND_MSC;
    NDEVS = NDEVS.max(d + 1);
    MSC_D = d;
    MSC_SLOT = slot;
    MSC_PORT = port;
    MSC_IN = din;
    MSC_OUT = dout;
    MSC_EPI_ADDR = 0x80 | epi;
    MSC_EPO_ADDR = epo;
    READY = true;
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
        // (a failed command's sense data -- after power-on, UNIT ATTENTION -- is read by bot())
        crate::timer::busywait_ms(100);
    }
    // READ CAPACITY(10): last LBA and block length, big-endian
    if !ready || !bot(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], 8, true) {
        crate::kprintln!("[xhci] port {}: USB storage {:04x}:{:04x} did not report its capacity", port, vid, pid);
        return true; // kept (it is set up), but not used for storage
    }
    let b = &mscbuf();
    MSC_BLOCKS = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64 + 1;
    MSC_BS = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
    let usable = matches!(MSC_BS, 512 | 1024 | 2048 | 4096) && msc_io(0, false) && {
        let s0 = &mscbuf()[..512];
        &s0[..8] == b"AFRIVFS1" || s0.iter().all(|&x| x == 0)
    };
    MSC_USABLE = usable;
    let name = core::str::from_utf8(&*addr_of!(MSC_NAME)).unwrap_or("?");
    crate::kprintln!(
        "[xhci] port {}: USB storage {:04x}:{:04x} \"{}\", {} blocks of {} B ({} MiB), {}",
        port, vid, pid, name.trim(),
        { MSC_BLOCKS }, { MSC_BS }, MSC_BLOCKS * MSC_BS as u64 / (1024 * 1024),
        if usable { "blank or ours: used for storage" } else { "holds other data: left alone" }
    );
    true
}

/// Wait for the transfer event of endpoint `dci` of `slot`, serving every other event.
unsafe fn wait_xfer(slot: u32, dci: u32) -> Option<Trb> {
    for _ in 0..60_000_000u64 {
        if let Some(t) = next_event() {
            if (t.control >> 10) & 0x3F == T_EV_TRANSFER && t.control >> 24 == slot && (t.control >> 16) & 0x1F == dci {
                return Some(t);
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
    let was_busy = BUSY;
    BUSY = true; // the timer poll stands aside; wait_xfer serves the other devices
    let ok = bot_inner(cdb, len, dir_in);
    BUSY = was_busy;
    ok
}

unsafe fn bot_inner(cdb: &[u8], len: u32, dir_in: bool) -> bool {
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
        bot_reset(); // the device would not even take the command: start over
        return false;
    }
    // the data, if any; a STALL here is the device saying "no" -- clear it and read the status
    if len > 0 {
        let c = cc_of(msc_xfer(dir_in, addr_of!(MSCBUF) as u64, len));
        if c == Some(CC_STALL) {
            unwedge(dir_in);
        } else if !good(c) {
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
        bot_reset(); // phase error, or a status we cannot trust: Bulk-Only reset recovery
        return false;
    }
    if cs[12] != 0 && cdb[0] != 0x03 {
        // a failed command: read its sense data, or many sticks refuse the next one
        let saved = *mscbuf();
        bot_inner(&[0x03, 0, 0, 0, 18, 0], 18, true);
        *mscbuf() = saved;
    }
    cs[12] == 0
}

/// One bulk transfer on the stick's IN or OUT endpoint; its completion event.
unsafe fn msc_xfer(dir_in: bool, buf: u64, len: u32) -> Option<Trb> {
    let d = MSC_D;
    if dir_in {
        push(addr_of_mut!(EPIN[d].0) as *mut Trb, NXFER, &mut (*addr_of_mut!(PEPIN))[d], buf, len, T_NORMAL << 10 | 1 << 5 | 1 << 2);
        doorbell(MSC_SLOT, MSC_IN);
        wait_xfer(MSC_SLOT, MSC_IN)
    } else {
        push(addr_of_mut!(MSC_OUT_RING.0) as *mut Trb, NXFER, &mut *addr_of_mut!(PMSCOUT), buf, len, T_NORMAL << 10 | 1 << 5);
        doorbell(MSC_SLOT, MSC_OUT);
        wait_xfer(MSC_SLOT, MSC_OUT)
    }
}

/// A halted (stalled) endpoint: xHCI Reset Endpoint, CLEAR_FEATURE(ENDPOINT_HALT) to the
/// device, then move the controller's dequeue pointer past the stalled transfer to where the
/// next one will be queued (Set TR Dequeue Pointer).
unsafe fn unwedge(dir_in: bool) {
    let d = MSC_D;
    let (dci, addr) = if dir_in { (MSC_IN, MSC_EPI_ADDR) } else { (MSC_OUT, MSC_EPO_ADDR) };
    MSC_RECOVERIES += 1;
    command(0, T_RESET_EP << 10 | MSC_SLOT << 24 | dci << 16);
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

/// READ(10) / WRITE(10) of one device block at `lba` through MSCBUF.
unsafe fn msc_io(lba: u64, write: bool) -> bool {
    let l = (lba as u32).to_be_bytes();
    let cdb = [if write { 0x2A } else { 0x28 }, 0, l[0], l[1], l[2], l[3], 0, 0, 1, 0];
    bot(&cdb, MSC_BS, !write)
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

/// Capacity in 512-byte sectors.
pub fn msc_capacity() -> u64 {
    // SAFETY: scalar read.
    (unsafe { MSC_BLOCKS }) * msc_per()
}

pub fn msc_read(n: u64, out: &mut [u8; 512]) -> bool {
    if !msc_present() || n >= msc_capacity() {
        return false;
    }
    // SAFETY: mainline (the VFS); bot() keeps the timer poll off the event ring meanwhile.
    unsafe {
        let Some(home) = on(DEVS[MSC_D].ctl) else { return false };
        let ok = msc_io(n / msc_per(), false);
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

pub fn msc_write(n: u64, data: &[u8; 512]) -> bool {
    if !msc_present() || n >= msc_capacity() {
        return false;
    }
    // SAFETY: as msc_read.
    unsafe {
        let Some(home) = on(DEVS[MSC_D].ctl) else { return false };
        let ok = (msc_per() == 1 || msc_io(n / msc_per(), false)) && {
            let off = (n % msc_per()) as usize * 512;
            mscbuf()[off..off + 512].copy_from_slice(data);
            msc_io(n / msc_per(), true)
        };
        select(home);
        ok
    }
}

/// A USB Ethernet adapter: CDC-ECM (most USB Ethernet dongles and docks) or RNDIS (an Android
/// phone's "USB tethering", and older dongles made for Windows). Find a configuration with a
/// communication interface of either kind and a data interface (class 0x0A) with a bulk IN and
/// a bulk OUT endpoint (ECM: in an alternate setting >= 1; RNDIS: in setting 0). ECM is taken
/// when a device offers both, as Linux does (QEMU's usb-net offers both; the test rig asks for
/// RNDIS through fw_cfg `opt/afrisovera/usbnet-rndis`). ECM: the MAC is the string its Ethernet
/// functional descriptor names; SET_INTERFACE selects the data endpoints; the packet filter is
/// opened with SET_ETHERNET_PACKET_FILTER. RNDIS: INITIALIZE, a query of the permanent MAC and
/// a SET of the packet filter go as encapsulated control messages (RNDIS 1.0). Then both
/// endpoints are configured and receive buffers queued.
unsafe fn net_setup(d: usize, port: u32, speed: u32, ncfg: u8, vid: u16, pid: u16) -> bool {
    let slot = DEVS[d].slot;
    let want_rndis = crate::qemu::fw_cfg_file(b"opt/afrisovera/usbnet-rndis");
    for pass in 0..2 {
        let rndis = (pass == 0) == want_rndis;
        for ci in 0..ncfg.clamp(1, 4) {
            if !control(d, 0x80, 6, 0x0200 | ci as u16, 0, 9) {
                continue;
            }
            let total = (u16::from_le_bytes([DBUF.0[2], DBUF.0[3]]) as usize).min(512);
            let cval = DBUF.0[5];
            if !control(d, 0x80, 6, 0x0200 | ci as u16, 0, total as u16) {
                continue;
            }
            let (mut i, mut comm, mut mac_idx) = (0usize, None, 0u8);
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
                        let is_comm = if rndis {
                            matches!(cur_class, (2, 2, 0xFF) | (0xE0, 1, 3) | (0xEF, 4, 1))
                        } else {
                            (cur_class.0, cur_class.1) == (2, 6)
                        };
                        if is_comm && comm.is_none() {
                            comm = Some(cur_if);
                        }
                    }
                    // CS_INTERFACE, Ethernet Networking functional descriptor: iMACAddress
                    0x24 if i + 4 <= total && DBUF.0[i + 2] == 0x0F && comm == Some(cur_if) => mac_idx = DBUF.0[i + 3],
                    5 if i + 7 <= total && cur_class.0 == 0x0A && (if rndis { cur_alt == 0 } else { cur_alt >= 1 }) => {
                        let (addr, attr) = (DBUF.0[i + 2], DBUF.0[i + 3]);
                        let mps = u16::from_le_bytes([DBUF.0[i + 4], DBUF.0[i + 5]]) as u32 & 0x7FF;
                        if attr & 3 == 2 {
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
            NET_MPSO = mpso.max(8);
            for b in 0..NRXB {
                queue_rx(b);
            }
            // RNDIS passes data only once the filter is set: after the receive buffers are queued
            let filtered = !rndis || rndis_filter(d, comm_if);
            READY = true;
            crate::kprintln!(
                "[xhci] port {}: USB Ethernet adapter {:04x}:{:04x} ({}, {} speed), slot {}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                port, vid, pid, if rndis { "RNDIS" } else { "CDC-ECM" },
                match speed { 1 => "full", 2 => "low", 3 => "high", 4 => "super", _ => "?" }, slot,
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
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
    let at = push_held(
        addr_of_mut!(EPIN[d].0) as *mut Trb,
        NXFER,
        &mut (*addr_of_mut!(PEPIN))[d],
        addr_of!(NETRX[b]) as u64,
        FRAME as u32,
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

/// One received Ethernet frame into the queue for the network stack (dropped if full).
unsafe fn rxq_put(src: *const u8, n: usize) {
    let next = (RXQ_TAIL + 1) % NRXQ;
    if (14..=FRAME).contains(&n) && next != RXQ_HEAD {
        core::ptr::copy_nonoverlapping(src, addr_of_mut!(RXQ[RXQ_TAIL]) as *mut u8, n);
        RXQLEN[RXQ_TAIL] = n;
        core::sync::atomic::fence(Ordering::SeqCst);
        RXQ_TAIL = next;
        NET_RXN += 1;
    } else {
        NET_RXDROP += 1;
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
        port, vid, pid, chip, match speed { 1 => "full", 2 => "low", 3 => "high", 4 => "super", _ => "?" }, slot, epi, epo
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
#[repr(C, align(64))]
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
            crate::kprintln!("[xhci] port {}: USB audio {:04x}:{:04x} is Audio Class 2.0 -- not driven yet", port, vid, pid);
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
        port, vid, pid, match speed { 1 => "full", 2 => "low", 3 => "high", 4 => "super", _ => "?" }, slot,
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
            crate::kprintln!("[audio] done — played by the USB headset or speaker");
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

/// A click on the desktop's dock (tonga.rs: five tiles across 24..616, y 408..466) runs
/// that tile's command, as if it were typed: back to the text console, then the command.
unsafe fn dock_click(x: i32, y: i32) {
    if !(408..466).contains(&y) || !(24..616).contains(&x) {
        return;
    }
    let cmd: &[u8] = match (x - 24) / 118 {
        0 => b"",
        1 => b"ls",
        2 => b"store",
        3 => b"ask who are you",
        _ => b"devices",
    };
    crate::screen::set_desktop(false);
    for &c in b"clear\r".iter().chain(cmd.iter()) {
        crate::uart::inject(c);
    }
    if !cmd.is_empty() {
        crate::uart::inject(b'\r');
    }
    DOCK_CLICKS += 1;
}

/// An absolute pointer's report: the pointer goes to the place it names (buttons as they
/// were, so a held button drags), then the buttons change there -- a tap lands where touched.
unsafe fn tablet_report(r: &[u8]) {
    let a = TAB;
    if a.id != 0 && r.first() != Some(&a.id) {
        return; // another of the device's reports
    }
    if (r.len() as u32) * 8 < (a.x.bit + a.x.size).max(a.y.bit + a.y.size) {
        return;
    }
    TREPORTS += 1;
    let (w, h) = (crate::framebuffer::WIDTH as i32, crate::framebuffer::HEIGHT as i32);
    let (x, y) = (scale(r, a.x, w), scale(r, a.y, h));
    let mut buttons = 0u8;
    for k in 0..a.nbtn.min(3) {
        buttons |= (bits(r, a.btn + k, 1) as u8) << k;
    }
    mouse_report(MBUTTONS, x - MX, y - MY);
    mouse_report(buttons, 0, 0);
}

/// One pointer report: buttons (bit 0 left, 1 right, 2 middle), then signed x and y moves.
unsafe fn mouse_report(buttons: u8, dx: i32, dy: i32) {
    MCLICKS += (buttons & !MBUTTONS & 7).count_ones() as u64;
    let left_down = buttons & 1 != 0 && MBUTTONS & 1 == 0;
    MBUTTONS = buttons & 7;
    // the window manager: a left press on a title bar picks the window up, moves carry it,
    // the release puts it down
    if left_down && crate::screen::desktop() {
        dock_click(MX, MY);
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
    let (w, h) = (crate::framebuffer::WIDTH as i32, crate::framebuffer::HEIGHT as i32);
    let (x, y) = ((MX + dx).clamp(0, w - 1), (MY + dy).clamp(0, h - 1));
    if (x, y) != (MX, MY) {
        if let Some(id) = DRAG {
            crate::tonga::win_move(id, x - MX, y - MY);
            crate::screen::wm_moved();
        }
        MX = x;
        MY = y;
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
            if MSC_RECOVERIES > 0 {
                crate::kprintln!("  storage: {} stall/phase recoveries", { MSC_RECOVERIES });
            }
        }
        if PLUGS + UNPLUGS > 0 {
            crate::kprintln!("  hot-plug: {} plugged in, {} unplugged since boot", { PLUGS }, { UNPLUGS });
        }
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
        }
        if MOUSE_SLOT != 0 {
            crate::kprintln!(
                "  mouse: slot {}, {} reports, pointer at {},{}, buttons {:03b}, {} clicks",
                { MOUSE_SLOT }, { MREPORTS }, { MX }, { MY }, { MBUTTONS }, { MCLICKS }
            );
            crate::kprintln!("  mouse: {} window drag(s)", { DRAGS });
            if DOCK_CLICKS > 0 {
                crate::kprintln!("  mouse: {} dock click(s) on the desktop", { DOCK_CLICKS });
            }
        }
        if TABLET_SLOT != 0 {
            crate::kprintln!(
                "  tablet: slot {}, {} reports, pointer at {},{}, buttons {:03b}, {} clicks",
                { TABLET_SLOT }, { TREPORTS }, { MX }, { MY }, { MBUTTONS }, { MCLICKS }
            );
            crate::kprintln!("  tablet: {} window drag(s), {} dock click(s)", { DRAGS }, { DOCK_CLICKS });
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
