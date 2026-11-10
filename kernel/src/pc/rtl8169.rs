//! Realtek wired Ethernet on the PC: roadmap PC step 3, the network (7 Nov 2026).
//!
//! The RTL8111/8168 is the wired chip on most consumer desktop boards and laptops (the
//! RTL8101/8102/8103 is its 100 Mbit sibling, the RTL8169/8110 the older PCI one): Linux
//! drives them all with r8169. They share the "C+" descriptor format of the RTL8139C+ (Linux
//! 8139cp), which is what QEMU's `rtl8139` card is -- so one ring layout is proved on QEMU and
//! the real chips differ only in a few register offsets (marked below).
//!
//! 16-byte descriptors (opts1, opts2, buffer address) in two rings, polled, as e1000.rs does:
//! `send` puts one frame on the wire, `recv` takes one off. The shared TCP/IP stack never sees
//! this file -- pc/nic.rs and pc/virtio_pci.rs present the card to it as a virtio-net device.
//! Sources: Realtek RTL8139C+ and RTL8169 datasheets (descriptor formats), Linux 8139cp.c and
//! r8169_main.c (register names and the start-up order), QEMU hw/net/rtl8139.c (C+ mode).
//!
//! Not done (honest): the extra start-up settings Linux carries per RTL8168 revision (EPHY and
//! OCP tables) and its PHY firmware patches; the firmware's own PHY set-up is trusted. The
//! RTL8125 (2.5 GbE) moves the poll and interrupt registers and is not claimed.

use crate::mmu;
use crate::pci;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

/// The parts this driver knows, by (vendor, device); wifi.rs names them too. The 8139 counts
/// only from revision 0x20 (the C+ chip; older 8139s have no descriptor rings).
pub(crate) const PARTS: &[(u16, u16, &str)] = &[
    (0x10EC, 0x8139, "RTL8139C+ (8139cp)"),
    (0x10EC, 0x8168, "RTL8111/8168 (r8169)"),
    (0x10EC, 0x8161, "RTL8111/8168 (r8169)"),
    (0x10EC, 0x8136, "RTL8101/8102/8103 (r8169)"),
    (0x10EC, 0x8169, "RTL8169/8110 (r8169)"),
    (0x10EC, 0x8167, "RTL8169SC/8110SC (r8169)"),
    (0x1186, 0x4300, "D-Link DGE-528T, RTL8169 (r8169)"),
    (0x1186, 0x4302, "D-Link DGE-530T, RTL8169 (r8169)"),
];

// shared by the 8139C+ and the r8169 family
const IDR0: u64 = 0x00; // MAC address, loaded from the EEPROM at power-on
const TX_DESC_LO: u64 = 0x20; // 8139C+: TxAddr0/1 hold the ring address in C+ mode
const TX_DESC_HI: u64 = 0x24;
const CHIP_CMD: u64 = 0x37;
const INTR_MASK: u64 = 0x3C;
const INTR_STATUS: u64 = 0x3E;
const TX_CONFIG: u64 = 0x40;
const RX_CONFIG: u64 = 0x44;
const CFG9346: u64 = 0x50;
const CPLUS_CMD: u64 = 0xE0;
const RX_DESC_LO: u64 = 0xE4;
const RX_DESC_HI: u64 = 0xE8;
// where the two differ
const TX_POLL_8139: u64 = 0xD9;
const TX_POLL_8169: u64 = 0x38;
const MEDIA_STATUS_8139: u64 = 0x58; // bit 2: link FAILED
const HLT_CLK_8139: u64 = 0x5B; // 'R' runs the chip's clock
const PHY_STATUS_8169: u64 = 0x6C; // bit 1: link up
const RX_MAX_SIZE_8169: u64 = 0xDA;

const CMD_RESET: u8 = 1 << 4;
const CMD_RX_EN: u8 = 1 << 3;
const CMD_TX_EN: u8 = 1 << 2;
const CPLUS_RX_EN: u16 = 1 << 1; // 8139C+ only: descriptor-ring mode
const CPLUS_TX_EN: u16 = 1 << 0;
const POLL_NORMAL: u8 = 1 << 6; // NPQ: normal-priority queue has work
// RX: my MAC, broadcast; no FIFO threshold (7), unlimited DMA burst (7)
const RXCFG: u32 = 7 << 13 | 7 << 8 | 1 << 3 | 1 << 1;
// TX: standard inter-frame gap (3), unlimited DMA burst (7)
const TXCFG: u32 = 3 << 24 | 7 << 8;

const OWN: u32 = 1 << 31;
const EOR: u32 = 1 << 30; // end of ring: the chip goes back to descriptor 0
const FS: u32 = 1 << 29;
const LS: u32 = 1 << 28;

const NRX: usize = 32;
const NTX: usize = 8;
const BUF: usize = 2048;
const MIN_FRAME: usize = 60; // padded here: several RTL8168 revisions do not pad short frames

#[derive(Clone, Copy)]
#[repr(C)]
struct Desc {
    opts1: u32,
    opts2: u32,
    addr: u64,
}
#[repr(C, align(256))]
struct RxRing([Desc; NRX]);
#[repr(C, align(256))]
struct TxRing([Desc; NTX]);

const D0: Desc = Desc { opts1: 0, opts2: 0, addr: 0 };
static mut RXR: RxRing = RxRing([D0; NRX]);
static mut TXR: TxRing = TxRing([D0; NTX]);
/// Packet buffers, each BUF (2 KiB) aligned so none crosses a 4 KiB page: the r8169 family wants 8-byte
/// aligned buffers and the 82545/82546 must not cross a 64 KiB line (Linux e1000_check_64k_bound).
#[repr(C, align(2048))]
struct Bufs<const N: usize>([[u8; BUF]; N]);
static mut RXB: Bufs<NRX> = Bufs([[0; BUF]; NRX]);
static mut TXB: Bufs<NTX> = Bufs([[0; BUF]; NTX]);

static mut BAR: u64 = 0;
/// The RTL8139C+ (QEMU's card) rather than the r8169 family.
static mut IS_8139: bool = false;
static mut RX_NEXT: usize = 0;
static mut TX_NEXT: usize = 0;
static mut MAC: [u8; 6] = [0; 6];
static mut NAME: &str = "";
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut SENT: u64 = 0;
static mut RECEIVED: u64 = 0;
/// Inside a frame spread over several descriptors (FS without LS): dropped up to its LS.
static mut DROPPING: bool = false;

fn r8(off: u64) -> u8 {
    // SAFETY: the register BAR is mapped uncached by init before any register access.
    unsafe { read_volatile((BAR + off) as *const u8) }
}
fn w8(off: u64, v: u8) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u8, v) }
}
fn w16(off: u64, v: u16) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u16, v) }
}
fn r16(off: u64) -> u16 {
    // SAFETY: as r8.
    unsafe { read_volatile((BAR + off) as *const u16) }
}
fn w32(off: u64, v: u32) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u32, v) }
}

fn wait(mut done: impl FnMut() -> bool) -> bool {
    for _ in 0..20_000_000u64 {
        if done() {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// The first memory BAR (the registers): BAR1 on the 8139/8169, BAR2 (64-bit) on PCIe 8168s.
fn mem_bar(b: u8, d: u8, f: u8) -> u64 {
    let mut i = 0u8;
    while i < 6 {
        let v = pci::cfg_read32(b, d, f, 0x10 + i * 4);
        if v & 1 == 0 {
            let mut a = (v & !0xF) as u64;
            if v & 0x6 == 0x4 && i < 5 {
                a |= (pci::cfg_read32(b, d, f, 0x14 + i * 4) as u64) << 32;
            }
            if a != 0 {
                return a;
            }
            if v & 0x6 == 0x4 {
                i += 1; // the upper half of an empty 64-bit BAR
            }
        }
        i += 1;
    }
    0
}

fn link() -> bool {
    // SAFETY: scalar read of boot-time state.
    if unsafe { IS_8139 } {
        r8(MEDIA_STATUS_8139) & 1 << 2 == 0
    } else {
        r8(PHY_STATUS_8169) & 1 << 1 != 0
    }
}

/// Find a known Realtek NIC, reset it and bring up its RX and TX rings. True when ready.
pub fn init() -> bool {
    let mut name = "";
    let hit = |b, d, f, id: u32| {
        let (v, dv) = ((id & 0xFFFF) as u16, (id >> 16) as u16);
        match PARTS.iter().find(|p| p.0 == v && p.1 == dv) {
            // an 8139 before the C+ (revision < 0x20) has no descriptor rings
            Some(_) if dv == 0x8139 && pci::cfg_read32(b, d, f, 0x08) & 0xFF < 0x20 => false,
            Some(&(_, _, n)) => {
                name = n;
                true
            }
            None => false,
        }
    };
    let Some((b, d, f)) = pci::find_by(8, hit) else { return false };
    let bar = mem_bar(b, d, f);
    if bar == 0 || !mmu::map_mmio(bar, 0x1000) {
        crate::kprintln!("[rtl] {} at {:02x}:{:02x}.{} has no usable register BAR", name, b, d, f);
        return false;
    }
    pci::enable(b, d, f, 0x406); // memory decoding + bus mastering, INTx off (polled)
    let is8139 = (pci::cfg_read32(b, d, f, 0) >> 16) as u16 == 0x8139;
    // SAFETY: boot, single core, before any reader.
    unsafe {
        BAR = bar;
        NAME = name;
        PCI_AT = (b, d, f);
        IS_8139 = is8139;
    }
    // Every interrupt masked (this driver polls), then a software reset.
    w16(INTR_MASK, 0);
    w8(CHIP_CMD, CMD_RESET);
    if !wait(|| r8(CHIP_CMD) & CMD_RESET == 0) {
        crate::kprintln!("[rtl] {} did not come out of reset", name);
        return false;
    }
    w16(INTR_MASK, 0);
    w16(INTR_STATUS, 0xFFFF);
    if is8139 {
        w8(HLT_CLK_8139, b'R'); // the chip's clock running (the 8139 can halt it)
    }
    let mut mac = [0u8; 6];
    for (i, m) in mac.iter_mut().enumerate() {
        *m = r8(IDR0 + i as u64);
    }
    if mac == [0; 6] || mac == [0xFF; 6] {
        crate::kprintln!("[rtl] {}: no MAC address (the EEPROM left IDR0-5 empty)", name);
        return false;
    }
    // SAFETY: the rings and buffers are ours alone until CHIP_CMD enables the engines.
    unsafe {
        MAC = mac;
        let rx = &mut (*addr_of_mut!(RXR)).0;
        for (i, dsc) in rx.iter_mut().enumerate() {
            let end = if i == NRX - 1 { EOR } else { 0 };
            *dsc = Desc { opts1: OWN | end | BUF as u32, opts2: 0, addr: addr_of!(RXB.0[i]) as u64 };
        }
        let tx = &mut (*addr_of_mut!(TXR)).0;
        for (i, dsc) in tx.iter_mut().enumerate() {
            let end = if i == NTX - 1 { EOR } else { 0 };
            *dsc = Desc { opts1: end, opts2: 0, addr: addr_of!(TXB.0[i]) as u64 };
        }
        RX_NEXT = 0;
        TX_NEXT = 0;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    let (rxa, txa) = (addr_of!(RXR) as u64, addr_of!(TXR) as u64);
    // Linux's order: configuration unlocked, C+ command, ring addresses (high half first),
    // locked again, then RX/TX on, then the RX/TX configuration (some 8169s drop it before)
    w8(CFG9346, 0xC0);
    if is8139 {
        w16(CPLUS_CMD, r16(CPLUS_CMD) | CPLUS_RX_EN | CPLUS_TX_EN);
    } else {
        w16(RX_MAX_SIZE_8169, BUF as u16);
    }
    w32(TX_DESC_HI, (txa >> 32) as u32);
    w32(TX_DESC_LO, txa as u32);
    w32(RX_DESC_HI, (rxa >> 32) as u32);
    w32(RX_DESC_LO, rxa as u32);
    w32(RX_CONFIG, RXCFG);
    w32(TX_CONFIG, TXCFG);
    w8(CFG9346, 0x00);
    w8(CHIP_CMD, CMD_RX_EN | CMD_TX_EN);
    w32(RX_CONFIG, RXCFG);
    w32(TX_CONFIG, TXCFG);
    if r8(CHIP_CMD) & (CMD_RX_EN | CMD_TX_EN) != CMD_RX_EN | CMD_TX_EN {
        crate::kprintln!("[rtl] {}: receiver and transmitter did not switch on", name);
        return false;
    }
    // a real PHY needs a few seconds of auto-negotiation before the link is up
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while !link() && crate::timer::cycles() - t0 < 5 * hz {
        core::hint::spin_loop();
    }
    crate::kprintln!(
        "[rtl] {} at {:02x}:{:02x}.{}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, link {}",
        name, b, d, f, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        if link() { "up" } else { "down" }
    );
    true
}

pub fn mac() -> [u8; 6] {
    // SAFETY: scalar read of boot-time state.
    unsafe { MAC }
}

pub fn link_up() -> bool {
    present() && link()
}

/// Put one Ethernet frame (no FCS: the NIC appends it) on the wire. False if the ring
/// stays full.
pub fn send(frame: &[u8]) -> bool {
    // SAFETY: single core; the descriptor at TX_NEXT is ours once the NIC cleared OWN on it.
    unsafe {
        if BAR == 0 || frame.len() > BUF {
            return false;
        }
        let i = TX_NEXT;
        let dsc = addr_of_mut!(TXR.0[i]);
        if !wait(|| read_volatile(addr_of!((*dsc).opts1)) & OWN == 0) {
            crate::kprintln!("[rtl] transmit ring stuck");
            return false;
        }
        let buf = addr_of_mut!(TXB.0[i]) as *mut u8;
        core::ptr::copy_nonoverlapping(frame.as_ptr(), buf, frame.len());
        let n = frame.len().max(MIN_FRAME);
        if n > frame.len() {
            core::ptr::write_bytes(buf.add(frame.len()), 0, n - frame.len());
        }
        let end = if i == NTX - 1 { EOR } else { 0 };
        write_volatile(addr_of_mut!((*dsc).addr), addr_of!(TXB.0[i]) as u64);
        write_volatile(addr_of_mut!((*dsc).opts2), 0);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_volatile(addr_of_mut!((*dsc).opts1), OWN | end | FS | LS | n as u32);
        TX_NEXT = (i + 1) % NTX;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w8(if IS_8139 { TX_POLL_8139 } else { TX_POLL_8169 }, POLL_NORMAL);
        SENT += 1;
        true
    }
}

/// Take the next received frame, copying it into `dst`; its length, or None if none waits.
pub fn recv(dst: &mut [u8]) -> Option<usize> {
    // SAFETY: single core; a descriptor with OWN clear is ours until we set OWN again.
    unsafe {
        if BAR == 0 {
            return None;
        }
        let i = RX_NEXT;
        let dsc = addr_of_mut!(RXR.0[i]);
        let st = read_volatile(addr_of!((*dsc).opts1));
        if st & OWN != 0 {
            return None;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        // the length counts the 4-byte FCS the chip also wrote; the error summary bit is 20
        // on the 8139C+ and 21 (RES) on the r8169 family
        let (mask, err) = if IS_8139 { (0x1FFF, 1u32 << 20) } else { (0x3FFF, 1u32 << 21) };
        let len = ((st & mask) as usize).saturating_sub(4);
        let whole = st & FS != 0 && st & LS != 0 && st & err == 0 && !DROPPING && len > 0;
        // a first piece without its last starts dropping until the piece that has LS
        DROPPING = st & LS == 0;
        let n = len.min(dst.len());
        if whole {
            core::ptr::copy_nonoverlapping(addr_of!(RXB.0[i]) as *const u8, dst.as_mut_ptr(), n);
        }
        let end = if i == NRX - 1 { EOR } else { 0 };
        write_volatile(addr_of_mut!((*dsc).addr), addr_of!(RXB.0[i]) as u64);
        write_volatile(addr_of_mut!((*dsc).opts2), 0);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        write_volatile(addr_of_mut!((*dsc).opts1), OWN | end | BUF as u32);
        RX_NEXT = (i + 1) % NRX;
        if whole {
            RECEIVED += 1;
            Some(n)
        } else {
            Some(0) // a bad or split frame, dropped; the caller asks again
        }
    }
}

/// True if a received frame is waiting.
pub fn rx_ready() -> bool {
    // SAFETY: a volatile read of one descriptor's opts1.
    unsafe { BAR != 0 && read_volatile(addr_of!(RXR.0[RX_NEXT].opts1)) & OWN == 0 }
}

/// `lsrtl`: the NIC, its link and its counters.
pub fn list() {
    // SAFETY: scalar reads.
    let (bar, name, (b, d, f), m, s, rcv) = unsafe { (BAR, NAME, *addr_of!(PCI_AT), MAC, SENT, RECEIVED) };
    if bar == 0 {
        crate::kprintln!("rtl: no Realtek wired NIC (RTL8111/8168, RTL8101, RTL8169, RTL8139C+)");
        return;
    }
    crate::kprintln!(
        "rtl: {} at {:02x}:{:02x}.{} registers {:#x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, link {}",
        name, b, d, f, bar, m[0], m[1], m[2], m[3], m[4], m[5], if link_up() { "up" } else { "down" }
    );
    crate::kprintln!("  frames sent {}, received {}", s, rcv);
    dma_line();
}

/// Every ring and packet buffer against the card's DMA limits: rings 256-byte aligned, buffers 8-byte aligned
/// inside one 4 KiB page. Returns (outside, total, first outside).
fn dma_check() -> (usize, usize, &'static str) {
    let mut bad = (0usize, "");
    let mut see = |name: &'static str, a: u64, len: u64, align: u64| {
        let ok = a % align == 0 && (if align == 8 { a >> 12 == (a + len - 1) >> 12 } else { true });
        if !ok {
            if bad.0 == 0 {
                bad.1 = name;
            }
            bad.0 += 1;
        }
    };
    // SAFETY: addresses only, nothing read or written.
    unsafe {
        see("the receive ring", addr_of!(RXR) as u64, core::mem::size_of::<RxRing>() as u64, 256);
        see("the transmit ring", addr_of!(TXR) as u64, core::mem::size_of::<TxRing>() as u64, 256);
        for i in 0..NRX {
            see("a receive buffer", addr_of!(RXB.0[i]) as u64, BUF as u64, 8);
        }
        for i in 0..NTX {
            see("a transmit buffer", addr_of!(TXB.0[i]) as u64, BUF as u64, 8);
        }
    }
    (bad.0, 2 + NRX + NTX, bad.1)
}

/// The memory line of the `lsrtl` listing.
fn dma_line() {
    let (bad, total, first) = dma_check();
    if bad == 0 {
        crate::kprintln!(
            "  memory: all {} rings and buffers inside their limits (rings 256-byte aligned, buffers 8-byte aligned in one 4 KiB page)",
            total
        );
    } else {
        crate::kprintln!("  memory: {} of {} rings and buffers outside their limits (first: {})", bad, total, first);
    }
}

/// True once init found and set up a NIC.
pub fn present() -> bool {
    // SAFETY: scalar read.
    unsafe { BAR != 0 }
}
