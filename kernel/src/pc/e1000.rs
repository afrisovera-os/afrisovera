//! Intel e1000 / e1000e wired Ethernet on the PC: roadmap PC step 3, the network (5 Nov 2026).
//!
//! The common wired NIC of PCs and laptops (82540EM in QEMU's `e1000`, 82574L in `e1000e`,
//! and the same register set on many real Intel parts). Legacy descriptors, one RX ring and
//! one TX ring, polled: `send` puts one frame on the wire, `recv` takes one off. The shared
//! TCP/IP stack never sees this file -- pc/virtio_pci.rs presents this NIC to it as a
//! virtio-net device, so `ping`, DHCP, DNS and TCP run on it unchanged.
//! Intel 8254x/82574 software developer's manuals: registers 13, descriptors 3.2/3.3.
//!
//! The newer Intel family (7 Nov 2026) shares the file: 82576/82580/I350 and the I210/I211 on
//! most desktop boards from 2013 (Linux igb), and the 2.5 GbE I225/I226 on boards since 2019
//! (Linux igc). Same CTRL/STATUS/RCTL/TCTL/RAL/MTA registers, but queue 0's rings sit at
//! 0xC000 (RX) / 0xE000 (TX), each queue must be switched on in RXDCTL/TXDCTL, and this
//! driver uses ADVANCED descriptors there (the one format igc has): 82576 datasheet 7.1.5,
//! 7.2.2, 8.10; QEMU's `igb` is the 82576.

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

/// The parts this driver knows, by PCI device id (vendor 0x8086); wifi.rs names them too.
pub(crate) const PARTS: &[(u16, &str)] = &[
    (0x100E, "82540EM (e1000)"),
    (0x100F, "82545EM (e1000)"),
    (0x10D3, "82574L (e1000e)"),
    (0x10F5, "82567LM (e1000e)"),
    (0x1502, "82579LM (e1000e)"),
    (0x153A, "I217-LM (e1000e)"),
    (0x15A3, "I218-V (e1000e)"),
    // I219 (2015 onwards: most Intel laptops and desktops); LM and V variants by chipset
    (0x156F, "I219-LM (e1000e)"),
    (0x1570, "I219-V (e1000e)"),
    (0x15B7, "I219-LM (e1000e)"),
    (0x15B8, "I219-V (e1000e)"),
    (0x15BB, "I219-LM (e1000e)"),
    (0x15BC, "I219-V (e1000e)"),
    (0x15BD, "I219-LM (e1000e)"),
    (0x15BE, "I219-V (e1000e)"),
    (0x15D7, "I219-LM (e1000e)"),
    (0x15D8, "I219-V (e1000e)"),
    (0x15E3, "I219-LM (e1000e)"),
    (0x15D6, "I219-V (e1000e)"),
    (0x0D4E, "I219-LM (e1000e)"),
    (0x0D4F, "I219-V (e1000e)"),
    (0x0D4C, "I219-LM (e1000e)"),
    (0x0D4D, "I219-V (e1000e)"),
    (0x0D53, "I219-LM (e1000e)"),
    (0x0D55, "I219-V (e1000e)"),
    (0x15FB, "I219-LM (e1000e)"),
    (0x15FC, "I219-V (e1000e)"),
    // the igb/igc family: advanced descriptors, rings at 0xC000/0xE000 (see the top of the file)
    (0x10C9, "82576 (igb)"),
    (0x150E, "82580 (igb)"),
    (0x1521, "I350 (igb)"),
    (0x1533, "I210 (igb)"),
    (0x1536, "I210 (igb)"),
    (0x157B, "I210 (igb)"),
    (0x1539, "I211 (igb)"),
    (0x15F2, "I225-LM (igc)"),
    (0x15F3, "I225-V (igc)"),
    (0x15F8, "I225-I (igc)"),
    (0x125B, "I226-LM (igc)"),
    (0x125C, "I226-V (igc)"),
    (0x125D, "I226-IT (igc)"),
];

/// True for a part of the igb/igc family (advanced descriptors, queue registers).
fn advanced(name: &str) -> bool {
    name.ends_with("(igb)") || name.ends_with("(igc)")
}

const CTRL: u64 = 0x0000;
const STATUS: u64 = 0x0008;
const IMC: u64 = 0x00D8;
const RCTL: u64 = 0x0100;
const TCTL: u64 = 0x0400;
const TIPG: u64 = 0x0410;
const RDBAL: u64 = 0x2800;
const RDBAH: u64 = 0x2804;
const RDLEN: u64 = 0x2808;
const RDH: u64 = 0x2810;
const RDT: u64 = 0x2818;
const TDBAL: u64 = 0x3800;
const TDBAH: u64 = 0x3804;
const TDLEN: u64 = 0x3808;
const TDH: u64 = 0x3810;
const TDT: u64 = 0x3818;
const MTA: u64 = 0x5200;
const EERD: u64 = 0x0014;
const RAL0: u64 = 0x5400;
const RAH0: u64 = 0x5404;
// igb/igc only
const MDIC: u64 = 0x0020;
const EIMC: u64 = 0x1528;
const SRRCTL: u64 = 0xC00C;
const RXDCTL: u64 = 0xC028;
const TXDCTL: u64 = 0xE028;
const QUEUE_ENABLE: u32 = 1 << 25;
const SRRCTL_ADV_ONEBUF: u32 = 1 << 25; // DESCTYPE 001: advanced, one buffer
const ADV_DTYP_DATA: u64 = 0b0011 << 20;
const ADV_DEXT: u64 = 1 << 29;
const RXE: u32 = 1 << 31; // advanced RX write-back: frame error (CRC, symbol, sequence, length)

const CTRL_SLU: u32 = 1 << 6;
const CTRL_ASDE: u32 = 1 << 5;
const CTRL_RST: u32 = 1 << 26;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const DD: u8 = 1 << 0; // descriptor done
const EOP: u8 = 1 << 1; // end of packet (RX status) / (TX cmd bit 0)
const CMD_EOP: u8 = 1 << 0;
const CMD_IFCS: u8 = 1 << 1;
const CMD_RS: u8 = 1 << 3;

const NRX: usize = 32;
const NTX: usize = 8;
const BUF: usize = 2048;

#[derive(Clone, Copy)]
#[repr(C)]
struct RxDesc {
    addr: u64,
    len: u16,
    csum: u16,
    status: u8,
    errors: u8,
    special: u16,
}
#[derive(Clone, Copy)]
#[repr(C)]
struct TxDesc {
    addr: u64,
    len: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}
#[repr(C, align(128))]
struct RxRing([RxDesc; NRX]);
#[repr(C, align(128))]
struct TxRing([TxDesc; NTX]);

const RX0: RxDesc = RxDesc { addr: 0, len: 0, csum: 0, status: 0, errors: 0, special: 0 };
const TX0: TxDesc = TxDesc { addr: 0, len: 0, cso: 0, cmd: 0, status: 0, css: 0, special: 0 };
static mut RXR: RxRing = RxRing([RX0; NRX]);
static mut TXR: TxRing = TxRing([TX0; NTX]);
static mut RXB: [[u8; BUF]; NRX] = [[0; BUF]; NRX];
static mut TXB: [[u8; BUF]; NTX] = [[0; BUF]; NTX];

static mut BAR: u64 = 0;
/// An igb/igc part: advanced descriptors and the 0xC000/0xE000 queue registers.
static mut ADV: bool = false;
static mut RX_NEXT: usize = 0;
static mut TX_NEXT: usize = 0;
static mut MAC: [u8; 6] = [0; 6];
static mut NAME: &str = "";
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut SENT: u64 = 0;
static mut RECEIVED: u64 = 0;
/// Inside a frame that arrived in several descriptors (no EOP yet) or with errors: every
/// piece up to and including its EOP descriptor is dropped, so its tail is never taken for
/// a frame of its own (Gemini D3).
static mut DROPPING: bool = false;

/// Queue 0's ring registers: 0x2800/0x3800 on e1000/e1000e, 0xC000/0xE000 on igb/igc.
fn q(off: u64) -> u64 {
    // SAFETY: scalar read of boot-time state.
    let adv = unsafe { ADV };
    match off {
        0x2800..=0x28FF if adv => off - 0x2800 + 0xC000,
        0x3800..=0x38FF if adv => off - 0x3800 + 0xE000,
        _ => off,
    }
}
fn r(off: u64) -> u32 {
    // SAFETY: BAR0 is mapped uncached by init before any register access.
    unsafe { read_volatile((BAR + q(off)) as *const u32) }
}
fn w(off: u64, v: u32) {
    // SAFETY: as r.
    unsafe { write_volatile((BAR + q(off)) as *mut u32, v) }
}

/// RX descriptor i's state in either format (advanced write-back: status and errors in the
/// low 32 bits of the second quadword, the length in the next 16; legacy: the length at
/// byte 8, status at 12, errors at 13). (DD, EOP, bad, length)
///
/// SAFETY: the caller is the only reader of the RX ring (single core).
unsafe fn rx_state(i: usize) -> (bool, bool, bool, usize) {
    let dsc = addr_of!(RXR.0[i]);
    if ADV {
        let q1 = read_volatile((dsc as *const u64).add(1));
        let st = q1 as u32;
        (st & 1 != 0, st & 2 != 0, st & RXE != 0, (q1 >> 32) as u16 as usize)
    } else {
        let st = read_volatile(addr_of!((*dsc).status));
        let bad = read_volatile(addr_of!((*dsc).errors)) != 0;
        (st & DD != 0, st & EOP != 0, bad, read_volatile(addr_of!((*dsc).len)) as usize)
    }
}

/// igb/igc: switch the copper PHY's power on (register 0 bit 11, Power Down, over MDIC to
/// PHY address 1), as Linux igb_power_up_phy_copper does -- a firmware that never used the
/// port may leave it off -- and restart auto-negotiation (bit 12 on, bit 9 restart), as
/// igb_copper_link_autoneg does: these parts do not report a link (STATUS.LU) until it has
/// run after a reset (QEMU's igb likewise). Silent when MDIC does not answer.
fn phy_power_up() {
    let mdic = |op: u32, reg: u32, data: u32| -> Option<u32> {
        w(MDIC, data & 0xFFFF | reg << 16 | 1 << 21 | op << 26);
        if !wait(|| r(MDIC) & 1 << 28 != 0) || r(MDIC) & 1 << 30 != 0 {
            return None;
        }
        Some(r(MDIC) & 0xFFFF)
    };
    if let Some(bmcr) = mdic(2, 0, 0) {
        let _ = mdic(1, 0, bmcr & !(1 << 11) | 1 << 12 | 1 << 9);
    }
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

/// Find a known Intel NIC, reset it and bring up its RX and TX rings. True when ready.
pub fn init() -> bool {
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
                if id & 0xFFFF == 0x8086 {
                    if let Some(&(_, name)) = PARTS.iter().find(|p| p.0 as u32 == id >> 16) {
                        at = Some((b, d, f, name));
                        break 'scan;
                    }
                }
            }
        }
    }
    let Some((b, d, f, name)) = at else { return false };
    let bar0 = cfg32(b, d, f, 0x10);
    let mut bar = (bar0 & !0xF) as u64;
    if bar0 & 0x6 == 0x4 {
        bar |= (cfg32(b, d, f, 0x14) as u64) << 32; // a 64-bit BAR (igb/igc parts)
    }
    if bar == 0 || !mmu::map_mmio(bar, 0x20000) {
        crate::kprintln!("[e1000] {} at {:02x}:{:02x}.{} has no usable BAR0", name, b, d, f);
        return false;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory decoding + bus mastering, INTx off (polled)
    // SAFETY: boot, single core, before any reader.
    unsafe {
        BAR = bar;
        NAME = name;
        PCI_AT = (b, d, f);
        ADV = advanced(name);
    }
    let adv = advanced(name);
    // Reset, with every interrupt masked (this driver polls), then link up.
    w(IMC, 0xFFFF_FFFF);
    w(CTRL, r(CTRL) | CTRL_RST);
    if !wait(|| r(CTRL) & CTRL_RST == 0) {
        crate::kprintln!("[e1000] {} did not come out of reset", name);
        return false;
    }
    w(IMC, 0xFFFF_FFFF);
    if adv {
        // the NVM/iNVM is read back after a reset (igb waits ~10 ms); extended interrupts off too
        let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
        while crate::timer::cycles() - t0 < hz / 50 {
            core::hint::spin_loop();
        }
        w(EIMC, 0xFFFF_FFFF);
        phy_power_up();
    }
    w(CTRL, r(CTRL) | CTRL_SLU | CTRL_ASDE);
    for i in 0..128 {
        w(MTA + i * 4, 0);
    }
    // The MAC the hardware loaded into receive-address register 0 at reset; if that is
    // empty, words 0-2 of the EEPROM through EERD (82540/82545: address << 8, done bit 4;
    // 82574: address << 2, done bit 1). PCH parts (I217-I219) have no EERD: their MAC comes
    // from flash into RAL0/RAH0 at reset, so an empty one there is a real failure.
    let (lo, hi) = (r(RAL0), r(RAH0));
    let mut mac = [lo as u8, (lo >> 8) as u8, (lo >> 16) as u8, (lo >> 24) as u8, hi as u8, (hi >> 8) as u8];
    if mac == [0; 6] {
        let dev = (cfg32(b, d, f, 0) >> 16) as u16;
        let (shift, done) = match dev {
            0x100E | 0x100F => (8, 1u32 << 4),
            0x10D3 | 0x10C9 => (2, 1u32 << 1),
            _ => (0, 0),
        };
        let mut ok = done != 0;
        for word in 0..3u32 {
            if !ok {
                break;
            }
            w(EERD, word << shift | 1);
            ok = wait(|| r(EERD) & done != 0);
            let v = r(EERD) >> 16;
            mac[word as usize * 2] = v as u8;
            mac[word as usize * 2 + 1] = (v >> 8) as u8;
        }
        if !ok || mac == [0; 6] {
            crate::kprintln!("[e1000] {}: no MAC address (RAL0 empty, EEPROM unreadable)", name);
            return false;
        }
        w(RAL0, u32::from_le_bytes([mac[0], mac[1], mac[2], mac[3]]));
        crate::kprintln!("[e1000] {}: MAC read from the EEPROM", name);
    }
    w(RAH0, u32::from_le_bytes([mac[4], mac[5], 0, 0]) | 1 << 31); // address valid
    // SAFETY: the rings and buffers are ours alone until RCTL/TCTL enable the engines.
    unsafe {
        MAC = mac;
        let rx = &mut (*addr_of_mut!(RXR)).0;
        for (i, dsc) in rx.iter_mut().enumerate() {
            *dsc = RxDesc { addr: addr_of!(RXB[i]) as u64, ..RX0 };
        }
        let tx = &mut (*addr_of_mut!(TXR)).0;
        for (i, dsc) in tx.iter_mut().enumerate() {
            *dsc = TxDesc { addr: addr_of!(TXB[i]) as u64, status: DD, ..TX0 };
        }
    }
    let (rxa, txa) = (addr_of!(RXR) as u64, addr_of!(TXR) as u64);
    if adv {
        w(RXDCTL, 0); // queue 0 off while its ring is set (82576 7.1.5.1)
        w(TXDCTL, 0);
    }
    w(RDBAL, rxa as u32);
    w(RDBAH, (rxa >> 32) as u32);
    w(RDLEN, (NRX * 16) as u32);
    w(RDH, 0);
    if adv {
        // 2 KiB buffers, advanced one-buffer descriptors; switch the queue on (write-back
        // threshold 1: every descriptor written back at once), wait until it reads back on,
        // and only then hand the NIC the buffers through RDT
        w(SRRCTL, SRRCTL_ADV_ONEBUF | (BUF / 1024) as u32);
        w(RXDCTL, QUEUE_ENABLE | 1 << 16);
        if !wait(|| r(RXDCTL) & QUEUE_ENABLE != 0) {
            crate::kprintln!("[e1000] {}: receive queue did not switch on", name);
            return false;
        }
    }
    w(RDT, NRX as u32 - 1);
    w(RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC); // 2048-byte buffers (BSIZE 00)
    w(TDBAL, txa as u32);
    w(TDBAH, (txa >> 32) as u32);
    w(TDLEN, (NTX * 16) as u32);
    w(TDH, 0);
    w(TDT, 0);
    if adv {
        w(TXDCTL, QUEUE_ENABLE | 1 << 16);
        if !wait(|| r(TXDCTL) & QUEUE_ENABLE != 0) {
            crate::kprintln!("[e1000] {}: transmit queue did not switch on", name);
            return false;
        }
    }
    w(TIPG, 0x0060_200A);
    w(TCTL, TCTL_EN | TCTL_PSP | 0x0F << 4 | 0x40 << 12);
    // a real PHY needs a few seconds of auto-negotiation before the link is up
    let (t0, hz) = (crate::timer::cycles(), crate::timer::hz().max(1));
    while r(STATUS) & 2 == 0 && crate::timer::cycles() - t0 < 5 * hz {
        core::hint::spin_loop();
    }
    crate::kprintln!(
        "[e1000] {} at {:02x}:{:02x}.{}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, link {}",
        name, b, d, f, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        if r(STATUS) & 2 != 0 { "up" } else { "down" }
    );
    true
}

pub fn mac() -> [u8; 6] {
    // SAFETY: scalar read of boot-time state.
    unsafe { MAC }
}

pub fn link_up() -> bool {
    // SAFETY: scalar read.
    unsafe { BAR != 0 && r(STATUS) & 2 != 0 }
}

/// Put one Ethernet frame (no FCS: the NIC appends it) on the wire. False if the ring
/// stays full.
pub fn send(frame: &[u8]) -> bool {
    // SAFETY: single core; the descriptor at TX_NEXT is ours once the NIC set DD on it.
    unsafe {
        if BAR == 0 || frame.len() > BUF {
            return false;
        }
        let i = TX_NEXT;
        let dsc = addr_of_mut!(TXR.0[i]);
        if !wait(|| read_volatile(addr_of!((*dsc).status)) & DD != 0) {
            crate::kprintln!("[e1000] transmit ring stuck");
            return false;
        }
        core::ptr::copy_nonoverlapping(frame.as_ptr(), addr_of_mut!(TXB[i]) as *mut u8, frame.len());
        if ADV {
            // advanced data descriptor: length, DTYP data, DCMD (EOP, IFCS, RS, DEXT), PAYLEN;
            // the NIC writes DD back into bit 32 -- the same byte as the legacy status
            let n = frame.len() as u64;
            let cmd = ((CMD_EOP | CMD_IFCS | CMD_RS) as u64) << 24 | ADV_DEXT;
            let p = dsc as *mut u64;
            write_volatile(p, addr_of!(TXB[i]) as u64);
            write_volatile(p.add(1), n | ADV_DTYP_DATA | cmd | n << 46);
        } else {
            write_volatile(
                dsc,
                TxDesc { addr: addr_of!(TXB[i]) as u64, len: frame.len() as u16, cmd: CMD_EOP | CMD_IFCS | CMD_RS, ..TX0 },
            );
        }
        TX_NEXT = (i + 1) % NTX;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w(TDT, TX_NEXT as u32);
        SENT += 1;
        true
    }
}

/// Take the next received frame, copying it into `dst`; its length, or None if none waits.
pub fn recv(dst: &mut [u8]) -> Option<usize> {
    // SAFETY: single core; a descriptor with DD set is ours until RDT gives it back.
    unsafe {
        if BAR == 0 {
            return None;
        }
        let i = RX_NEXT;
        let dsc = addr_of_mut!(RXR.0[i]);
        if !rx_state(i).0 {
            return None;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let (_, eop, bad, len) = rx_state(i);
        let whole = eop && !bad && !DROPPING;
        // a piece without EOP, or a bad frame, starts (or continues) dropping until its EOP
        DROPPING = !eop;
        let n = len.min(dst.len());
        if whole {
            core::ptr::copy_nonoverlapping(addr_of!(RXB[i]) as *const u8, dst.as_mut_ptr(), n);
        }
        write_volatile(dsc, RxDesc { addr: addr_of!(RXB[i]) as u64, ..RX0 });
        RX_NEXT = (i + 1) % NRX;
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w(RDT, i as u32);
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
    // SAFETY: a volatile read of one descriptor's status.
    unsafe { BAR != 0 && rx_state(RX_NEXT).0 }
}

/// `lse1000`: the NIC, its link and its counters.
pub fn list() {
    // SAFETY: scalar reads.
    let (bar, name, (b, d, f), m, s, rcv) = unsafe { (BAR, NAME, *addr_of!(PCI_AT), MAC, SENT, RECEIVED) };
    if bar == 0 {
        crate::kprintln!("e1000: no Intel e1000/e1000e NIC");
        return;
    }
    crate::kprintln!(
        "e1000: {} at {:02x}:{:02x}.{} BAR0 {:#x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, link {}",
        name, b, d, f, bar, m[0], m[1], m[2], m[3], m[4], m[5], if link_up() { "up" } else { "down" }
    );
    crate::kprintln!("  frames sent {}, received {}", s, rcv);
}

/// True once init found and set up a NIC.
pub fn present() -> bool {
    // SAFETY: scalar read.
    unsafe { BAR != 0 }
}
