//! The PC's virtio transport: virtio devices as PCI functions (PC track step 2, docs/05).
//!
//! The ARM kernel's `virtio.rs` drives net, disk, sound and rng through the virtio-MMIO
//! register block of QEMU `virt`: every access is `r(base, R_X)` / `w(base, R_X, v)`, plus
//! byte reads of the device config at `base + R_CONFIG`. A PC has the same devices as PCI
//! functions with the VIRTIO 1.x "modern" PCI layout instead: a common-config block, a notify
//! area, and a device-config block, each found through a vendor capability and living in a
//! BAR. This module is compiled INSIDE `virtio.rs` on the PC (a child module, so it reaches
//! the driver's own set-up functions) and supplies the `r`/`w`/`dsb`/`init`/`lsvirtio` that
//! the ARM build defines for itself. Every other line of the driver -- rings, packets, TCP,
//! DHCP, sectors, PCM -- is the ARM code unchanged.
//!
//! The trick that keeps the driver unedited: each PCI device's `base` is chosen as
//! (its device-config address - R_CONFIG), so the driver's own `base + R_CONFIG + i` byte
//! reads land in the right place, and `r`/`w` find the device by that `base` and translate
//! each virtio-MMIO register into its common-config field (same meaning, per the spec).
//!
//! A PC with no virtio NIC (every real PC) gets one in software: when an Intel e1000/e1000e
//! is present (pc/e1000.rs), `init` adds an EMULATED virtio-net device whose registers this
//! module answers itself. The driver's TX doorbell sends each queued frame through the
//! e1000, and `emu_poll` (the local-APIC timer interrupt, and every RX doorbell) moves
//! received frames into the driver's RX buffers and used ring -- exactly what a virtio-net
//! device would do, so the shared TCP/IP stack runs on the Intel NIC unedited.

use super::{dev_name, VIRTIO_MAGIC};
use super::{R_CONFIG, R_DEVICE_FEATURES, R_DEVICE_FEATURES_SEL, R_DEVICE_ID, R_DRIVER_FEATURES};
use super::{R_DRIVER_FEATURES_SEL, R_MAGIC, R_QUEUE_DESC_HIGH, R_QUEUE_DESC_LOW, R_QUEUE_DEVICE_HIGH};
use super::{R_QUEUE_DEVICE_LOW, R_QUEUE_DRIVER_HIGH, R_QUEUE_DRIVER_LOW, R_QUEUE_NOTIFY, R_QUEUE_NUM};
use super::{R_QUEUE_NUM_MAX, R_QUEUE_READY, R_QUEUE_SEL, R_STATUS, R_VERSION};
use super::{DEV_BLK, DEV_NET, DEV_RNG, DEV_SOUND};
use core::ptr::{read_volatile, write_volatile};

const VENDOR_VIRTIO: u16 = 0x1AF4;
const MAX_DEVS: usize = 8;

// virtio_pci_common_cfg field offsets (VIRTIO 1.x, 4.1.4.3)
const C_DFSELECT: u64 = 0x00;
const C_DF: u64 = 0x04;
const C_GFSELECT: u64 = 0x08;
const C_GF: u64 = 0x0C;
const C_STATUS: u64 = 0x14;
const C_Q_SELECT: u64 = 0x16;
const C_Q_SIZE: u64 = 0x18;
const C_Q_ENABLE: u64 = 0x1C;
const C_Q_NOFF: u64 = 0x1E;
const C_Q_DESC: u64 = 0x20;
const C_Q_DRIVER: u64 = 0x28;
const C_Q_DEVICE: u64 = 0x30;

#[derive(Clone, Copy)]
struct Dev {
    base: u64,   // the driver's handle: device-config address - R_CONFIG
    id: u32,     // virtio device id (1 net, 2 blk, 4 rng, 25 sound)
    common: u64, // common-config block
    notify: u64, // notify area
    mult: u32,   // notify_off_multiplier
    bdf: (u8, u8, u8),
}

/// An emulated device is marked by `common == 0` (a real one always has a common block).
const NO_DEV: Dev = Dev { base: 0, id: 0, common: 0, notify: 0, mult: 0, bdf: (0, 0, 0) };
static mut DEVS: [Dev; MAX_DEVS] = [NO_DEV; MAX_DEVS];
static mut NDEVS: usize = 0;

// --- PCI configuration space through the shared pc/pci.rs (review R1-8) ---

use crate::pci::{cfg_read32 as cfg32, cfg_write32};

fn cfg8(b: u8, d: u8, f: u8, off: u8) -> u8 {
    (cfg32(b, d, f, off) >> ((off & 3) * 8)) as u8
}
fn cfg16(b: u8, d: u8, f: u8, off: u8) -> u16 {
    (cfg32(b, d, f, off) >> ((off & 2) * 8)) as u16
}

/// The address BAR `bar` decodes to (memory BARs, 32- or 64-bit), or 0.
fn bar_addr(b: u8, d: u8, f: u8, bar: u8) -> u64 {
    if bar > 5 {
        return 0;
    }
    let lo = cfg32(b, d, f, 0x10 + bar * 4);
    if lo & 1 != 0 {
        return 0; // an I/O-port BAR: the modern layout lives in memory BARs
    }
    let mut a = (lo & !0xF) as u64;
    if (lo >> 1) & 3 == 2 && bar < 5 {
        a |= (cfg32(b, d, f, 0x10 + (bar + 1) * 4) as u64) << 32;
    }
    a
}

/// Look at one PCI function; if it is a modern-capable virtio device, record it.
fn probe(b: u8, d: u8, f: u8) {
    let vd = cfg32(b, d, f, 0);
    if vd as u16 != VENDOR_VIRTIO {
        return;
    }
    let pid = (vd >> 16) as u16;
    let id = match pid {
        0x1040..=0x107F => (pid - 0x1040) as u32,
        0x1000..=0x103F => cfg16(b, d, f, 0x2E) as u32, // transitional: the subsystem id
        _ => return,
    };
    if cfg16(b, d, f, 0x06) & (1 << 4) == 0 {
        return; // no capability list: a legacy-only device
    }
    let (mut common, mut notify, mut mult, mut devcfg) = (0u64, 0u64, 0u32, 0u64);
    let mut lens = [0u64; 4];
    let mut p = cfg8(b, d, f, 0x34) & 0xFC;
    let mut guard = 0;
    while p != 0 && guard < 48 {
        guard += 1;
        if cfg8(b, d, f, p) == 0x09 {
            let ty = cfg8(b, d, f, p + 3);
            let bar = cfg8(b, d, f, p + 4);
            let off = cfg32(b, d, f, p + 8) as u64;
            let len = cfg32(b, d, f, p + 12) as u64;
            let ba = bar_addr(b, d, f, bar);
            if ba != 0 {
                match ty {
                    1 if common == 0 => { common = ba + off; lens[0] = len; }
                    2 if notify == 0 => {
                        notify = ba + off;
                        lens[1] = len;
                        mult = cfg32(b, d, f, p + 16);
                    }
                    4 if devcfg == 0 => { devcfg = ba + off; lens[3] = len; }
                    _ => {}
                }
            }
        }
        p = cfg8(b, d, f, p + 1) & 0xFC;
    }
    if common == 0 || notify == 0 {
        crate::kprintln!("[virtio] pci {:02x}:{:02x}.{}: {} without the modern layout, skipped", b, d, f, dev_name(id));
        return;
    }
    // memory decoding + bus mastering (the device reads and writes our rings)
    let cmd = cfg32(b, d, f, 0x04);
    cfg_write32(b, d, f, 0x04, (cmd & 0xFFFF) | 0x406); // ... and INTx off: these devices are polled
    let ok = crate::mmu::map_mmio(common, lens[0].max(0x38))
        && crate::mmu::map_mmio(notify, lens[1].max(2))
        && (devcfg == 0 || crate::mmu::map_mmio(devcfg, lens[3].max(1)));
    if !ok {
        crate::kprintln!("[virtio] pci {:02x}:{:02x}.{}: could not map its registers", b, d, f);
        return;
    }
    // SAFETY: single-threaded bring-up.
    unsafe {
        if NDEVS == MAX_DEVS {
            return;
        }
        let key = if devcfg != 0 { devcfg } else { common } - R_CONFIG;
        DEVS[NDEVS] = Dev { base: key, id, common, notify, mult, bdf: (b, d, f) };
        NDEVS += 1;
    }
}

/// Find every virtio PCI function (once). Buses 0-7 cover QEMU q35 with its root ports.
fn scan() {
    // SAFETY: single-threaded bring-up; NDEVS only grows here.
    if unsafe { NDEVS } != 0 {
        return;
    }
    for b in 0..8u8 {
        for d in 0..32u8 {
            if cfg32(b, d, 0, 0) as u16 == 0xFFFF {
                continue;
            }
            let funcs = if cfg8(b, d, 0, 0x0E) & 0x80 != 0 { 8 } else { 1 };
            for f in 0..funcs {
                probe(b, d, f);
            }
        }
    }
}

fn dev(base: u64) -> Option<Dev> {
    // SAFETY: read-only look-up of the table `scan` filled.
    unsafe { DEVS[..NDEVS].iter().copied().find(|d| d.base == base) }
}

// --- the emulated virtio-net device over an Intel NIC ---

const VNET_HDR: usize = 12; // virtio_net_hdr (v1), as the driver lays it out
#[derive(Clone, Copy)]
struct Eq {
    num: u32,
    desc: u64,
    avail: u64,
    used: u64,
    ready: bool,
    last: u16, // the next avail-ring entry this device will take
}
const EQ0: Eq = Eq { num: 0, desc: 0, avail: 0, used: 0, ready: false, last: 0 };
struct Emu {
    dfsel: u32,
    gfsel: u32,
    gf: [u32; 2],
    status: u32,
    qsel: u32,
    q: [Eq; 2], // 0 = RX, 1 = TX
}
static mut EMU: Emu = Emu { dfsel: 0, gfsel: 0, gf: [0; 2], status: 0, qsel: 0, q: [EQ0; 2] };
/// The device-config block the driver reads bytes of: MAC (0..6), status (6..8, LINK_UP).
static mut EMU_CFG: [u8; 64] = [0; 64];
static mut EMU_BASE: u64 = 0;
static mut EMU_RX: u64 = 0;
static mut EMU_TX: u64 = 0;

unsafe fn emu_r(off: u64) -> u32 {
    let e = &*core::ptr::addr_of!(EMU);
    let q = e.q[(e.qsel as usize).min(1)];
    match off {
        R_MAGIC => VIRTIO_MAGIC,
        R_VERSION => 2,
        R_DEVICE_ID => DEV_NET,
        R_DEVICE_FEATURES_SEL => e.dfsel,
        // VIRTIO_NET_F_MAC (bit 5) in word 0, VIRTIO_F_VERSION_1 (bit 32) in word 1
        R_DEVICE_FEATURES => match e.dfsel {
            0 => 1 << 5,
            1 => 1,
            _ => 0,
        },
        R_DRIVER_FEATURES_SEL => e.gfsel,
        R_DRIVER_FEATURES => e.gf[(e.gfsel as usize).min(1)],
        R_QUEUE_SEL => e.qsel,
        R_QUEUE_NUM_MAX => if e.qsel < 2 { 256 } else { 0 },
        R_QUEUE_NUM => q.num,
        R_QUEUE_READY => q.ready as u32,
        R_STATUS => e.status,
        R_QUEUE_DESC_LOW => q.desc as u32,
        R_QUEUE_DESC_HIGH => (q.desc >> 32) as u32,
        R_QUEUE_DRIVER_LOW => q.avail as u32,
        R_QUEUE_DRIVER_HIGH => (q.avail >> 32) as u32,
        R_QUEUE_DEVICE_LOW => q.used as u32,
        R_QUEUE_DEVICE_HIGH => (q.used >> 32) as u32,
        o if o >= R_CONFIG => rd32(EMU_BASE + o),
        _ => 0,
    }
}

unsafe fn emu_w(off: u64, v: u32) {
    let e = &mut *core::ptr::addr_of_mut!(EMU);
    let qi = e.qsel as usize;
    let lo = |x: u64| (x & !0xFFFF_FFFF) | v as u64;
    let hi = |x: u64| (x & 0xFFFF_FFFF) | (v as u64) << 32;
    match off {
        R_DEVICE_FEATURES_SEL => e.dfsel = v,
        R_DRIVER_FEATURES_SEL => e.gfsel = v,
        R_DRIVER_FEATURES => e.gf[(e.gfsel as usize).min(1)] = v,
        R_QUEUE_SEL => e.qsel = v,
        R_STATUS => {
            if v == 0 {
                e.status = 0;
                e.q = [EQ0; 2];
            } else {
                e.status = v;
            }
        }
        R_QUEUE_NUM if qi < 2 => e.q[qi].num = v,
        R_QUEUE_READY if qi < 2 => e.q[qi].ready = v != 0,
        R_QUEUE_DESC_LOW if qi < 2 => e.q[qi].desc = lo(e.q[qi].desc),
        R_QUEUE_DESC_HIGH if qi < 2 => e.q[qi].desc = hi(e.q[qi].desc),
        R_QUEUE_DRIVER_LOW if qi < 2 => e.q[qi].avail = lo(e.q[qi].avail),
        R_QUEUE_DRIVER_HIGH if qi < 2 => e.q[qi].avail = hi(e.q[qi].avail),
        R_QUEUE_DEVICE_LOW if qi < 2 => e.q[qi].used = lo(e.q[qi].used),
        R_QUEUE_DEVICE_HIGH if qi < 2 => e.q[qi].used = hi(e.q[qi].used),
        R_QUEUE_NOTIFY => {
            if v == 1 {
                emu_tx();
            } else {
                emu_poll();
            }
        }
        _ => {}
    }
}

/// Publish one used-ring entry for queue `q` (the device's half of the contract).
unsafe fn emu_used(q: &Eq, id: u32, len: u32) {
    let n = q.num.max(1) as u64;
    let idx = rd16(q.used + 2) as u16;
    let elem = q.used + 4 + 8 * (idx as u64 % n);
    wr32(elem, id);
    wr32(elem + 4, len);
    dsb();
    wr16(q.used + 2, idx.wrapping_add(1) as u32);
    dsb();
}

/// The TX doorbell: every frame the driver queued goes out through the Intel NIC.
unsafe fn emu_tx() {
    let q = (*core::ptr::addr_of!(EMU)).q[1];
    if !q.ready || q.num == 0 {
        return;
    }
    let mut buf = [0u8; 2048 + VNET_HDR];
    let mut last = q.last;
    while last != rd16(q.avail + 2) as u16 {
        let id = rd16(q.avail + 4 + 2 * (last as u64 % q.num as u64));
        // gather the descriptor chain (the driver uses one descriptor; a chain is followed)
        let (mut d, mut n, mut hops) = (id as u64, 0usize, 0);
        loop {
            let da = q.desc + 16 * d;
            let addr = rd32(da) as u64 | (rd32(da + 4) as u64) << 32;
            let (len, flags, next) = (rd32(da + 8) as usize, rd16(da + 12), rd16(da + 14));
            let take = len.min(buf.len() - n);
            core::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr().add(n), take);
            n += take;
            hops += 1;
            if flags & 1 == 0 || hops >= 4 {
                break;
            }
            d = next as u64;
        }
        if n > VNET_HDR {
            crate::nic::send(&buf[VNET_HDR..n]);
            EMU_TX += 1;
        }
        emu_used(&q, id, 0);
        last = last.wrapping_add(1);
    }
    (*core::ptr::addr_of_mut!(EMU)).q[1].last = last;
}

/// Move every frame the Intel NIC has received into the driver's free RX buffers. Called
/// from the local-APIC timer interrupt and on the driver's RX doorbell; does nothing until
/// the driver has set the device up (DRIVER_OK).
pub fn emu_poll() {
    // SAFETY: single core. It runs either in the driver's own context (the RX doorbell) or
    // in the timer interrupt; the driver only READS the used ring this writes, and writes
    // the avail ring entry before its index, so whatever index is read here is complete.
    unsafe {
        if EMU_BASE == 0 {
            return;
        }
        let q = (*core::ptr::addr_of!(EMU)).q[0];
        if !q.ready || q.num == 0 || (*core::ptr::addr_of!(EMU)).status & 4 == 0 {
            return;
        }
        let mut last = q.last;
        while last != rd16(q.avail + 2) as u16 && crate::nic::rx_ready() {
            let id = rd16(q.avail + 4 + 2 * (last as u64 % q.num as u64));
            let da = q.desc + 16 * id as u64;
            let addr = rd32(da) as u64 | (rd32(da + 4) as u64) << 32;
            let cap = rd32(da + 8) as usize;
            if cap <= VNET_HDR {
                break;
            }
            let dst = core::slice::from_raw_parts_mut(addr as *mut u8, cap);
            match crate::nic::recv(&mut dst[VNET_HDR..]) {
                Some(0) => continue, // a bad frame, dropped; the buffer stays the driver's
                Some(n) => {
                    dst[..VNET_HDR].fill(0);
                    emu_used(&q, id, (VNET_HDR + n) as u32);
                    EMU_RX += 1;
                    last = last.wrapping_add(1);
                }
                None => break,
            }
        }
        (*core::ptr::addr_of_mut!(EMU)).q[0].last = last;
    }
}

unsafe fn rd8(a: u64) -> u32 { read_volatile(a as *const u8) as u32 }
unsafe fn rd16(a: u64) -> u32 { read_volatile(a as *const u16) as u32 }
unsafe fn rd32(a: u64) -> u32 { read_volatile(a as *const u32) }
unsafe fn wr8(a: u64, v: u32) { write_volatile(a as *mut u8, v as u8) }
unsafe fn wr16(a: u64, v: u32) { write_volatile(a as *mut u16, v as u16) }
unsafe fn wr32(a: u64, v: u32) { write_volatile(a as *mut u32, v) }

/// A virtio-MMIO register read, answered from the PCI device's common config.
pub(super) unsafe fn r(base: u64, off: u64) -> u32 {
    let d = match dev(base) {
        Some(d) => d,
        None => return 0,
    };
    if d.common == 0 {
        return emu_r(off);
    }
    let c = d.common;
    match off {
        R_MAGIC => VIRTIO_MAGIC,
        R_VERSION => 2,
        R_DEVICE_ID => d.id,
        R_DEVICE_FEATURES_SEL => rd32(c + C_DFSELECT),
        R_DEVICE_FEATURES => rd32(c + C_DF),
        R_DRIVER_FEATURES_SEL => rd32(c + C_GFSELECT),
        R_DRIVER_FEATURES => rd32(c + C_GF),
        R_QUEUE_SEL => rd16(c + C_Q_SELECT),
        R_QUEUE_NUM_MAX | R_QUEUE_NUM => rd16(c + C_Q_SIZE),
        R_QUEUE_READY => rd16(c + C_Q_ENABLE),
        R_STATUS => rd8(c + C_STATUS),
        R_QUEUE_DESC_LOW => rd32(c + C_Q_DESC),
        R_QUEUE_DESC_HIGH => rd32(c + C_Q_DESC + 4),
        R_QUEUE_DRIVER_LOW => rd32(c + C_Q_DRIVER),
        R_QUEUE_DRIVER_HIGH => rd32(c + C_Q_DRIVER + 4),
        R_QUEUE_DEVICE_LOW => rd32(c + C_Q_DEVICE),
        R_QUEUE_DEVICE_HIGH => rd32(c + C_Q_DEVICE + 4),
        o if o >= R_CONFIG => rd32(base + o),
        _ => 0,
    }
}

/// A virtio-MMIO register write, carried out on the PCI device's common config / notify area.
pub(super) unsafe fn w(base: u64, off: u64, v: u32) {
    let d = match dev(base) {
        Some(d) => d,
        None => return,
    };
    if d.common == 0 {
        return emu_w(off, v);
    }
    let c = d.common;
    match off {
        R_DEVICE_FEATURES_SEL => wr32(c + C_DFSELECT, v),
        R_DRIVER_FEATURES_SEL => wr32(c + C_GFSELECT, v),
        R_DRIVER_FEATURES => wr32(c + C_GF, v),
        R_QUEUE_SEL => wr16(c + C_Q_SELECT, v),
        R_QUEUE_NUM => wr16(c + C_Q_SIZE, v),
        // virtio-MMIO lets a driver write QueueReady = 0 before setting a queue up; on PCI
        // only 1 may be written (QEMU marks the device broken on a 0), and a queue starts
        // disabled after reset anyway -- so a 0 is dropped here
        R_QUEUE_READY => {
            if v != 0 {
                wr16(c + C_Q_ENABLE, v)
            }
        }
        R_STATUS => {
            wr8(c + C_STATUS, v);
            if v == 0 {
                // a reset completes when the status reads back 0 (4.1.4.3.2)
                let mut n = 0;
                while rd8(c + C_STATUS) != 0 && n < 1_000_000 {
                    n += 1;
                    core::hint::spin_loop();
                }
            }
        }
        R_QUEUE_DESC_LOW => wr32(c + C_Q_DESC, v),
        R_QUEUE_DESC_HIGH => wr32(c + C_Q_DESC + 4, v),
        R_QUEUE_DRIVER_LOW => wr32(c + C_Q_DRIVER, v),
        R_QUEUE_DRIVER_HIGH => wr32(c + C_Q_DRIVER + 4, v),
        R_QUEUE_DEVICE_LOW => wr32(c + C_Q_DEVICE, v),
        R_QUEUE_DEVICE_HIGH => wr32(c + C_Q_DEVICE + 4, v),
        R_QUEUE_NOTIFY => {
            // the queue's doorbell: notify + queue_notify_off(v) * multiplier
            let was = rd16(c + C_Q_SELECT);
            wr16(c + C_Q_SELECT, v);
            let noff = rd16(c + C_Q_NOFF) as u64;
            wr16(c + C_Q_SELECT, was);
            wr16(d.notify + noff * d.mult as u64, v);
        }
        o if o >= R_CONFIG => wr32(base + o, v),
        _ => {}
    }
}

/// The ARM build's `dsb sy`: on x86 the device sees memory in program order once the
/// compiler is kept from reordering, so a full fence is enough.
#[inline]
pub(super) unsafe fn dsb() {
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// Find the virtio PCI devices, log them, and bring each one up with the shared driver's
/// own set-up routine (the same calls, in the same order, as the ARM `init`).
pub fn init() {
    scan();
    // No virtio NIC but an Intel one: present it to the driver as virtio-net (see the top).
    // SAFETY: single-threaded bring-up.
    unsafe {
        if !DEVS[..NDEVS].iter().any(|d| d.id == DEV_NET) && NDEVS < MAX_DEVS && crate::nic::init() {
            let mac = crate::nic::mac();
            let cfg = core::ptr::addr_of_mut!(EMU_CFG) as *mut u8;
            for (i, b) in mac.iter().enumerate() {
                *cfg.add(i) = *b;
            }
            *cfg.add(6) = 1; // VIRTIO_NET_S_LINK_UP
            EMU_BASE = cfg as u64 - R_CONFIG;
            DEVS[NDEVS] = Dev { base: EMU_BASE, id: DEV_NET, common: 0, notify: 0, mult: 0, bdf: (0, 0, 0) };
            NDEVS += 1;
        }
    }
    // SAFETY: single-threaded bring-up.
    let devs = unsafe { DEVS };
    let n = unsafe { NDEVS };
    for (i, dv) in devs[..n].iter().enumerate() {
        let (b, d, f) = dv.bdf;
        if dv.common == 0 {
            crate::kprintln!("[virtio] slot {}: {} (id={}), emulated over {}", i, dev_name(dv.id), dv.id, crate::nic::describe());
        } else {
            crate::kprintln!(
                "[virtio] pci {:02x}:{:02x}.{} (slot {}) @ {:#x}: {} (id={}, v2)",
                b, d, f, i, dv.common, dev_name(dv.id), dv.id
            );
        }
        let base = dv.base;
        if dv.id == DEV_SOUND {
            let ok = super::sound_negotiate(base, 2);
            // SAFETY: single-threaded bring-up.
            unsafe {
                super::SOUND_BASE = if ok { base } else { 0 };
            }
            crate::asili::register("virtio-sound", crate::asili::Class::Audio, base, ok);
        } else if dv.id == DEV_NET {
            let ok = super::net_setup(base);
            crate::asili::register("virtio-net", crate::asili::Class::Net, base, ok);
        } else if dv.id == DEV_BLK {
            let ok = super::blk_setup(base);
            crate::asili::register("virtio-blk", crate::asili::Class::Storage, base, ok);
        } else if dv.id == DEV_RNG {
            let ok = super::rng_setup(base);
            crate::asili::register("virtio-rng", crate::asili::Class::Entropy, base, ok);
        } else {
            crate::asili::register(dev_name(dv.id), crate::asili::Class::Firmware, base, true);
        }
    }
    if n == 0 {
        crate::kprintln!("[virtio] no virtio PCI devices present (add -device virtio-net-pci)");
    }
}

/// The `lsvirtio` shell command on a PC: every virtio PCI function, read-only.
pub fn lsvirtio() {
    scan();
    // SAFETY: read-only look at the table.
    let n = unsafe { NDEVS };
    for i in 0..n {
        let dv = unsafe { DEVS[i] };
        if dv.common == 0 {
            // SAFETY: counter reads.
            let (rx, tx) = unsafe { (EMU_RX, EMU_TX) };
            crate::kprintln!("lsvirtio: slot {} (emulated over {}) id {} ({}), {} frames in, {} out", i, crate::nic::short(), dv.id, dev_name(dv.id), rx, tx);
        } else {
            crate::kprintln!("lsvirtio: slot {} @ {:#010x} id {} ({})", i, dv.common, dv.id, dev_name(dv.id));
        }
    }
    crate::kprintln!("lsvirtio: {} virtio device(s) present", n);
}
