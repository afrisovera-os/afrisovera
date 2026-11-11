//! virtio-mmio bus enumeration + a **virtio-sound** driver (bring-up).
//!
//! The QEMU `virt` machine always instantiates 32 `virtio-mmio` transport slots
//! at `0x0a00_0000` (stride `0x200`). Each slot advertises a device via memory-
//! mapped registers; an empty slot reports device id 0. This module walks those
//! slots, identifies what is attached, and — for the audio device — performs the
//! virtio initialization handshake (reset → ACKNOWLEDGE → DRIVER → feature
//! negotiation → FEATURES_OK) so Asili Voice has a real device to talk to.
//!
//! Scope of *this* milestone (2.20): **detect + negotiate**. The device is found,
//! features are agreed, and its four virtqueues (control/event/tx/rx) are sized.
//! Actual PCM streaming (setting up the descriptor rings and pushing audio frames
//! so `speak()` makes sound into the `wav` audiodev) is the next milestone (2.21).
//! Everything here is honest bring-up: no queue is driven yet, so no sound plays.
//!
//! The virtio-mmio register layout and status bits follow the VIRTIO 1.x spec.

use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile}; #[cfg(not(target_arch = "aarch64"))] #[path = "pc/virtio_pci.rs"] mod pci; #[cfg(not(target_arch = "aarch64"))] pub use pci::{emu_poll, init, lsvirtio}; #[cfg(not(target_arch = "aarch64"))] use pci::{dsb, r, w};

const VIRTIO_MMIO_BASE: u64 = crate::board::VIRTIO_MMIO_BASE;
const VIRTIO_MMIO_STRIDE: u64 = 0x200;
const VIRTIO_MMIO_SLOTS: usize = 32;
const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt" little-endian

// --- virtio-mmio register offsets (VIRTIO 1.x, "modern" v2) ---
const R_MAGIC: u64 = 0x000;
const R_VERSION: u64 = 0x004;
const R_DEVICE_ID: u64 = 0x008;
const R_DEVICE_FEATURES: u64 = 0x010;
const R_DEVICE_FEATURES_SEL: u64 = 0x014;
const R_DRIVER_FEATURES: u64 = 0x020;
const R_DRIVER_FEATURES_SEL: u64 = 0x024;
const R_QUEUE_SEL: u64 = 0x030;
const R_QUEUE_NUM_MAX: u64 = 0x034;
const R_QUEUE_NUM: u64 = 0x038;
const R_QUEUE_READY: u64 = 0x044;
const R_QUEUE_NOTIFY: u64 = 0x050;
const R_STATUS: u64 = 0x070;
const R_QUEUE_DESC_LOW: u64 = 0x080;
const R_QUEUE_DESC_HIGH: u64 = 0x084;
const R_QUEUE_DRIVER_LOW: u64 = 0x090;
const R_QUEUE_DRIVER_HIGH: u64 = 0x094;
const R_QUEUE_DEVICE_LOW: u64 = 0x0a0;
const R_QUEUE_DEVICE_HIGH: u64 = 0x0a4;

// --- device status bits ---
const S_ACK: u32 = 1;
const S_DRIVER: u32 = 2;
const S_DRIVER_OK: u32 = 4;
const S_FEATURES_OK: u32 = 8;
const S_FAILED: u32 = 128;

const DEV_SOUND: u32 = 25; // virtio device id for sound
const DEV_NET: u32 = 1; // virtio device id for network
const DEV_BLK: u32 = 2; // virtio device id for block storage
const DEV_RNG: u32 = 4; // virtio device id for entropy (VIRTIO_ID_ENTROPY)
const R_CONFIG: u64 = 0x100; // device-specific config space

// Base address of the negotiated virtio-sound device, or 0 if none.
static mut SOUND_BASE: u64 = 0;
// Max descriptors reported for the tx (playback) queue — used by the streaming
// milestone; recorded here at negotiation time.
static mut SOUND_TXQ_MAX: u32 = 0;

#[inline] #[cfg(target_arch = "aarch64")]
unsafe fn r(base: u64, off: u64) -> u32 {
    read_volatile((base + off) as *const u32)
}
#[inline] #[cfg(target_arch = "aarch64")]
unsafe fn w(base: u64, off: u64, v: u32) {
    write_volatile((base + off) as *mut u32, v);
}

fn dev_name(id: u32) -> &'static str {
    match id {
        1 => "virtio-net",
        2 => "virtio-blk",
        3 => "virtio-console",
        4 => "virtio-rng",
        16 => "virtio-gpu",
        18 => "virtio-input",
        19 => "virtio-crypto",
        25 => "virtio-sound",
        _ => "virtio-dev",
    }
}
#[cfg(target_arch = "aarch64")]
/// Walk the virtio-mmio bus, log every attached device, and negotiate with the
/// sound device if present. Registers findings in the Asili device manager.
pub fn init() {
    let mut found = 0;
    for i in 0..VIRTIO_MMIO_SLOTS {
        let base = VIRTIO_MMIO_BASE + (i as u64) * VIRTIO_MMIO_STRIDE;
        // SAFETY: the whole [0,1GiB) window is device-mapped (paging.rs).
        let magic = unsafe { r(base, R_MAGIC) };
        if magic != VIRTIO_MAGIC {
            continue; // not a virtio-mmio transport
        }
        let devid = unsafe { r(base, R_DEVICE_ID) };
        if devid == 0 {
            continue; // slot present but no device bound
        }
        found += 1;
        let ver = unsafe { r(base, R_VERSION) };
        crate::kprintln!(
            "[virtio] slot {} @ {:#010x}: {} (id={}, v{})",
            i,
            base,
            dev_name(devid),
            devid,
            ver
        );
        if devid == DEV_SOUND {
            let ok = sound_negotiate(base, ver);
            // SAFETY: single-threaded bring-up.
            unsafe {
                SOUND_BASE = if ok { base } else { 0 };
            }
            crate::asili::register("virtio-sound", crate::asili::Class::Audio, base, ok);
        } else if devid == DEV_NET {
            let ok = net_setup(base);
            crate::asili::register("virtio-net", crate::asili::Class::Net, base, ok);
        } else if devid == DEV_BLK {
            let ok = blk_setup(base);
            crate::asili::register("virtio-blk", crate::asili::Class::Storage, base, ok);
        } else if devid == DEV_RNG {
            let ok = rng_setup(base);
            crate::asili::register("virtio-rng", crate::asili::Class::Entropy, base, ok);
        } else {
            crate::asili::register(dev_name(devid), crate::asili::Class::Firmware, base, true);
        }
    }
    if found == 0 {
        crate::kprintln!("[virtio] no virtio-mmio devices present (add -device virtio-sound-device)");
    }
}
#[cfg(target_arch = "aarch64")]
/// Read-only enumeration of the virtio-mmio transport slots (the `lsvirtio` shell
/// command). Walks all slots, prints each one that advertises a real transport
/// (magic == "virt") and a bound device (id != 0), and reports the total. Does not
/// touch device state -- pure discovery, safe to run at any time.
pub fn lsvirtio() {
    let mut found = 0;
    for i in 0..VIRTIO_MMIO_SLOTS {
        let base = VIRTIO_MMIO_BASE + (i as u64) * VIRTIO_MMIO_STRIDE;
        // SAFETY: the whole [0,1GiB) window is device-mapped on both boards.
        let magic = unsafe { r(base, R_MAGIC) };
        if magic != VIRTIO_MAGIC {
            continue; // not a virtio-mmio transport
        }
        let devid = unsafe { r(base, R_DEVICE_ID) };
        if devid == 0 {
            continue; // slot present but no device bound
        }
        found += 1;
        crate::kprintln!(
            "lsvirtio: slot {} @ {:#010x} id {} ({})",
            i,
            base,
            devid,
            dev_name(devid)
        );
    }
    crate::kprintln!("lsvirtio: {} virtio device(s) present", found);
}

/// Run the virtio init handshake with the sound device through FEATURES_OK, and
/// report its virtqueue sizes. Returns true if the device accepted our features.
fn sound_negotiate(base: u64, ver: u32) -> bool {
    // SAFETY: MMIO on a detected virtio-sound device; single-threaded bring-up.
    unsafe {
        w(base, R_STATUS, 0); // reset
        let mut status = S_ACK;
        w(base, R_STATUS, status); // ACKNOWLEDGE: we noticed the device
        status |= S_DRIVER;
        w(base, R_STATUS, status); // DRIVER: we know how to drive it

        // Read the device's feature bits (two 32-bit banks).
        w(base, R_DEVICE_FEATURES_SEL, 0);
        let feat_lo = r(base, R_DEVICE_FEATURES);
        w(base, R_DEVICE_FEATURES_SEL, 1);
        let feat_hi = r(base, R_DEVICE_FEATURES);

        // Accept only VIRTIO_F_VERSION_1 (feature bit 32) on modern transports;
        // a v2 device rejects FEATURES_OK unless the driver acks version 1.
        w(base, R_DRIVER_FEATURES_SEL, 0);
        w(base, R_DRIVER_FEATURES, 0);
        w(base, R_DRIVER_FEATURES_SEL, 1);
        w(base, R_DRIVER_FEATURES, if ver >= 2 { 1 } else { 0 });

        status |= S_FEATURES_OK;
        w(base, R_STATUS, status);
        let back = r(base, R_STATUS);
        if back & S_FEATURES_OK == 0 {
            crate::kprintln!("[virtio-sound] FEATURES_OK rejected (status={:#x})", back);
            w(base, R_STATUS, S_FAILED);
            return false;
        }

        // Size the four standard virtio-sound queues: control, event, tx, rx.
        let mut qmax = [0u32; 4];
        for (q, slot) in qmax.iter_mut().enumerate() {
            w(base, R_QUEUE_SEL, q as u32);
            *slot = r(base, R_QUEUE_NUM_MAX);
        }
        SOUND_TXQ_MAX = qmax[2];
        crate::kprintln!(
            "[virtio-sound] negotiated: features={:#010x}{:08x}, queues ctrl/event/tx/rx max={}/{}/{}/{}",
            feat_hi,
            feat_lo,
            qmax[0],
            qmax[1],
            qmax[2],
            qmax[3]
        );
        crate::kprintln!(
            "[virtio-sound] device ready to negotiate PCM (streaming = milestone 2.21)"
        );
        // DRIVER_OK is deliberately withheld until the queues are set up (2.21).
        true
    }
}

/// Whether a virtio-sound device was found and successfully negotiated.
pub fn sound_present() -> bool {
    // SAFETY: scalar read of bring-up state.
    unsafe { SOUND_BASE != 0 }
}

/// Base MMIO address of the negotiated sound device (0 if none). For 2.21.
pub fn sound_base() -> u64 {
    // SAFETY: scalar read.
    unsafe { SOUND_BASE }
}

/// Max descriptors on the playback (tx) queue, recorded at negotiation. For 2.21.
#[allow(dead_code)]
pub fn sound_txq_max() -> u32 {
    // SAFETY: scalar read.
    unsafe { SOUND_TXQ_MAX }
}

// ===========================================================================
// Milestone 2.21: split virtqueues + virtio-sound PCM streaming.
//
// We drive two of the device's queues by hand: the control queue (index 0) to
// configure/prepare/start a PCM stream, and the tx queue (index 2) to hand it
// audio frames. Each queue is a classic VIRTIO "split" virtqueue: a descriptor
// table plus a driver-owned "avail" ring and a device-owned "used" ring, all in
// Normal DRAM (identity-mapped, so VA == PA — the address we put in a descriptor
// is the physical address the device DMAs). QEMU is cache-coherent for virtio, so
// a `dsb` for ordering is enough; no cache maintenance is needed.
// ===========================================================================

const QSZ: usize = 8; // descriptors per queue (<= QueueNumMax)
const F_NEXT: u16 = 1; // VIRTQ_DESC_F_NEXT: descriptor chains to `next`
const F_WRITE: u16 = 2; // VIRTQ_DESC_F_WRITE: device-writable (else device-readable)

// virtio-sound control request codes and status.
const R_PCM_SET_PARAMS: u32 = 0x0101;
const R_PCM_PREPARE: u32 = 0x0102;
const R_PCM_START: u32 = 0x0104;
const SND_S_OK: u32 = 0x8000;
// PCM format / rate enum values.
const FMT_S16: u8 = 5;
const RATE_44100: u8 = 6;
const RATE_HZ: u32 = 44100;

#[derive(Clone, Copy)]
#[repr(C)]
struct Desc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}
#[repr(C, align(16))]
struct DescTable {
    d: [Desc; QSZ],
}
#[repr(C, align(16))]
struct AvailRing {
    flags: u16,
    idx: u16,
    ring: [u16; QSZ],
}
#[derive(Clone, Copy)]
#[repr(C)]
struct UsedElem {
    id: u32,
    len: u32,
}
#[repr(C, align(16))]
struct UsedRing {
    flags: u16,
    idx: u16,
    ring: [UsedElem; QSZ],
}

const DESC0: Desc = Desc { addr: 0, len: 0, flags: 0, next: 0 };

// Control queue (index 0) backing store.
static mut CTRL_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut CTRL_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut CTRL_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };
// Tx / playback queue (index 2) backing store.
static mut TX_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut TX_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut TX_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };

// DMA buffers (device-visible).
static mut REQ_BUF: [u8; 32] = [0; 32]; // control request (device-readable)
static mut RESP_BUF: [u8; 32] = [0; 32]; // control response (device-writable)
static mut XFER_HDR: [u8; 4] = [0; 4]; // tx: stream_id header
static mut TX_STATUS: [u8; 8] = [0; 8]; // tx: per-buffer status (device-writable)
const PERIOD_BYTES: u32 = 4096; // one period = 1024 stereo S16 frames
static mut PCM_BUF: [u8; PERIOD_BYTES as usize] = [0; PERIOD_BYTES as usize];
// 3.370 (design-3.367 (B)): streaming playback state. PCM_FILL is how many
// stereo frames are staged in PCM_BUF but not yet handed to the device, so a
// caller may push samples in ARBITRARY chunk sizes and still submit whole
// periods; PCM_PERIODS is the running period index, used only in the abort line.
static mut PCM_FILL: usize = 0;
static mut PCM_PERIODS: usize = 0;

static mut PHASE: u32 = 0; // tone phase, persists across periods for continuity
static mut STARTED: bool = false; // PCM stream configured + started once

#[inline] #[cfg(target_arch = "aarch64")]
unsafe fn dsb() {
    core::arch::asm!("dsb sy", options(nostack, preserves_flags));
}
#[inline]
unsafe fn wr32(buf: *mut u8, off: usize, v: u32) {
    let b = v.to_le_bytes();
    for i in 0..4 {
        write_volatile(buf.add(off + i), b[i]);
    }
}
#[inline]
unsafe fn rd32(buf: *const u8, off: usize) -> u32 {
    let mut b = [0u8; 4];
    for i in 0..4 {
        b[i] = read_volatile(buf.add(off + i));
    }
    u32::from_le_bytes(b)
}

/// Point a virtio-mmio queue's registers at our split-ring backing store.
unsafe fn queue_setup(base: u64, qidx: u32, desc: u64, avail: u64, used: u64) {
    w(base, R_QUEUE_SEL, qidx);
    w(base, R_QUEUE_READY, 0);
    w(base, R_QUEUE_NUM, QSZ as u32);
    w(base, R_QUEUE_DESC_LOW, desc as u32);
    w(base, R_QUEUE_DESC_HIGH, (desc >> 32) as u32);
    w(base, R_QUEUE_DRIVER_LOW, avail as u32);
    w(base, R_QUEUE_DRIVER_HIGH, (avail >> 32) as u32);
    w(base, R_QUEUE_DEVICE_LOW, used as u32);
    w(base, R_QUEUE_DEVICE_HIGH, (used >> 32) as u32);
    w(base, R_QUEUE_READY, 1);
}

/// Submit a REQ_BUF→RESP_BUF request on the control queue, wait for the device,
/// and return the status code it wrote (SND_S_OK on success).
unsafe fn ctrl_request(base: u64, req_len: u32, resp_len: u32) -> u32 {
    let dt = addr_of_mut!(CTRL_DESC) as *mut Desc;
    write_volatile(
        dt.add(0),
        Desc { addr: addr_of_mut!(REQ_BUF) as u64, len: req_len, flags: F_NEXT, next: 1 },
    );
    write_volatile(
        dt.add(1),
        Desc { addr: addr_of_mut!(RESP_BUF) as u64, len: resp_len, flags: F_WRITE, next: 0 },
    );
    let avail = addr_of_mut!(CTRL_AVAIL);
    let idx = read_volatile(addr_of!((*avail).idx));
    let ring0 = addr_of_mut!((*avail).ring) as *mut u16;
    write_volatile(ring0.add((idx as usize) % QSZ), 0u16); // head descriptor = 0
    dsb();
    write_volatile(addr_of_mut!((*avail).idx), idx.wrapping_add(1));
    dsb();
    let prev = read_volatile(addr_of!(CTRL_USED.idx));
    w(base, R_QUEUE_NOTIFY, 0);
    // Bound the wait to ~50ms of real time (CNTPCT), not a raw spin count: a slow
    // or unresponsive sound control queue must NOT stall the interactive shell for
    // multiple seconds. The normal path completes in microseconds.
    let start = crate::timer::cycles();
    let deadline = (crate::timer::hz() / 20).max(1);
    while read_volatile(addr_of!(CTRL_USED.idx)) == prev {
        if crate::timer::cycles().wrapping_sub(start) > deadline {
            crate::kprintln!("[virtio-sound] control queue timeout");
            return 0;
        }
        core::hint::spin_loop();
    }
    rd32(addr_of_mut!(RESP_BUF) as *const u8, 0)
}

/// VIRTIO_SND_R_PCM_SET_PARAMS for stream 0: stereo S16 @ 44100 Hz.
unsafe fn pcm_set_params(base: u64) -> u32 {
    let rq = addr_of_mut!(REQ_BUF) as *mut u8;
    wr32(rq, 0, R_PCM_SET_PARAMS);
    wr32(rq, 4, 0); // stream_id
    wr32(rq, 8, PERIOD_BYTES * 4); // buffer_bytes (4 periods)
    wr32(rq, 12, PERIOD_BYTES); // period_bytes
    wr32(rq, 16, 0); // features
    write_volatile(rq.add(20), 2u8); // channels
    write_volatile(rq.add(21), FMT_S16);
    write_volatile(rq.add(22), RATE_44100);
    write_volatile(rq.add(23), 0u8); // padding
    ctrl_request(base, 24, 4)
}

/// A bare virtio_snd_pcm_hdr control op (PREPARE/START/STOP) for stream 0.
unsafe fn pcm_ctrl(base: u64, code: u32) -> u32 {
    let rq = addr_of_mut!(REQ_BUF) as *mut u8;
    wr32(rq, 0, code);
    wr32(rq, 4, 0); // stream_id
    ctrl_request(base, 8, 4)
}

/// Fill PCM_BUF with the next `frames` frames of a ~441 Hz square-wave tone.
unsafe fn fill_tone(frames: usize) {
    let buf = addr_of_mut!(PCM_BUF) as *mut u8;
    let half = (RATE_HZ / 441 / 2) as u32; // samples per half-cycle
    for f in 0..frames {
        let s: i16 = if (PHASE / half) & 1 == 0 { 8000 } else { -8000 };
        let b = s.to_le_bytes();
        let off = f * 4;
        write_volatile(buf.add(off), b[0]);
        write_volatile(buf.add(off + 1), b[1]);
        write_volatile(buf.add(off + 2), b[0]); // right channel = left
        write_volatile(buf.add(off + 3), b[1]);
        PHASE = PHASE.wrapping_add(1);
    }
}

// ---- 3.358 margin instrument -------------------------------------------------
// OBSERVABILITY ONLY, and COMPILED OUT of every ordinary build: the whole thing
// is behind `#[cfg(voice_trace)]`, enabled with RUSTFLAGS="--cfg voice_trace",
// so all 687 registry suites build and print exactly what they printed before.
// 3.356 saw ONE 135 ms LATE COMPLETION against a 50 ms cap; one sample cannot
// justify widening a deadline. Every boot already carries 47 SUCCESS-path
// samples of the same wait, so record min/max/max-index and report them once
// per sound_play_pcm. Nothing here touches tx_period's return value, the abort
// arm, `hz/20`, or the `[asili voice]` line.
/// The tx-queue completion deadline, in timer cycles. ONE CONSTANT, chosen at
/// 3.360 against the 3.359 DISTRIBUTION, not against a predicted contention
/// factor (3.116 killed the cost models): the traced instrument measured a max
/// SUCCESS-path wait of 42688 us and 39362 us on two boots that did NOT
/// truncate -- 85.4% and 78.7% of the old 50 ms cap -- and 3.356 caught the
/// device completing at 135 ms, 1 ms after the driver gave up. hz/5 = 200 ms
/// covers that observed completion with ~1.5x margin and the clean-boot max
/// with ~4.7x. `ctrl_request`'s own hz/20 is a DIFFERENT queue and is untouched.
#[inline(always)]
fn tx_deadline(hz: u64) -> u64 {
    (hz / 5).max(1)
}

#[cfg(voice_trace)]
static mut TXW_N: u32 = 0;
#[cfg(voice_trace)]
static mut TXW_MIN: u64 = u64::MAX;
#[cfg(voice_trace)]
static mut TXW_MAX: u64 = 0;
#[cfg(voice_trace)]
static mut TXW_MAXI: u32 = 0;

// 3.475, (B)-ingest observability ONLY: the txw_* block above measures ONE
// period's device wait. It cannot answer 3.474's pre-registered (B) question,
// which is about the WHOLE stream: how many PCM bytes per wall-clock second can
// this guest push into sound_stream_write. Same voice_trace gate, so the
// default build reads no clock and prints nothing -- all 688 green suites grade
// a byte-identical serial log.
#[cfg(voice_trace)]
static mut STRM_START: u64 = 0;
#[cfg(voice_trace)]
static mut STRM_SAMPLES: u64 = 0;

#[cfg(voice_trace)]
unsafe fn strm_reset() {
    STRM_SAMPLES = 0;
    STRM_START = crate::timer::cycles();
}
#[cfg(not(voice_trace))]
unsafe fn strm_reset() {}

#[cfg(voice_trace)]
unsafe fn strm_add(n: usize) {
    STRM_SAMPLES = STRM_SAMPLES.wrapping_add(n as u64);
}
#[cfg(not(voice_trace))]
unsafe fn strm_add(_n: usize) {}

/// Whole-stream ingest report. `bytes` counts the CALLER's mono S16 payload
/// (2 B/sample), which is the quantity a Piper-class producer would have to
/// deliver -- NOT the 4 B/frame stereo the device consumes. The comparison
/// clock is 3.474's: 22050 Hz mono 16-bit = 44100 B/s of real time.
#[cfg(voice_trace)]
unsafe fn strm_report() {
    if STRM_SAMPLES == 0 {
        return;
    }
    let hz = crate::timer::hz().max(1);
    let cyc = crate::timer::cycles().wrapping_sub(STRM_START);
    let bytes = STRM_SAMPLES.wrapping_mul(2);
    let us = cyc.wrapping_mul(1_000_000) / hz;
    // Guard the divide: a stream that measures 0 us is reported as such rather
    // than trapping, and says its own rate is unknown.
    if us == 0 {
        crate::kprintln!(
            "[virtio-sound] stream ingest: {} mono bytes in 0 us (rate UNMEASURED)",
            bytes
        );
        return;
    }
    let bps = bytes.wrapping_mul(1_000_000) / us;
    // x100 fixed point against 44100 B/s -- no float in this kernel yet (3.473).
    let xrt_x100 = bps.wrapping_mul(100) / 44100;
    crate::kprintln!(
        "[virtio-sound] stream ingest: {} mono bytes in {} us = {} B/s = {}.{:02}x realtime (44100 B/s)",
        bytes,
        us,
        bps,
        xrt_x100 / 100,
        xrt_x100 % 100
    );
}
#[cfg(not(voice_trace))]
unsafe fn strm_report() {}

#[cfg(voice_trace)]
unsafe fn txw_reset() {
    TXW_N = 0;
    TXW_MIN = u64::MAX;
    TXW_MAX = 0;
    TXW_MAXI = 0;
}
#[cfg(not(voice_trace))]
unsafe fn txw_reset() {}

/// Called on tx_period's SUCCESS exit only. Takes the start stamp (already in
/// scope) rather than an elapsed value so the default build reads no clock.
#[cfg(voice_trace)]
unsafe fn txw_record(start: u64) {
    let e = crate::timer::cycles().wrapping_sub(start);
    if e < TXW_MIN {
        TXW_MIN = e;
    }
    if e > TXW_MAX {
        TXW_MAX = e;
        TXW_MAXI = TXW_N;
    }
    TXW_N = TXW_N.wrapping_add(1);
}
#[cfg(not(voice_trace))]
unsafe fn txw_record(_start: u64) {}

#[cfg(voice_trace)]
unsafe fn txw_report() {
    if TXW_N == 0 {
        return;
    }
    let hz = crate::timer::hz().max(1);
    let us = |c: u64| c.wrapping_mul(1_000_000) / hz;
    crate::kprintln!(
        "[virtio-sound] tx wait over {} completed periods: min {} us, max {} us at period {} (cap {} us)",
        TXW_N,
        us(TXW_MIN),
        us(TXW_MAX),
        TXW_MAXI,
        us(tx_deadline(hz))
    );
}
#[cfg(not(voice_trace))]
unsafe fn txw_report() {}

/// Hand one period of PCM_BUF to the device via the tx queue; return its status.
unsafe fn tx_period(base: u64, len: u32) -> u32 {
    wr32(addr_of_mut!(XFER_HDR) as *mut u8, 0, 0); // stream_id 0
    let dt = addr_of_mut!(TX_DESC) as *mut Desc;
    write_volatile(dt.add(0), Desc { addr: addr_of_mut!(XFER_HDR) as u64, len: 4, flags: F_NEXT, next: 1 });
    write_volatile(dt.add(1), Desc { addr: addr_of_mut!(PCM_BUF) as u64, len, flags: F_NEXT, next: 2 });
    write_volatile(dt.add(2), Desc { addr: addr_of_mut!(TX_STATUS) as u64, len: 8, flags: F_WRITE, next: 0 });
    let avail = addr_of_mut!(TX_AVAIL);
    let idx = read_volatile(addr_of!((*avail).idx));
    let ring0 = addr_of_mut!((*avail).ring) as *mut u16;
    write_volatile(ring0.add((idx as usize) % QSZ), 0u16);
    dsb();
    write_volatile(addr_of_mut!((*avail).idx), idx.wrapping_add(1));
    dsb();
    let prev = read_volatile(addr_of!(TX_USED.idx));
    w(base, R_QUEUE_NOTIFY, 2);
    // Bound to real time (see `tx_deadline`): a stalled tx queue must not hang
    // the shell.
    let start = crate::timer::cycles();
    let hz = crate::timer::hz().max(1);
    let deadline = tx_deadline(hz);
    while read_volatile(addr_of!(TX_USED.idx)) == prev {
        if crate::timer::cycles().wrapping_sub(start) > deadline {
            // OBSERVABILITY ONLY (3.347) -- the return value and the abort are
            // UNCHANGED. Reporting the elapsed time AT this arm is tautological:
            // the loop above exits the instant elapsed passes `deadline`, so it
            // can only ever read ~50 ms whether the deadline is the binding
            // constraint or the device is wedged. What DOES discriminate is
            // whether the device ever completes this descriptor at all, so keep
            // watching for a bounded post-mortem window and say which happened.
            let t_out = crate::timer::cycles().wrapping_sub(start);
            // ~1 s total, stated ABSOLUTELY. It used to read `deadline * 20`,
            // which at the old hz/20 was EXACTLY `hz`; writing it as `hz` keeps
            // the wedged-device post-mortem byte-identical to what it was and
            // stops it stretching to 5 s just because the deadline widened.
            let pm = hz;
            let mut landed: u64 = 0;
            while crate::timer::cycles().wrapping_sub(start) < pm {
                if read_volatile(addr_of!(TX_USED.idx)) != prev {
                    landed = crate::timer::cycles().wrapping_sub(start);
                    break;
                }
                core::hint::spin_loop();
            }
            let ms = |c: u64| c.wrapping_mul(1000) / hz;
            if landed != 0 {
                crate::kprintln!(
                    "[virtio-sound] tx queue timeout at {} ms (cap {} ms) -- LATE COMPLETION at {} ms: device alive, deadline is the binding constraint",
                    ms(t_out), ms(deadline), ms(landed));
            } else {
                crate::kprintln!(
                    "[virtio-sound] tx queue timeout at {} ms (cap {} ms) -- NO completion within {} ms: device wedged, deadline innocent",
                    ms(t_out), ms(deadline), ms(pm));
            }
            return 0;
        }
        core::hint::spin_loop();
    }
    txw_record(start);
    rd32(addr_of_mut!(TX_STATUS) as *const u8, 0)
}

/// Play a short test tone through the virtio-sound device — the end-to-end proof
/// that the audio path works (captured by QEMU's `wav` audiodev). Returns false
/// if no device is present or PCM setup was rejected. This is the real signal
/// under Asili Voice until neural TTS (Piper) lands; `speak()` calls it.
pub fn sound_play_tone() -> bool {
    if !sound_present() {
        return false;
    }
    let base = sound_base();
    // SAFETY: exclusive kernel-context access to the device + our vrings.
    unsafe {
        if !ensure_started(base) {
            return false;
        }
        let mut sent = 0u32;
        for _ in 0..16 {
            fill_tone(PERIOD_BYTES as usize / 4);
            if tx_period(base, PERIOD_BYTES) != SND_S_OK {
                break;
            }
            sent += 1;
        }
        crate::kprintln!(
            "[virtio-sound] streamed {} periods (~{} ms) of a 441 Hz tone",
            sent,
            sent * (PERIOD_BYTES / 4) * 1000 / RATE_HZ
        );
        sent > 0
    }
}

/// Stream a mono PCM buffer (signed 16-bit samples) to the sound device,
/// duplicated to both channels. Starts/configures the stream on first use.
/// This is the general playback entry point the Asili Voice synthesizer uses.
pub fn sound_play_pcm(samples: &[i16]) -> bool {
    // One code path: the whole-buffer form is the streaming form with a single
    // write between begin and end (design-3.367 (B), 3.370).
    sound_stream_begin() && sound_stream_write(samples) && sound_stream_end()
}

/// Hand the staged (possibly partial) period to the device. Returns false and
/// names the abort on a non-OK status; the staging buffer is emptied either way.
unsafe fn submit_staged(base: u64) -> bool {
    if PCM_FILL == 0 {
        return true;
    }
    let bytes = (PCM_FILL as u32) * 4;
    PCM_FILL = 0;
    let st = tx_period(base, bytes);
    if st != SND_S_OK {
        // 3.343: this was the ONLY early exit in the submit loop and it was
        // SILENT (say()'s bool is discarded by its callers), so a guest-side
        // abort and a host-side wav truncation are observationally identical.
        // Name it. Prints ONLY on the failing path -- the passing case is
        // byte-identical on serial, so the [asili voice] line all 687 green
        // suites grade is untouched. 3.370: the total period count is no longer
        // knowable in advance (the stream has no end until the caller says so),
        // so the "of N" half is gone -- the index and the bytes already
        // submitted, which is what 3.344 actually read, are both kept.
        crate::kprintln!(
            "[voice] tx_period ABORT at period {} (status {:#x}, {} PCM bytes submitted)",
            PCM_PERIODS,
            st,
            PCM_PERIODS * PERIOD_BYTES as usize
        );
        return false;
    }
    PCM_PERIODS += 1;
    true
}

/// Open a streaming playback session: configure/start the stream once and reset
/// the staging buffer. Pair with [`sound_stream_write`] + [`sound_stream_end`].
pub fn sound_stream_begin() -> bool {
    if !sound_present() {
        return false;
    }
    let base = sound_base();
    // SAFETY: exclusive kernel-context access to the device + our vrings.
    unsafe {
        if !ensure_started(base) {
            return false;
        }
        txw_reset();
        strm_reset();
        PCM_FILL = 0;
        PCM_PERIODS = 0;
        true
    }
}

/// Push mono samples into the open stream, duplicated to both channels. Chunks
/// of ANY length are accepted -- whole periods go to the device as they fill and
/// the remainder is carried in PCM_BUF until the next write or the end of the
/// stream, so the caller never has to know the device's period size.
pub fn sound_stream_write(samples: &[i16]) -> bool {
    if !sound_present() {
        return false;
    }
    let base = sound_base();
    // SAFETY: exclusive kernel-context access to the device + our vrings.
    unsafe {
        let frames_per_period = (PERIOD_BYTES / 4) as usize; // stereo S16
        let buf = addr_of_mut!(PCM_BUF) as *mut u8;
        let mut i = 0usize;
        while i < samples.len() {
            let take = core::cmp::min(frames_per_period - PCM_FILL, samples.len() - i);
            for f in 0..take {
                let s = samples[i + f];
                let b = s.to_le_bytes();
                let off = (PCM_FILL + f) * 4;
                write_volatile(buf.add(off), b[0]);
                write_volatile(buf.add(off + 1), b[1]);
                write_volatile(buf.add(off + 2), b[0]);
                write_volatile(buf.add(off + 3), b[1]);
            }
            PCM_FILL += take;
            i += take;
            if PCM_FILL == frames_per_period && !submit_staged(base) {
                return false;
            }
        }
        strm_add(samples.len());
        true
    }
}

/// Close the stream: flush whatever partial period is staged and report the tx
/// wait distribution for the whole session.
pub fn sound_stream_end() -> bool {
    if !sound_present() {
        return false;
    }
    let base = sound_base();
    // SAFETY: exclusive kernel-context access to the device + our vrings.
    unsafe {
        let ok = submit_staged(base);
        txw_report();
        strm_report();
        ok
    }
}

/// One-time queue setup + PCM stream configuration (SET_PARAMS/PREPARE/START).
/// Returns true once the stream is live and ready to accept tx buffers.
unsafe fn ensure_started(base: u64) -> bool {
    if STARTED {
        return true;
    }
    queue_setup(
        base,
        0,
        addr_of_mut!(CTRL_DESC) as u64,
        addr_of_mut!(CTRL_AVAIL) as u64,
        addr_of_mut!(CTRL_USED) as u64,
    );
    queue_setup(
        base,
        2,
        addr_of_mut!(TX_DESC) as u64,
        addr_of_mut!(TX_AVAIL) as u64,
        addr_of_mut!(TX_USED) as u64,
    );
    let st = r(base, R_STATUS) | S_DRIVER_OK;
    w(base, R_STATUS, st); // device is now live
    let a = pcm_set_params(base);
    let b = pcm_ctrl(base, R_PCM_PREPARE);
    let c = pcm_ctrl(base, R_PCM_START);
    crate::kprintln!("[virtio-sound] set_params={:#x} prepare={:#x} start={:#x}", a, b, c);
    if a != SND_S_OK || b != SND_S_OK || c != SND_S_OK {
        crate::kprintln!("[virtio-sound] PCM setup rejected");
        return false;
    }
    STARTED = true;
    true
}

// ===========================================================================
// virtio-net (device id 1): a minimal L2 driver reusing the split-virtqueue
// machinery above. Sets up the RX (queue 0) and TX (queue 1) rings, reads the
// MAC from config space, and can send an ARP request + receive the reply — proof
// that both directions work end to end. IP/UDP/TCP are later milestones.
// ===========================================================================

const NBUF: usize = QSZ; // RX buffers (one per descriptor)
const VNET_HDR: usize = 12; // virtio_net_hdr (v1) prepended to every packet

static mut NET_BASE: u64 = 0;
static mut NET_MAC: [u8; 6] = [0; 6];
static mut GW_MAC: [u8; 6] = [0; 6]; // gateway MAC, learned via ARP
static mut GW_KNOWN: bool = false;
static mut RX_COUNT: u32 = 0;
/// Frames taken OFF the RX used ring, counted before parse or any filter.
/// `RX_COUNT` is a MATCHED-segment counter (3.516 honest limit); this one is the
/// raw absorption tally, so "absorbed" and "matched for me" stop being one number.
static mut RX_RAW_COUNT: u32 = 0;
/// The SINK loop's two escapes, NAMED (3.521). 3.520 finding (1): `net_tcp_sink`
/// is the pair whose raw→matched span has exactly TWO exits — (A) `parse_tcp`
/// returned `None`, (B) the 4-tuple did not match — so a frame absorbed but not
/// matched there is one of these two and nothing else. HONEST LIMIT: only that
/// one loop increments them; four other raw/matched pairs in this file skip by
/// ENCLOSING `if` rather than `continue` and stay uninstrumented, so
/// `unparsed + offtuple == raw - matched` is an identity over the SINK WINDOW,
/// never over a whole boot. ARP, malformed and short frames are NOT separable
/// here: all three collapse into `parse_tcp`'s single `None` (3.520), which is
/// why this buys a TWO-way split and not the four 3.518 pre-registered.
static mut RX_UNPARSED: u32 = 0;
static mut RX_OFFTUPLE: u32 = 0;
/// The OTHER TWO raw sites inside the sink window, named (3.522 pre-registration).
/// `net_tcp_sink` takes its cursor BEFORE `tcp_accept` ("the ARP is the peer's cue
/// to dial", :3140), so `tcp_accept` and `tcp_close_cur` each dequeue off the SAME
/// ring inside the window the census covers — and each has raw-without-matched
/// escapes that no counter modelled, which is the shape of 3.518/3.521's delta of
/// one. These five are observability ONLY: no `continue` was removed and no filter
/// loosened. With `esc` = the sum of ALL SEVEN escape counters, the pre-committed
/// reading is `esc == raw - matched` (the frame is NAMED) vs `esc < raw - matched`
/// (a frame still escapes every modelled branch).
static mut RX_ACC_UNPARSED: u32 = 0;
static mut RX_ACC_OFFPORT: u32 = 0;
static mut RX_ACC_OFFCONN: u32 = 0;
static mut RX_CLO_UNPARSED: u32 = 0;
static mut RX_CLO_OFFTUPLE: u32 = 0;

/// Number of times a descriptor was OFFERED BACK to the device — the census's
/// missing term `R`, design-3.535 §4 (instrument (B)).
///
/// Why a count and not a cursor: `NET_RX_AVAIL.idx` has exactly two writers,
/// `net_setup`'s one-shot `= NBUF` and `net_rx_recycle`'s `+1`, so `avail` is
/// the identity `NBUF + R`. A used-ring CURSOR would answer a different
/// question — how far the driver has REACHED — and under a double walk of the
/// same element it advances ONCE where `R` advances TWICE, i.e. it reads the
/// suspected defect as absent (design-3.535 §3). The quantity that decides
/// branch (iii) is `R`, so `R` is what is counted.
static mut RX_RECYCLED: u32 = 0;

/// The used-ring high-water mark a NESTED consumer has already taken AND already
/// recycled (design-3.529 §3, candidate (a)).
///
/// `net_arp_probe` consumes one used-ring element itself and drops an `RxSlot`
/// over it, so that element is recycled before the probe returns. A walker whose
/// cursor was sampled BEFORE the ARP (which every listener's must be — 2.759:
/// the ARP is the peer's cue to dial, so a cursor read after it starts past the
/// SYN it provoked) then walks that same element a second time, builds a second
/// `RxSlot` on the same `elem.id`, and recycles it AGAIN. 3.536 MEASURED that
/// second publish: `R == U + 1` on a first-command sink run.
///
/// So the consumer publishes what it consumed through, and the walkers skip
/// exactly that — not by moving `cur0` later (which would give back the SYN this
/// stack spent 2.759 learning to catch), but by stepping over the slots someone
/// else has already accounted for. A frame nobody consumed still reaches the
/// loop.
///
/// 0 is not a sentinel: it is a legitimate index, so a mark is only ever acted
/// on through the wrapping comparison in `rx_skip_consumed`.
static mut RX_CONSUMED_IDX: u16 = 0;

/// Advance a walker's cursor to `RX_CONSUMED_IDX` when — and only when — the
/// mark is genuinely AHEAD of it.
///
/// The comparison is `wrapping_sub(..) as i16 > 0`, not `>`. These are virtio
/// ring indices: they are free-running `u16` that wrap at 65536, so a plain `>`
/// reads a mark of 3 as BEHIND a cursor of 65534 when it is 5 slots ahead, and
/// the skip silently stops happening after the first wrap. Named in
/// design-3.529 §3 so this tick could not get it quietly wrong.
unsafe fn rx_skip_consumed(cur: u16) -> u16 {
    let mark = read_volatile(addr_of!(RX_CONSUMED_IDX));
    if mark.wrapping_sub(cur) as i16 > 0 { mark } else { cur }
}

/// Standard 16-bit ones-complement internet checksum (RFC 1071).
/// Fold one buffer into a running ones-complement (RFC 1071) sum, so a checksum
/// may span a pseudo-header, a segment header and a payload without first
/// copying the three into one contiguous scratch. Every buffer but the LAST must
/// have an EVEN length: an odd tail is zero-padded, and padding is only correct
/// at the very end of the summed data.
fn inet_sum_add(mut sum: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | (data[i + 1] as u32);
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    sum
}

/// Carry-fold a running sum into 16 bits and complement it into the wire value.
fn inet_sum_fold(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn inet_checksum(data: &[u8]) -> u16 {
    inet_sum_fold(inet_sum_add(0, data))
}

static mut NET_RX_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut NET_RX_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut NET_RX_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };
static mut NET_TX_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut NET_TX_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut NET_TX_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };
static mut RX_BUFS: [[u8; 2048]; NBUF] = [[0; 2048]; NBUF];
static mut TX_BUF: [u8; 2048] = [0; 2048];

/// Bring up the virtio-net device: negotiate, read MAC, arm RX, ready TX.
fn net_setup(base: u64) -> bool {
    // SAFETY: MMIO on a detected virtio-net device; single-threaded bring-up.
    unsafe {
        w(base, R_STATUS, 0);
        let mut st = S_ACK;
        w(base, R_STATUS, st);
        st |= S_DRIVER;
        w(base, R_STATUS, st);

        // Accept VIRTIO_NET_F_MAC (bit 5) and VIRTIO_F_VERSION_1 (bit 32).
        w(base, R_DEVICE_FEATURES_SEL, 0);
        let flo = r(base, R_DEVICE_FEATURES);
        w(base, R_DRIVER_FEATURES_SEL, 0);
        w(base, R_DRIVER_FEATURES, flo & (1 << 5));
        w(base, R_DRIVER_FEATURES_SEL, 1);
        w(base, R_DRIVER_FEATURES, 1);
        st |= S_FEATURES_OK;
        w(base, R_STATUS, st);
        if r(base, R_STATUS) & S_FEATURES_OK == 0 {
            crate::kprintln!("[virtio-net] FEATURES_OK rejected");
            w(base, R_STATUS, S_FAILED);
            return false;
        }

        for i in 0..6 {
            NET_MAC[i] = read_volatile((base + R_CONFIG + i as u64) as *const u8);
        }

        // RX queue 0: point at our ring, then fill every descriptor with a
        // device-writable buffer and publish them all.
        queue_setup(
            base,
            0,
            addr_of_mut!(NET_RX_DESC) as u64,
            addr_of_mut!(NET_RX_AVAIL) as u64,
            addr_of_mut!(NET_RX_USED) as u64,
        );
        let rxd = addr_of_mut!(NET_RX_DESC) as *mut Desc;
        let rring = addr_of_mut!(NET_RX_AVAIL.ring) as *mut u16;
        for i in 0..NBUF {
            write_volatile(
                rxd.add(i),
                Desc { addr: addr_of_mut!(RX_BUFS[i]) as u64, len: 2048, flags: F_WRITE, next: 0 },
            );
            write_volatile(rring.add(i), i as u16);
        }
        dsb();
        write_volatile(addr_of_mut!(NET_RX_AVAIL.idx), NBUF as u16);
        dsb();

        // TX queue 1: ring only (buffers are supplied per send).
        queue_setup(
            base,
            1,
            addr_of_mut!(NET_TX_DESC) as u64,
            addr_of_mut!(NET_TX_AVAIL) as u64,
            addr_of_mut!(NET_TX_USED) as u64,
        );

        st |= S_DRIVER_OK;
        w(base, R_STATUS, st);
        w(base, R_QUEUE_NOTIFY, 0); // tell the device RX buffers are ready

        NET_BASE = base;
        crate::kprintln!(
            "[virtio-net] up @ {:#010x}, MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}, {} rx buffers",
            base, NET_MAC[0], NET_MAC[1], NET_MAC[2], NET_MAC[3], NET_MAC[4], NET_MAC[5], NBUF
        );
        true
    }
}

pub fn net_present() -> bool {
    // SAFETY: scalar read.
    unsafe { NET_BASE != 0 }
}
pub fn net_mac() -> [u8; 6] {
    // SAFETY: written once at setup.
    unsafe { NET_MAC }
}
pub fn net_rx_count() -> u32 {
    // SAFETY: scalar read.
    unsafe { RX_COUNT }
}
pub fn net_rx_raw_count() -> u32 {
    // SAFETY: scalar read.
    unsafe { RX_RAW_COUNT }
}
pub fn net_rx_unparsed() -> u32 {
    // SAFETY: scalar read.
    unsafe { RX_UNPARSED }
}
pub fn net_rx_offtuple() -> u32 {
    // SAFETY: scalar read.
    unsafe { RX_OFFTUPLE }
}
/// The 3.522 sites: `tcp_accept`'s three raw-without-matched escapes and
/// `tcp_close_cur`'s two. SAFETY on each: scalar read.
pub fn net_rx_acc_unparsed() -> u32 { unsafe { RX_ACC_UNPARSED } }
pub fn net_rx_acc_offport() -> u32 { unsafe { RX_ACC_OFFPORT } }
pub fn net_rx_acc_offconn() -> u32 { unsafe { RX_ACC_OFFCONN } }
pub fn net_rx_clo_unparsed() -> u32 { unsafe { RX_CLO_UNPARSED } }
pub fn net_rx_clo_offtuple() -> u32 { unsafe { RX_CLO_OFFTUPLE } }

/// The RX ring's two published indices, for design-3.529 §4.2 — the arm that
/// decides whether the double-walk is a COUNTER defect or a real DOUBLE
/// RECYCLE. `avail.idx` counts every buffer this driver has offered the device
/// (it starts at `NBUF`, `:949`); `used.idx` counts every frame the device has
/// handed back. Each delivered frame that is consumed exactly once costs one
/// recycle, so `avail - used` can never EXCEED `NBUF`; a value above it is a
/// buffer published twice. Observability only: nothing here is written, and
/// `net_rx_recycle` is unchanged.
///
/// SAFETY on both: volatile scalar read of a driver-owned ring header.
pub fn net_rx_avail_idx() -> u16 { unsafe { read_volatile(addr_of!(NET_RX_AVAIL.idx)) } }
pub fn net_rx_used_idx() -> u16 { unsafe { read_volatile(addr_of!(NET_RX_USED.idx)) } }

/// ONE-PLACE SNAPSHOT of the pair, with `used` read FIRST (finding 3.531's
/// pre-registered step). The census used to take these as two separate reads
/// inside one `kprintln!` argument list. `avail` is written only by the DRIVER
/// (this core, and not during the census), but `used` is written by the DEVICE
/// at any instant -- so a frame handed back BETWEEN the two reads raised `used`
/// alone and made `avail - used` read one LOW. Reading `used` FIRST cannot make
/// that error in the same direction: a hand-back landing between the reads now
/// leaves the older, lower `used` beside the same `avail`, so the pair errs
/// HIGH. The sign of any remaining race is therefore REVERSED, which is exactly
/// what 3.531 pre-committed to reading.
///
/// Observability only: nothing is written here, no counter, cursor or
/// `net_rx_recycle` path changes, and the caller's printed line is unchanged.
///
/// 3.535 step (instrument (B)): a THIRD value, `RX_RECYCLED` = the number of
/// completed `net_rx_recycle` calls (`R`). Unlike `used`, this one cannot skew
/// against `avail`: BOTH are written only by the driver, on this core, and the
/// census is itself driver code — so no recycle can land between the two reads
/// and `avail - NBUF == R` is a claim about the code, not about timing. The two
/// existing fields and their read order are UNCHANGED.
pub fn net_rx_ring_snapshot() -> (u16, u16, u32) {
    let used = net_rx_used_idx();
    let avail = net_rx_avail_idx();
    // SAFETY: single-core kernel context; a plain read of our own counter.
    let recycled = unsafe { read_volatile(addr_of!(RX_RECYCLED)) };
    (avail, used, recycled)
}

/// Offer RX descriptor `id` back to the device.
///
/// The avail ring is the driver's half of the contract: a descriptor the device
/// has filled and handed back in the USED ring is ours until we re-publish it,
/// and until we do, it is one fewer place an inbound frame can land. The
/// descriptor table entry is rewritten rather than assumed — the buffer address,
/// the full 2048-byte length and F_WRITE are what make it usable again, and a
/// device that wrote a short frame does not restore the length we advertised.
unsafe fn net_rx_recycle(id: u32) {
    if NET_BASE == 0 {
        return;
    }
    let i = (id as usize) % NBUF;
    let rxd = addr_of_mut!(NET_RX_DESC) as *mut Desc;
    write_volatile(
        rxd.add(i),
        Desc { addr: addr_of_mut!(RX_BUFS[i]) as u64, len: 2048, flags: F_WRITE, next: 0 },
    );
    let idx = read_volatile(addr_of!(NET_RX_AVAIL.idx));
    let ring = addr_of_mut!(NET_RX_AVAIL.ring) as *mut u16;
    write_volatile(ring.add((idx as usize) % QSZ), i as u16);
    dsb();
    write_volatile(addr_of_mut!(NET_RX_AVAIL.idx), idx.wrapping_add(1));
    dsb();
    w(NET_BASE, R_QUEUE_NOTIFY, 0);
    // design-3.535 §4: count the publish, not the entry. Deliberately AFTER the
    // `NET_BASE == 0` early return above — a suppressed recycle raises no
    // `avail` and must not be counted as one, or the identity it is here to
    // test would be broken by the instrument itself.
    RX_RECYCLED = RX_RECYCLED.wrapping_add(1);
}

/// A received RX descriptor, held for exactly as long as the frame in its buffer
/// is being read and returned to the device when it goes out of scope.
///
/// Why a guard and not a call at the end of each loop: the RX ring is a fixed
/// pool of `NBUF` buffers, and `net_setup` published all of them once. Nothing
/// ever gave one back, so a boot had a LIFETIME budget of `NBUF` inbound frames
/// — measured, not feared: ten pings in one boot answered five, the driver's
/// `rx frames` counter froze at 8, and the NIC stayed deaf until reboot. The
/// device was fine; we had simply stopped offering it anywhere to put the next
/// frame. (The same shape as 2.749's UART deafness.)
///
/// Recycling must happen on EVERY path out of a frame, exactly once: twice
/// publishes one buffer into two ring slots, and never publishing is the bug
/// above. These loops leave a frame by `continue` from several places and by
/// `return` from several more, so "exactly once, on every path" is not a rule to
/// audit by eye — it is precisely what scope exit means, which is what `Drop`
/// is. The guard is created straight after the used-ring element is read and
/// lives to the end of that frame's block, so the buffer is also still off the
/// device's free list while it is being parsed.
struct RxSlot(u32);

impl Drop for RxSlot {
    fn drop(&mut self) {
        // SAFETY: the id came out of a used-ring element, so it indexes our own
        // descriptor table, and the frame that occupied it is finished with.
        unsafe { net_rx_recycle(self.0) }
    }
}

/// Send an ARP request for the SLIRP gateway (10.0.2.2) and wait for the reply —
/// proves TX and RX both work. Returns true if a frame was received.
pub fn net_arp_probe() -> bool {
    if !net_present() {
        return false;
    }
    // SAFETY: exclusive kernel-context access to the device + our rings/buffers.
    unsafe {
        let base = NET_BASE;
        // Capture the RX used index BEFORE sending — the reply can arrive almost
        // instantly, so baselining after the send would miss it.
        let base_rx = read_volatile(addr_of!(NET_RX_USED.idx));
        let tx = addr_of_mut!(TX_BUF) as *mut u8;
        for i in 0..VNET_HDR {
            *tx.add(i) = 0; // zero virtio_net_hdr
        }
        let f = tx.add(VNET_HDR); // ethernet frame
        for i in 0..6 {
            *f.add(i) = 0xff; // dest: broadcast
            *f.add(6 + i) = NET_MAC[i]; // src: us
        }
        *f.add(12) = 0x08;
        *f.add(13) = 0x06; // ethertype ARP
        let arp: [u8; 28] = [
            0x00, 0x01, 0x08, 0x00, 6, 4, 0x00, 0x01, // htype/ptype/hlen/plen/op=request
            NET_MAC[0], NET_MAC[1], NET_MAC[2], NET_MAC[3], NET_MAC[4], NET_MAC[5], // sender MAC
            10, 0, 2, 15, // sender IP 10.0.2.15
            0, 0, 0, 0, 0, 0, // target MAC (unknown)
            10, 0, 2, 2, // target IP 10.0.2.2 (gateway)
        ];
        for (i, &b) in arp.iter().enumerate() {
            *f.add(14 + i) = b;
        }
        let total = (VNET_HDR + 14 + 28) as u32;

        let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
        write_volatile(txd.add(0), Desc { addr: addr_of_mut!(TX_BUF) as u64, len: total, flags: 0, next: 0 });
        let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
        let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
        write_volatile(tring.add((tidx as usize) % QSZ), 0);
        dsb();
        write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
        dsb();
        w(base, R_QUEUE_NOTIFY, 1);
        crate::kprintln!("[virtio-net] sent ARP who-has 10.0.2.2");

        // Wait for a NEW frame (rx used index advances past the baseline).
        let mut spins = 0u64;
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != base_rx {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((base_rx as usize) % QSZ));
                RX_RAW_COUNT += 1;
                let _slot = RxSlot(elem.id);
                // design-3.529 §3(a): publish the high-water mark this consumer
                // took, so a walker holding a cursor sampled before this ARP does
                // not walk — and re-recycle — the element we are about to drop an
                // `RxSlot` over. Written where the element is TAKEN, not where the
                // function returns: the recycle is the `RxSlot` drop below, and a
                // mark written after an early `return` path would be missed.
                write_volatile(addr_of_mut!(RX_CONSUMED_IDX), base_rx.wrapping_add(1));
                let frame = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let et = ((*frame.add(12) as u16) << 8) | (*frame.add(13) as u16);
                RX_COUNT += 1;
                crate::kprintln!(
                    "[virtio-net] RX {} bytes, ethertype {:#06x}, src {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                    (elem.len as usize).saturating_sub(VNET_HDR), et,
                    *frame.add(6), *frame.add(7), *frame.add(8), *frame.add(9), *frame.add(10), *frame.add(11)
                );
                if et == 0x0806 {
                    // Learn the gateway MAC from the ARP reply's sender field
                    // (Ethernet 14 + ARP sender-hardware-address offset 8 = 22).
                    for i in 0..6 {
                        GW_MAC[i] = *frame.add(22 + i);
                    }
                    GW_KNOWN = true;
                    crate::kprintln!("[virtio-net] -> ARP reply received (TX + RX round-trip OK)");
                }
                return true;
            }
            spins += 1;
            if spins > 60_000_000 {
                crate::kprintln!("[virtio-net] no reply within timeout");
                return false;
            }
        }
    }
}

/// Ping the SLIRP gateway (10.0.2.2) with an ICMP echo request over IPv4 and wait
/// for the echo reply. Learns the gateway MAC via ARP first if needed. This is raw
/// ICMP over a hardcoded SLIRP config, not a general IP stack (no ARP cache/DHCP/
/// routing yet) — see docs/07.
pub fn net_ping() -> bool {
    if !net_present() {
        return false;
    }
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        if !GW_KNOWN {
            net_arp_probe(); // learn the gateway MAC
        }
        if !GW_KNOWN {
            crate::kprintln!("[net] ping: could not resolve the gateway MAC");
            return false;
        }
        let base = NET_BASE;

        // ICMP echo request (8 bytes): type 8, code 0, checksum, id, seq.
        let mut icmp = [0u8; 8];
        icmp[0] = 8;
        icmp[4] = 0x13;
        icmp[5] = 0x37;
        icmp[7] = 0x01;
        let ck = inet_checksum(&icmp);
        icmp[2] = (ck >> 8) as u8;
        icmp[3] = ck as u8;

        // IPv4 header (20 bytes).
        let mut ip = [0u8; 20];
        ip[0] = 0x45; // version 4, IHL 5
        let total: u16 = 20 + 8;
        ip[2] = (total >> 8) as u8;
        ip[3] = total as u8;
        ip[8] = 64; // TTL
        ip[9] = 1; // protocol ICMP
        ip[12] = 10; ip[13] = 0; ip[14] = 2; ip[15] = 15; // src 10.0.2.15
        ip[16] = 10; ip[17] = 0; ip[18] = 2; ip[19] = 2; // dst 10.0.2.2
        let ipck = inet_checksum(&ip);
        ip[10] = (ipck >> 8) as u8;
        ip[11] = ipck as u8;

        // Assemble Ethernet + IPv4 + ICMP into TX_BUF (after the virtio_net_hdr).
        let tx = addr_of_mut!(TX_BUF) as *mut u8;
        for i in 0..VNET_HDR {
            *tx.add(i) = 0;
        }
        let f = tx.add(VNET_HDR);
        for i in 0..6 {
            *f.add(i) = GW_MAC[i];
            *f.add(6 + i) = NET_MAC[i];
        }
        *f.add(12) = 0x08;
        *f.add(13) = 0x00; // ethertype IPv4
        for i in 0..20 {
            *f.add(14 + i) = ip[i];
        }
        for i in 0..8 {
            *f.add(34 + i) = icmp[i];
        }
        let frame_len = 14 + 20 + 8;

        let base_rx = read_volatile(addr_of!(NET_RX_USED.idx)); // baseline before send
        let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
        write_volatile(
            txd.add(0),
            Desc { addr: addr_of_mut!(TX_BUF) as u64, len: (VNET_HDR + frame_len) as u32, flags: 0, next: 0 },
        );
        let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
        let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
        write_volatile(tring.add((tidx as usize) % QSZ), 0);
        dsb();
        write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
        dsb();
        w(base, R_QUEUE_NOTIFY, 1);
        crate::kprintln!("[net] ping 10.0.2.2 (ICMP echo request sent)");

        // Consume RX frames until we see the ICMP echo reply or time out.
        let mut cur = base_rx;
        let mut spins = 0u64;
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                spins = 0;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let et = ((*fr.add(12) as u16) << 8) | (*fr.add(13) as u16);
                if et == 0x0800 {
                    let proto = *fr.add(14 + 9);
                    let ihl = (*fr.add(14) & 0x0f) as usize * 4;
                    let icmp_type = *fr.add(14 + ihl);
                    if proto == 1 && icmp_type == 0 {
                        RX_COUNT += 1;
                        crate::kprintln!("[net] ICMP echo reply from 10.0.2.2 (ping OK)");
                        return true;
                    }
                }
            }
            spins += 1;
            if spins > 60_000_000 {
                crate::kprintln!("[net] ping: no echo reply within timeout");
                return false;
            }
        }
    }
}

/// What one DNS query came back with.
///
/// THREE states, not a `bool` and not an `Option`, because the two non-answers
/// are different things and a green suite depends on the difference: a response
/// carrying only CNAMEs still completed a UDP round trip, and `net-test.sh:35`
/// greps the "UDP round-trip OK" line that says so. Folding "the resolver
/// answered, with no address" into "the resolver did not answer" would turn a
/// permissive stack into a red suite (design-2.767 §2(c)).
enum DnsOutcome {
    /// Nothing came back within the timeout (or the rig could not send).
    NoResponse,
    /// A response with >= 1 answer arrived; `Some(ip)` if one answer was an A.
    Answered(Option<[u8; 4]>),
}

unsafe fn dns_be16(d: *const u8, o: usize) -> u16 {
    ((*d.add(o) as u16) << 8) | (*d.add(o + 1) as u16)
}

/// Advance past a DNS name: a run of length-prefixed labels, or a compression
/// pointer (top two bits set) which is two bytes and ends the name.
unsafe fn dns_skip_name(d: *const u8, len: usize, mut off: usize) -> usize {
    while off < len {
        let b = *d.add(off);
        if b == 0 {
            return off + 1;
        }
        if b & 0xc0 == 0xc0 {
            return off + 2;
        }
        off += 1 + b as usize;
    }
    off
}

/// The FIRST A record (type 1, rdlength 4) in a DNS response, or `None`.
///
/// `None` is a real answer, not an error: a resolver that hands back a CNAME
/// chain has resolved nothing this stack can dial, and the caller says so.
/// Mirrors `scripts/dest-parse-2767.py:dns_a_records`, which reads the same
/// bytes off the wire — the probe and the kernel must agree about what arrived
/// or the measurement is measuring the parser.
unsafe fn dns_first_a(d: *const u8, len: usize) -> Option<[u8; 4]> {
    if len < 12 {
        return None;
    }
    let qd = dns_be16(d, 4) as usize;
    let an = dns_be16(d, 6) as usize;
    let mut off = 12usize;
    for _ in 0..qd {
        off = dns_skip_name(d, len, off) + 4; // + QTYPE + QCLASS
    }
    for _ in 0..an {
        off = dns_skip_name(d, len, off);
        if off + 10 > len {
            return None;
        }
        let rtype = dns_be16(d, off);
        let rdlen = dns_be16(d, off + 8) as usize;
        let rdata = off + 10;
        if rdata + rdlen > len {
            return None;
        }
        if rtype == 1 && rdlen == 4 {
            return Some([*d.add(rdata), *d.add(rdata + 1), *d.add(rdata + 2), *d.add(rdata + 3)]);
        }
        off = rdata + rdlen;
    }
    None
}

/// Encode `name` as a DNS QNAME (length-prefixed labels + a zero) into `out`,
/// returning the bytes written. `None` for anything malformed or too long —
/// a name is caller-supplied text now, so this is the only place that gets to
/// decide it is not a name.
fn dns_encode_name(name: &str, out: &mut [u8]) -> Option<usize> {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 200 {
        return None;
    }
    let mut n = 0usize;
    let mut start = 0usize;
    let mut i = 0usize;
    loop {
        let at_end = i == b.len();
        if at_end || b[i] == b'.' {
            let l = i - start;
            if at_end && l == 0 && n > 0 {
                break; // a trailing dot: "example.com." is the same name
            }
            if l == 0 || l > 63 {
                return None;
            }
            if n + 1 + l + 1 > out.len() {
                return None;
            }
            out[n] = l as u8;
            out[n + 1..n + 1 + l].copy_from_slice(&b[start..i]);
            n += 1 + l;
            start = i + 1;
            if at_end {
                break;
            }
        }
        i += 1;
    }
    out[n] = 0;
    Some(n + 1)
}

/// Resolve `name` via a raw DNS A-record query to SLIRP's built-in DNS
/// (10.0.2.3:53) over UDP. Reuses ARP (gateway MAC) + IPv4. UDP checksum is 0
/// (legal/optional for IPv4). Minimal DNS — not a resolver.
///
/// Before 2.767 this walked the response far enough to COUNT the answers and
/// then returned `bool`: the address was in the datagram it had just parsed and
/// was thrown away (measured, not read out of the source —
/// `docs/evidence-2.767-dest-probe.log`). It now hands back what it found.
fn dns_query(name: &str) -> DnsOutcome {
    if !net_present() {
        return DnsOutcome::NoResponse;
    }
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        if !GW_KNOWN {
            net_arp_probe();
        }
        if !GW_KNOWN {
            crate::kprintln!("[net] dns: could not resolve the gateway MAC");
            return DnsOutcome::NoResponse;
        }
        let base = NET_BASE;
        let dns_id: u16 = 0x1234;

        // DNS query message: 12-byte header + question(<name>, A, IN).
        let mut dns = [0u8; 256];
        dns[0] = (dns_id >> 8) as u8;
        dns[1] = dns_id as u8;
        dns[2] = 0x01; // flags: recursion desired
        dns[5] = 1; // qdcount = 1
        let qlen = match dns_encode_name(name, &mut dns[12..]) {
            Some(l) => l,
            None => {
                crate::kprintln!("[net] dns: '{}' is not a name this stack can ask about", name);
                return DnsOutcome::NoResponse;
            }
        };
        let mut n = 12 + qlen;
        dns[n + 1] = 1; // QTYPE = A
        dns[n + 3] = 1; // QCLASS = IN
        n += 4;
        let dns_len = n;

        let udp_len = 8 + dns_len;
        let ip_total = 20 + udp_len;
        let mut ip = [0u8; 20];
        ip[0] = 0x45;
        ip[2] = (ip_total >> 8) as u8;
        ip[3] = ip_total as u8;
        ip[8] = 64;
        ip[9] = 17; // UDP
        ip[12] = 10; ip[13] = 0; ip[14] = 2; ip[15] = 15; // src 10.0.2.15
        ip[16] = 10; ip[17] = 0; ip[18] = 2; ip[19] = 3; // dst 10.0.2.3 (SLIRP DNS)
        let ipck = inet_checksum(&ip);
        ip[10] = (ipck >> 8) as u8;
        ip[11] = ipck as u8;

        let tx = addr_of_mut!(TX_BUF) as *mut u8;
        for i in 0..VNET_HDR {
            *tx.add(i) = 0;
        }
        let f = tx.add(VNET_HDR);
        for i in 0..6 {
            *f.add(i) = GW_MAC[i];
            *f.add(6 + i) = NET_MAC[i];
        }
        *f.add(12) = 0x08;
        *f.add(13) = 0x00; // IPv4
        for i in 0..20 {
            *f.add(14 + i) = ip[i];
        }
        // UDP header at offset 34.
        *f.add(34) = 0x13; // src port 5000
        *f.add(35) = 0x88;
        *f.add(36) = 0x00; // dst port 53
        *f.add(37) = 0x35;
        *f.add(38) = (udp_len >> 8) as u8;
        *f.add(39) = udp_len as u8;
        *f.add(40) = 0; // checksum 0 (optional for IPv4 UDP)
        *f.add(41) = 0;
        for i in 0..dns_len {
            *f.add(42 + i) = dns[i];
        }
        let frame_len = 14 + 20 + 8 + dns_len;

        let base_rx = read_volatile(addr_of!(NET_RX_USED.idx));
        let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
        write_volatile(
            txd.add(0),
            Desc { addr: addr_of_mut!(TX_BUF) as u64, len: (VNET_HDR + frame_len) as u32, flags: 0, next: 0 },
        );
        let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
        let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
        write_volatile(tring.add((tidx as usize) % QSZ), 0);
        dsb();
        write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
        dsb();
        w(base, R_QUEUE_NOTIFY, 1);
        crate::kprintln!("[net] dns query {} -> 10.0.2.3:53 (UDP)", name);

        let mut cur = base_rx;
        let mut spins = 0u64;
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                spins = 0;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let et = ((*fr.add(12) as u16) << 8) | (*fr.add(13) as u16);
                if et == 0x0800 && *fr.add(14 + 9) == 17 {
                    let ihl = (*fr.add(14) & 0x0f) as usize * 4;
                    let sport = ((*fr.add(14 + ihl) as u16) << 8) | (*fr.add(14 + ihl + 1) as u16);
                    if sport == 53 {
                        let doff = 14 + ihl + 8;
                        let rid = ((*fr.add(doff) as u16) << 8) | (*fr.add(doff + 1) as u16);
                        let ancount = ((*fr.add(doff + 6) as u16) << 8) | (*fr.add(doff + 7) as u16);
                        if rid == dns_id && ancount >= 1 {
                            RX_COUNT += 1;
                            // Byte-identical to the pre-2.767 line when `name` is
                            // "example.com" — green suite `net-test` greps the
                            // "UDP round-trip OK" fragment of it (net-test.sh:35).
                            crate::kprintln!(
                                "[net] DNS response for {} received (UDP round-trip OK), {} answer(s)",
                                name, ancount
                            );
                            // The response's own length, off the IP header — the
                            // answer section is only walkable inside the bytes that
                            // actually arrived.
                            let ip_total = ((*fr.add(16) as usize) << 8) | (*fr.add(17) as usize);
                            let a = if ip_total >= ihl + 8 {
                                dns_first_a(fr.add(doff), ip_total - ihl - 8)
                            } else {
                                None
                            };
                            match a {
                                Some(ip) => crate::kprintln!(
                                    "[net] DNS A {}.{}.{}.{} for {}",
                                    ip[0], ip[1], ip[2], ip[3], name
                                ),
                                None => crate::kprintln!(
                                    "[net] dns: {} answer(s) for {}, none of them an A record",
                                    ancount, name
                                ),
                            }
                            return DnsOutcome::Answered(a);
                        }
                    }
                }
            }
            spins += 1;
            if spins > 60_000_000 {
                crate::kprintln!("[net] dns: no response within timeout");
                return DnsOutcome::NoResponse;
            }
        }
    }
}

/// `net dns` — resolve `example.com` and report whether the round trip
/// completed. The pre-2.767 signature, kept verbatim because `nia.rs:2447`
/// gates a WASM host call on it and suite `net-test` greps its output.
pub fn net_dns() -> bool {
    matches!(dns_query("example.com"), DnsOutcome::Answered(_))
}

/// Resolve `name` to a single IPv4 address, or `None` if the resolver did not
/// answer or answered without an A record. This is step (c) of 2.767: the
/// address the kernel resolved is now something a CALLER can be handed, which
/// is what turns `net tcp send <host>:<port>` into a destination by name.
pub fn net_dns_resolve(name: &str) -> Option<[u8; 4]> {
    match dns_query(name) {
        DnsOutcome::Answered(a) => a,
        DnsOutcome::NoResponse => None,
    }
}

// ---------------------------------------------------------------------------
// DHCP client: acquire an IP lease from the network's DHCP server (QEMU SLIRP
// offers 10.0.2.15 / gw 10.0.2.2 / dns 10.0.2.3). Four-packet exchange over UDP
// (client port 68 -> server port 67), broadcast, reusing the same virtio-net TX/
// RX path as ping/DNS. Fixed xid, no RNG needed. UDP checksum 0 (legal on IPv4).
// ---------------------------------------------------------------------------

static mut DHCP_IP: [u8; 4] = [0; 4]; // our leased address (yiaddr)
static mut DHCP_MASK: [u8; 4] = [0; 4];
static mut DHCP_GW: [u8; 4] = [0; 4];
static mut DHCP_DNS: [u8; 4] = [0; 4];
static mut DHCP_SRV: [u8; 4] = [0; 4]; // DHCP server id (option 54)
static mut DHCP_HAVE: bool = false;

const DHCP_XID: u32 = 0x3903_F326; // fixed, nonzero transaction id

/// Build a DHCP/BOOTP message at `out`, returning its length. `msgtype` is 1
/// (DISCOVER) or 3 (REQUEST); REQUEST adds option 50 (requested IP) + 54 (server).
unsafe fn build_dhcp(
    out: *mut u8,
    msgtype: u8,
    req_ip: Option<[u8; 4]>,
    srv_id: Option<[u8; 4]>,
) -> usize {
    for i in 0..240 {
        *out.add(i) = 0;
    }
    *out.add(0) = 1; // op = BOOTREQUEST
    *out.add(1) = 1; // htype = Ethernet
    *out.add(2) = 6; // hlen
    *out.add(4) = (DHCP_XID >> 24) as u8;
    *out.add(5) = (DHCP_XID >> 16) as u8;
    *out.add(6) = (DHCP_XID >> 8) as u8;
    *out.add(7) = DHCP_XID as u8;
    *out.add(10) = 0x80; // flags: broadcast (server broadcasts its reply back)
    for i in 0..6 {
        *out.add(28 + i) = NET_MAC[i]; // chaddr = our MAC
    }
    // Magic cookie (99.130.83.99) then options.
    *out.add(236) = 0x63;
    *out.add(237) = 0x82;
    *out.add(238) = 0x53;
    *out.add(239) = 0x63;
    let mut o = 240;
    *out.add(o) = 53; // option 53: DHCP message type
    *out.add(o + 1) = 1;
    *out.add(o + 2) = msgtype;
    o += 3;
    if let Some(ip) = req_ip {
        *out.add(o) = 50; // requested IP address
        *out.add(o + 1) = 4;
        for i in 0..4 {
            *out.add(o + 2 + i) = ip[i];
        }
        o += 6;
    }
    if let Some(sid) = srv_id {
        *out.add(o) = 54; // server identifier
        *out.add(o + 1) = 4;
        for i in 0..4 {
            *out.add(o + 2 + i) = sid[i];
        }
        o += 6;
    }
    *out.add(o) = 55; // parameter request list
    *out.add(o + 1) = 3;
    *out.add(o + 2) = 1; // subnet mask
    *out.add(o + 3) = 3; // router
    *out.add(o + 4) = 6; // DNS
    o += 5;
    *out.add(o) = 255; // end
    o + 1
}

/// Wrap a freshly built DHCP payload in Ethernet(broadcast)/IPv4/UDP and transmit.
unsafe fn send_dhcp(base: u64, msgtype: u8, req_ip: Option<[u8; 4]>, srv_id: Option<[u8; 4]>) {
    let tx = addr_of_mut!(TX_BUF) as *mut u8;
    for i in 0..VNET_HDR {
        *tx.add(i) = 0;
    }
    let f = tx.add(VNET_HDR);
    // Build the DHCP payload first (at offset 42) so we know its length.
    let dhcp_len = build_dhcp(f.add(42), msgtype, req_ip, srv_id);
    let udp_len = 8 + dhcp_len;
    let ip_total = 20 + udp_len;

    // Ethernet: dst broadcast, src us, IPv4.
    for i in 0..6 {
        *f.add(i) = 0xff;
        *f.add(6 + i) = NET_MAC[i];
    }
    *f.add(12) = 0x08;
    *f.add(13) = 0x00;

    // IPv4 header (src 0.0.0.0 -> dst 255.255.255.255).
    let mut ip = [0u8; 20];
    ip[0] = 0x45;
    ip[2] = (ip_total >> 8) as u8;
    ip[3] = ip_total as u8;
    ip[8] = 64;
    ip[9] = 17; // UDP
    ip[16] = 255;
    ip[17] = 255;
    ip[18] = 255;
    ip[19] = 255;
    let ipck = inet_checksum(&ip);
    ip[10] = (ipck >> 8) as u8;
    ip[11] = ipck as u8;
    for i in 0..20 {
        *f.add(14 + i) = ip[i];
    }

    // UDP header: src 68, dst 67, checksum 0.
    *f.add(34) = 0;
    *f.add(35) = 68;
    *f.add(36) = 0;
    *f.add(37) = 67;
    *f.add(38) = (udp_len >> 8) as u8;
    *f.add(39) = udp_len as u8;
    *f.add(40) = 0;
    *f.add(41) = 0;

    let frame_len = 14 + 20 + 8 + dhcp_len;
    let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
    write_volatile(
        txd.add(0),
        Desc { addr: addr_of_mut!(TX_BUF) as u64, len: (VNET_HDR + frame_len) as u32, flags: 0, next: 0 },
    );
    let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
    let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
    write_volatile(tring.add((tidx as usize) % QSZ), 0);
    dsb();
    write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
    dsb();
    w(base, R_QUEUE_NOTIFY, 1);
}

/// Parse a received DHCP reply frame `fr` (Ethernet start). Fills the DHCP_*
/// statics (yiaddr + options) and returns the DHCP message type (option 53), or
/// 0 if this is not a DHCP reply for our transaction.
unsafe fn parse_dhcp(fr: *const u8) -> u8 {
    let ihl = (*fr.add(14) & 0x0f) as usize * 4;
    let doff = 14 + ihl + 8; // start of DHCP payload
    // Match our transaction id.
    let xid = ((*fr.add(doff + 4) as u32) << 24)
        | ((*fr.add(doff + 5) as u32) << 16)
        | ((*fr.add(doff + 6) as u32) << 8)
        | (*fr.add(doff + 7) as u32);
    if *fr.add(doff) != 2 || xid != DHCP_XID {
        return 0; // not a BOOTREPLY for us
    }
    for i in 0..4 {
        DHCP_IP[i] = *fr.add(doff + 16 + i); // yiaddr
    }
    // Options begin after the 236-byte BOOTP area + 4-byte magic cookie.
    let mut o = doff + 240;
    let limit = doff + 548; // bound the walk (BOOTP + options < 548)
    let mut msgtype = 0u8;
    while o + 1 < limit {
        let opt = *fr.add(o);
        if opt == 255 {
            break;
        }
        if opt == 0 {
            o += 1;
            continue; // pad
        }
        let len = *fr.add(o + 1) as usize;
        let v = o + 2;
        match opt {
            53 => msgtype = *fr.add(v),
            54 => {
                for i in 0..4 {
                    DHCP_SRV[i] = *fr.add(v + i);
                }
            }
            1 => {
                for i in 0..4 {
                    DHCP_MASK[i] = *fr.add(v + i);
                }
            }
            3 => {
                for i in 0..4 {
                    DHCP_GW[i] = *fr.add(v + i);
                }
            }
            6 => {
                for i in 0..4 {
                    DHCP_DNS[i] = *fr.add(v + i);
                }
            }
            _ => {}
        }
        o += 2 + len;
    }
    msgtype
}

/// Wait (bounded) for a DHCP reply of `want_type` addressed to UDP port 68.
unsafe fn recv_dhcp(base_rx: u16, want_type: u8) -> bool {
    let mut cur = base_rx;
    let mut spins = 0u64;
    loop {
        if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
            let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
            let elem = read_volatile(uring.add((cur as usize) % QSZ));
            cur = cur.wrapping_add(1);
            RX_RAW_COUNT += 1;
            spins = 0;
            let _slot = RxSlot(elem.id);
            let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
            let et = ((*fr.add(12) as u16) << 8) | (*fr.add(13) as u16);
            if et == 0x0800 && *fr.add(14 + 9) == 17 {
                let ihl = (*fr.add(14) & 0x0f) as usize * 4;
                let dport = ((*fr.add(14 + ihl + 2) as u16) << 8) | (*fr.add(14 + ihl + 3) as u16);
                if dport == 68 && parse_dhcp(fr) == want_type {
                    RX_COUNT += 1;
                    return true;
                }
            }
        }
        spins += 1;
        if spins > 60_000_000 {
            return false;
        }
    }
}

/// Run a DHCP DISCOVER/OFFER/REQUEST/ACK exchange and print the acquired lease.
pub fn net_dhcp() -> bool {
    if !net_present() {
        crate::kprintln!("[net] dhcp: no virtio-net device");
        return false;
    }
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        let base = NET_BASE;

        let base_rx = read_volatile(addr_of!(NET_RX_USED.idx));
        send_dhcp(base, 1, None, None); // DISCOVER
        crate::kprintln!("[net] DHCP DISCOVER sent (broadcast, xid {:#010x})", DHCP_XID);
        if !recv_dhcp(base_rx, 2) {
            crate::kprintln!("[net] DHCP: no OFFER within timeout");
            return false;
        }
        let offered = DHCP_IP;
        let server = DHCP_SRV;
        crate::kprintln!(
            "[net] DHCP OFFER: {}.{}.{}.{} (server {}.{}.{}.{})",
            offered[0], offered[1], offered[2], offered[3],
            server[0], server[1], server[2], server[3]
        );

        let base_rx2 = read_volatile(addr_of!(NET_RX_USED.idx));
        send_dhcp(base, 3, Some(offered), Some(server)); // REQUEST
        crate::kprintln!("[net] DHCP REQUEST sent (requesting {}.{}.{}.{})",
            offered[0], offered[1], offered[2], offered[3]);
        if !recv_dhcp(base_rx2, 5) {
            crate::kprintln!("[net] DHCP: no ACK within timeout");
            return false;
        }
        DHCP_HAVE = true;
        crate::kprintln!(
            "[net] DHCP: IP {}.{}.{}.{}, mask {}.{}.{}.{}, gw {}.{}.{}.{}, dns {}.{}.{}.{}",
            DHCP_IP[0], DHCP_IP[1], DHCP_IP[2], DHCP_IP[3],
            DHCP_MASK[0], DHCP_MASK[1], DHCP_MASK[2], DHCP_MASK[3],
            DHCP_GW[0], DHCP_GW[1], DHCP_GW[2], DHCP_GW[3],
            DHCP_DNS[0], DHCP_DNS[1], DHCP_DNS[2], DHCP_DNS[3]
        );
        true
    }
}

/// The DHCP-leased IP address, or 0.0.0.0 if no lease has been acquired.
pub fn dhcp_ip() -> [u8; 4] {
    // SAFETY: scalar read of a static written only by net_dhcp on this core.
    unsafe {
        if DHCP_HAVE {
            DHCP_IP
        } else {
            [0; 4]
        }
    }
}

/// The DHCP-leased gateway, or 0.0.0.0 if no lease has been acquired.
pub fn dhcp_gw() -> [u8; 4] {
    // SAFETY: scalar read of a static written only by net_dhcp on this core.
    unsafe {
        if DHCP_HAVE {
            DHCP_GW
        } else {
            [0; 4]
        }
    }
}

/// The DHCP-leased DNS server, or 0.0.0.0 if no lease has been acquired.
pub fn dhcp_dns() -> [u8; 4] {
    // SAFETY: scalar read of a static written only by net_dhcp on this core.
    unsafe {
        if DHCP_HAVE {
            DHCP_DNS
        } else {
            [0; 4]
        }
    }
}

// ---------------------------------------------------------------------------
// TCP: the active open, and then data on the connection it opens.
//
// The segment layer is a PURE builder (`build_tcp_frame`) with the virtqueue
// submission wrapped around it. That split is the point: the checksum used to
// live inside a function that also drove a virtqueue, so the only way to check
// it was to boot with a network and read a packet capture — a payload of the
// wrong length, or an odd one, was a thing you found on the wire or not at all.
// `net tcp test` now runs the builder against synthetic segments on any boot.
//
// Above it, `tcp_open` performs the three-way handshake, `net_tcp_data` sends a
// payload on the established connection and reads the peer's answer, and
// `tcp_close` performs the ACTIVE CLOSE that ends it — FIN from this side,
// FIN_WAIT_1 → FIN_WAIT_2 → TIME_WAIT — so a connection is finished rather than
// abandoned when the shell command returns. An abandoned connection is not a
// harmless loose end: the peer sits in ESTABLISHED with a read that never
// returns EOF, and the four segments that end a session are exactly where a
// stack that only ever opened one has never had to get its sequence numbers
// right (our FIN consumes a sequence number of its own, on top of the payload).
//
// The data segment is also the first thing this stack RETRANSMITS. Until now
// every segment was sent exactly once, which is only correct on a wire that
// never loses one — and the loss case is not an exotic failure, it is the
// ordinary condition TCP exists to hide. The ACK wait is now armed with a
// retransmit timer read off CNTPCT (`TCP_RTO_MS`, doubled after each attempt,
// `TCP_MAX_RETX` attempts), and a segment nobody acknowledged is sent again
// with its ORIGINAL sequence number, so a peer that missed the first copy
// receives the same bytes at the same place in the stream rather than a hole
// followed by new data.
//
// Still NOT a TCP state machine: one connection at a time, retransmission of
// the data segment ONLY (a lost SYN or FIN is still a dead connection), no
// window management, no congestion control, no reassembly of out-of-order
// segments, no real TIME_WAIT dwell (the state is reached and named, not held
// for 2 MSL), and no listening side.
// ---------------------------------------------------------------------------

/// The largest payload a segment will carry. `TX_BUF` is 2048 bytes and a frame
/// costs 12 (virtio header) + 54 (Ethernet + IPv4 + TCP, no options) on top of
/// the payload, so this is far inside the buffer *and* inside one 1500-byte
/// Ethernet MTU — which matters because nothing here segments or fragments: a
/// payload that did not fit would be a frame the peer never sees, not a second
/// packet.
pub const TCP_MAX_PAYLOAD: usize = 512;

/// Ethernet + IPv4 + TCP headers, none of them carrying options.
const TCP_FRAME_MIN: usize = 14 + 20 + 20;

/// The SLIRP gateway, 10.0.2.2 — until 2.767 this address was not a value at all
/// but a set of literals compiled into `build_tcp_frame`. It is still the only
/// destination the PASSIVE paths use (`net tcp listen` / `net tcp serve` answer
/// whoever dialled them, and behind user-mode networking that is always the
/// gateway), so those call sites name it explicitly rather than inherit it: a
/// reply addressed to the caller's real source IP is a later milestone, and a
/// default parameter would have hidden which paths still cannot choose.
pub const GW_IP: [u8; 4] = [10, 0, 2, 2];

/// Build an Ethernet + IPv4 + TCP frame from 10.0.2.15:`src_port` to
/// `dst_ip`:`dst_port` into `buf`, returning the frame length
/// (`TCP_FRAME_MIN + data.len()`).
///
/// Pure — no MMIO, no device state, nothing but the bytes handed in.
///
/// **`dst_ip` is written TWICE and only one of the writes is visible.** It goes
/// into the IPv4 header, which is what a capture shows, *and* into the TCP
/// pseudo-header, which exists only for the length of a checksum. Move the IP
/// header alone and every segment leaves with a correct-looking destination and
/// a checksum computed over the old one — the peer discards it in silence and
/// the fault presents as "the network is broken" rather than as "the checksum is
/// wrong". The two writes below are deliberately adjacent to the same argument
/// for that reason (design-2.767 §2(a)).
///
/// Returns **0** if the payload is over `TCP_MAX_PAYLOAD` or `buf` cannot hold
/// the frame. Zero rather than a truncated frame: a short write here is a
/// segment whose IP total length disagrees with the bytes behind it, which the
/// peer reads as a checksum failure at best and as somebody else's data at
/// worst. A caller that ignores the 0 transmits nothing.
fn build_tcp_frame(
    buf: &mut [u8],
    dst_mac: &[u8; 6],
    src_mac: &[u8; 6],
    dst_ip: &[u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    data: &[u8],
) -> usize {
    if data.len() > TCP_MAX_PAYLOAD || buf.len() < TCP_FRAME_MIN + data.len() {
        return 0;
    }

    // TCP header (20 bytes, no options). The checksum field stays zero while it
    // is summed, which is what makes the receiver's sum-to-zero check work.
    let mut tcp = [0u8; 20];
    tcp[0] = (src_port >> 8) as u8;
    tcp[1] = src_port as u8;
    tcp[2] = (dst_port >> 8) as u8;
    tcp[3] = dst_port as u8;
    tcp[4] = (seq >> 24) as u8;
    tcp[5] = (seq >> 16) as u8;
    tcp[6] = (seq >> 8) as u8;
    tcp[7] = seq as u8;
    tcp[8] = (ack >> 24) as u8;
    tcp[9] = (ack >> 16) as u8;
    tcp[10] = (ack >> 8) as u8;
    tcp[11] = ack as u8;
    tcp[12] = 0x50; // data offset = 5 32-bit words (20 bytes)
    tcp[13] = flags;
    tcp[14] = 0xff;
    tcp[15] = 0xff; // window

    // Pseudo-header: src, dst, zero, protocol, TCP length — and the TCP length
    // is the header PLUS the payload, the part a data-less implementation never
    // had to get right because 20 was a constant.
    let seg_len = 20 + data.len();
    let mut pseudo = [0u8; 12];
    pseudo[0] = 10;
    pseudo[1] = 0;
    pseudo[2] = 2;
    pseudo[3] = 15; // src 10.0.2.15
    pseudo[4..8].copy_from_slice(dst_ip); // WRITE 1 of 2 — the invisible one
    pseudo[9] = 6; // protocol TCP
    pseudo[10] = (seg_len >> 8) as u8;
    pseudo[11] = seg_len as u8;

    // Summed in three pieces rather than copied into one scratch buffer. Only
    // the LAST piece may have an odd length (the pad byte is only correct at the
    // very end of the summed data) and only the payload can be odd, so the order
    // pseudo → header → data is the order that is allowed to work.
    let mut sum = inet_sum_add(0, &pseudo);
    sum = inet_sum_add(sum, &tcp);
    sum = inet_sum_add(sum, data);
    let tck = inet_sum_fold(sum);
    tcp[16] = (tck >> 8) as u8;
    tcp[17] = tck as u8;

    // IPv4 header (proto 6). Total length covers the payload too.
    let ip_total = 20 + seg_len;
    let mut ip = [0u8; 20];
    ip[0] = 0x45;
    ip[2] = (ip_total >> 8) as u8;
    ip[3] = ip_total as u8;
    ip[8] = 64;
    ip[9] = 6; // TCP
    ip[12] = 10;
    ip[13] = 0;
    ip[14] = 2;
    ip[15] = 15;
    ip[16..20].copy_from_slice(dst_ip); // WRITE 2 of 2 — the visible one
    let ipck = inet_checksum(&ip);
    ip[10] = (ipck >> 8) as u8;
    ip[11] = ipck as u8;

    for i in 0..6 {
        buf[i] = dst_mac[i];
        buf[6 + i] = src_mac[i];
    }
    buf[12] = 0x08;
    buf[13] = 0x00;
    buf[14..34].copy_from_slice(&ip);
    buf[34..54].copy_from_slice(&tcp);
    buf[54..54 + data.len()].copy_from_slice(data);
    TCP_FRAME_MIN + data.len()
}

/// Build a segment with `build_tcp_frame` and hand it to the TX queue. Returns
/// the frame length, or 0 if the builder refused (nothing is transmitted then).
unsafe fn send_tcp_segment(
    base: u64,
    dst_ip: &[u8; 4],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    data: &[u8],
) -> usize {
    let tx = addr_of_mut!(TX_BUF) as *mut u8;
    for i in 0..VNET_HDR {
        *tx.add(i) = 0;
    }
    let frame = core::slice::from_raw_parts_mut(tx.add(VNET_HDR), 2048 - VNET_HDR);
    // Copied out by value: the builder takes plain arrays, so it never holds a
    // reference into a mutable static.
    let dmac = read_volatile(addr_of!(GW_MAC));
    // The field report (PC only) talks to a real home network: its own leased
    // address and the next hop's own MAC, for the length of one report.
    #[cfg(feature = "pc")]
    let route = read_volatile(addr_of!(REPORT_ROUTE));
    #[cfg(feature = "pc")]
    let dmac = match route {
        Some((_, m)) => m,
        None => dmac,
    };
    let smac = read_volatile(addr_of!(NET_MAC));
    let frame_len =
        build_tcp_frame(frame, &dmac, &smac, dst_ip, src_port, dst_port, seq, ack, flags, data);
    if frame_len == 0 {
        return 0;
    }
    #[cfg(feature = "pc")]
    if let Some((sip, _)) = route {
        set_src_ip(&mut frame[..frame_len], &sip);
    }
    let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
    write_volatile(
        txd.add(0),
        Desc { addr: addr_of_mut!(TX_BUF) as u64, len: (VNET_HDR + frame_len) as u32, flags: 0, next: 0 },
    );
    let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
    let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
    write_volatile(tring.add((tidx as usize) % QSZ), 0);
    dsb();
    write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
    dsb();
    w(base, R_QUEUE_NOTIFY, 1);
    frame_len
}

/// The fields of a received TCP segment this stack looks at, including where its
/// payload starts and how long it is — read from the IP total length rather than
/// from the frame length, since the wire may pad a short frame to 60 bytes and
/// that padding is not data.
struct Seg {
    /// WHO SENT IT — the IPv4 source address, read out of the header rather than
    /// assumed. Added at 2.769: before it, `Seg` described a segment's ports and
    /// sequence space but not its far end, so a listener physically could not
    /// answer its caller. Every inbound rig this project owns dials from
    /// `GW_IP`, which is why 683 green suites never caught the assumption.
    src: [u8; 4],
    sport: u16,
    dport: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    doff: usize,
    dlen: usize,
}

unsafe fn be32(p: *const u8) -> u32 {
    ((*p as u32) << 24) | ((*p.add(1) as u32) << 16) | ((*p.add(2) as u32) << 8) | (*p.add(3) as u32)
}

/// Parse an Ethernet frame as IPv4/TCP, or `None` if it is neither.
unsafe fn parse_tcp(fr: *const u8) -> Option<Seg> {
    let et = ((*fr.add(12) as u16) << 8) | (*fr.add(13) as u16);
    if et != 0x0800 || *fr.add(14 + 9) != 6 {
        return None;
    }
    let ihl = (*fr.add(14) & 0x0f) as usize * 4;
    let ip_total = ((*fr.add(16) as usize) << 8) | (*fr.add(17) as usize);
    if ihl < 20 || ip_total < ihl + 20 {
        return None;
    }
    let toff = 14 + ihl;
    let hlen = ((*fr.add(toff + 12) >> 4) as usize) * 4;
    if hlen < 20 || ihl + hlen > ip_total {
        return None;
    }
    Some(Seg {
        // The IPv4 source address: frame offset 26..30 (14 Ethernet + 12 into the
        // IP header). Fixed regardless of `ihl`, since options come AFTER the
        // addresses — so this is not `14 + ihl`-relative and must not be written
        // as if it were.
        src: [*fr.add(26), *fr.add(27), *fr.add(28), *fr.add(29)],
        sport: ((*fr.add(toff) as u16) << 8) | (*fr.add(toff + 1) as u16),
        dport: ((*fr.add(toff + 2) as u16) << 8) | (*fr.add(toff + 3) as u16),
        seq: be32(fr.add(toff + 4)),
        ack: be32(fr.add(toff + 8)),
        flags: *fr.add(toff + 13),
        doff: toff + hlen,
        dlen: ip_total - ihl - hlen,
    })
}

/// Sequence-space "at least": `a >= b` modulo 2^32 (RFC 793's comparison, so an
/// ACK that has wrapped past zero is still recognised as covering our bytes).
fn seq_ge(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// What an active open ended as.
enum TcpOpen {
    /// Handshake complete. `cur` is the RX ring cursor to continue reading from,
    /// so a caller that goes on to send data does not re-read the frames the
    /// handshake already consumed — nor skip one that arrived during it.
    Established { our_seq: u32, our_ack: u32, cur: u16 },
    /// The peer sent RST: nothing is listening. A round trip, but not a session.
    Refused,
    NoReply,
}

/// A device, and a gateway MAC to address a frame to. Shared by every TCP entry
/// point so they refuse for the same reasons in the same words.
unsafe fn tcp_ready(what: &str) -> Option<u64> {
    if !net_present() {
        crate::kprintln!("[net] {}: no virtio-net device", what);
        return None;
    }
    if !GW_KNOWN {
        net_arp_probe();
    }
    if !GW_KNOWN {
        crate::kprintln!("[net] {}: could not resolve the gateway MAC", what);
        return None;
    }
    Some(NET_BASE)
}

/// Send a SYN to `dst_ip`:`dst_port`, await the reply, and complete the
/// three-way handshake with the final ACK.
///
/// The reply is matched on the full 4-tuple — `s.dport == src_port && s.sport
/// == dst_port && s.src == *dst_ip` below. Before 3.704 this checked ports
/// only, so any host on the link that guessed the port pair could finish a
/// connection the dialling side thought it was having with `dst_ip`; probe
/// `scripts/dial-peerip-probe-3704.sh` measured that gap (`DIAL-PORTS-ONLY`,
/// exit 10) before this fix and now measures its close (`DIAL-VERIFIED`,
/// exit 0). This mirrors the passive-side fix (2.769: `s.src != peer_ip`) for
/// the paths the guest DIALS rather than answers.
unsafe fn tcp_open(base: u64, dst_ip: &[u8; 4], src_port: u16, dst_port: u16, iss: u32) -> TcpOpen {
    // The cursor is taken BEFORE the send: a reply can land before the notify
    // returns, and a baseline read afterwards misses it (the 2.41 ARP bug).
    let mut cur = read_volatile(addr_of!(NET_RX_USED.idx));
    if send_tcp_segment(base, dst_ip, src_port, dst_port, iss, 0, 0x02, &[]) == 0 {
        crate::kprintln!("[net] tcp: could not build the SYN");
        return TcpOpen::NoReply;
    }
    // The address is printed from the ARGUMENT, not from a literal that happens
    // to agree with it: with `dst_ip` = 10.0.2.2 this line is byte-identical to
    // the one suites `tcp` and `tcp-data` have always seen, and with any other
    // address it is the only place the serial says where the segment went.
    crate::kprintln!(
        "[net] TCP SYN sent to {}.{}.{}.{}:{} (seq {:#010x})",
        dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_port, iss
    );

    let mut spins = 0u64;
    loop {
        if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
            let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
            let elem = read_volatile(uring.add((cur as usize) % QSZ));
            cur = cur.wrapping_add(1);
            RX_RAW_COUNT += 1;
            spins = 0;
            let _slot = RxSlot(elem.id);
            let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
            if let Some(s) = parse_tcp(fr) {
                if s.dport == src_port && s.sport == dst_port && s.src == *dst_ip {
                    RX_COUNT += 1;
                    if s.flags & 0x12 == 0x12 {
                        crate::kprintln!(
                            "[net] TCP SYN/ACK received from {}.{}.{}.{}:{} (their seq {:#010x})",
                            dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_port, s.seq
                        );
                        // Our SYN consumed one sequence number; ACK their SYN.
                        let our_seq = iss.wrapping_add(1);
                        let our_ack = s.seq.wrapping_add(1);
                        send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                        crate::kprintln!(
                            "[net] TCP ACK sent, connection ESTABLISHED (seq {:#010x} ack {:#010x})",
                            our_seq, our_ack
                        );
                        return TcpOpen::Established { our_seq, our_ack, cur };
                    } else if s.flags & 0x04 != 0 {
                        crate::kprintln!(
                            "[net] TCP RST received from {}.{}.{}.{}:{} (port closed; TCP round-trip proven)",
                            dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_port
                        );
                        return TcpOpen::Refused;
                    }
                }
            }
        }
        spins += 1;
        if spins > 60_000_000 {
            crate::kprintln!("[net] TCP: no reply within timeout (a valid SYN was transmitted)");
            return TcpOpen::NoReply;
        }
    }
}

/// The state an active close reached. Named after RFC 793's states because that
/// is what the serial line reports and what the test greps for: a close that
/// stopped early is a DIFFERENT outcome from one that finished, and "closed"
/// printed unconditionally would hide exactly the failure worth catching.
enum TcpClose {
    /// Our FIN was acknowledged, the peer's FIN arrived and was acknowledged.
    /// The full four-segment exchange. (Or, when the peer had already sent its
    /// FIN before we closed, the CLOSED that ends the LAST_ACK path.)
    Done,
    /// Our FIN was acknowledged, but the peer never sent one — a half-closed
    /// connection this end has finished with. Legal TCP; not a finished session.
    FinWait2,
    /// Nobody acknowledged our FIN. Either the segment did not arrive or the
    /// peer is gone; without retransmission this end cannot tell which.
    FinWait1,
    /// The FIN could not even be built or sent.
    NotSent,
}

/// Close an established connection from THIS side: send FIN|ACK and see the
/// exchange through.
///
/// `our_seq` is the next sequence number we would send (the FIN takes it and
/// consumes one), `our_ack` acknowledges everything read so far, and `cur` is
/// the RX cursor to continue from — the same discipline as `tcp_open`, so no
/// frame that arrived during the data phase is re-read or skipped.
///
/// `peer_fin_seen` says the peer already closed its half and we already ACKed
/// it, which makes this the CLOSE_WAIT → LAST_ACK → CLOSED path instead of
/// FIN_WAIT_1 → FIN_WAIT_2 → TIME_WAIT. Both end the connection; they differ in
/// who went first, and the states are printed as they actually are.
unsafe fn tcp_close_cur(
    base: u64,
    dst_ip: &[u8; 4],
    src_port: u16,
    dst_port: u16,
    our_seq: u32,
    mut our_ack: u32,
    mut cur: u16,
    peer_fin_seen: bool,
) -> (TcpClose, u16) {
    // FIN|ACK, not a bare FIN: on an established connection every segment we
    // send carries the ACK of what we have read.
    if send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x11, &[]) == 0 {
        crate::kprintln!("[net] tcp close: could not build the FIN");
        return (TcpClose::NotSent, cur);
    }
    // Our FIN occupies one sequence number, so the ACK that covers it is one
    // past the byte stream — this is the arithmetic a stack that never closed
    // has never had to be right about.
    let want_ack = our_seq.wrapping_add(1);
    let mut their_fin = peer_fin_seen;
    if peer_fin_seen {
        crate::kprintln!(
            "[net] TCP FIN sent (seq {:#010x} ack {:#010x}), state CLOSE_WAIT -> LAST_ACK",
            our_seq, our_ack
        );
    } else {
        crate::kprintln!(
            "[net] TCP FIN sent (seq {:#010x} ack {:#010x}, expecting ack {:#010x}), state FIN_WAIT_1",
            our_seq, our_ack, want_ack
        );
    }

    let mut fin_acked = false;
    let mut spins = 0u64;
    loop {
        if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
            let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
            let elem = read_volatile(uring.add((cur as usize) % QSZ));
            cur = cur.wrapping_add(1);
            RX_RAW_COUNT += 1;
            spins = 0;
            let _slot = RxSlot(elem.id);
            let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
            let s = match parse_tcp(fr) {
                Some(s) => s,
                None => {
                    RX_CLO_UNPARSED += 1;
                    continue;
                }
            };
            if s.dport != src_port || s.sport != dst_port || s.src != *dst_ip {
                RX_CLO_OFFTUPLE += 1;
                continue;
            }
            RX_COUNT += 1;
            if s.flags & 0x04 != 0 {
                // A RST also ends the connection, just not politely. Reported as
                // what it is rather than dressed up as a completed close.
                crate::kprintln!("[net] TCP RST during close (the peer aborted, state CLOSED)");
                return (TcpClose::Done, cur);
            }
            // A peer may still send data after our FIN: we closed our sending
            // half, not its. Acknowledge it, or our final ACK names a sequence
            // number that does not account for the bytes it sent.
            if s.dlen > 0 {
                our_ack = s.seq.wrapping_add(s.dlen as u32);
                send_tcp_segment(base, dst_ip, src_port, dst_port, want_ack, our_ack, 0x10, &[]);
                crate::kprintln!(
                    "[net] TCP {} bytes arrived after our FIN (ACKed, ack {:#010x})",
                    s.dlen, our_ack
                );
            }
            // ACK and FIN can be — and from a well-behaved peer usually are —
            // the same segment, so both are checked on every one.
            if !fin_acked && s.flags & 0x10 != 0 && seq_ge(s.ack, want_ack) {
                fin_acked = true;
                if !peer_fin_seen {
                    crate::kprintln!(
                        "[net] TCP our FIN ACKed (their ack {:#010x}), state FIN_WAIT_2",
                        s.ack
                    );
                }
            }
            if !their_fin && s.flags & 0x01 != 0 {
                their_fin = true;
                // Their FIN consumes a sequence number of its own, after any
                // data the same segment carried.
                our_ack = s.seq.wrapping_add(s.dlen as u32).wrapping_add(1);
                send_tcp_segment(base, dst_ip, src_port, dst_port, want_ack, our_ack, 0x10, &[]);
                crate::kprintln!(
                    "[net] TCP peer FIN received (their seq {:#010x}), final ACK sent (ack {:#010x})",
                    s.seq, our_ack
                );
            }
            if fin_acked && their_fin {
                if peer_fin_seen {
                    crate::kprintln!("[net] TCP connection CLOSED (we closed last)");
                } else {
                    crate::kprintln!("[net] TCP connection closed, state TIME_WAIT");
                }
                return (TcpClose::Done, cur);
            }
        }
        spins += 1;
        if spins > 60_000_000 {
            if fin_acked {
                crate::kprintln!(
                    "[net] TCP close: our FIN was ACKed but no FIN came back, state FIN_WAIT_2"
                );
                return (TcpClose::FinWait2, cur);
            }
            crate::kprintln!("[net] TCP close: our FIN was never ACKed, state FIN_WAIT_1");
            return (TcpClose::FinWait1, cur);
        }
    }
}

/// The cursor-less form the two green callers use. `net_tcp_listen` (:2100) and
/// `net_tcp_data` (:2579) each close exactly one connection and then return, so
/// they have nowhere to carry a cursor TO — and leaving their call sites
/// untouched is what keeps suites 678/679 regression checks instead of
/// diff-reading exercises.
unsafe fn tcp_close(
    base: u64,
    dst_ip: &[u8; 4],
    src_port: u16,
    dst_port: u16,
    our_seq: u32,
    our_ack: u32,
    cur: u16,
    peer_fin_seen: bool,
) -> TcpClose {
    tcp_close_cur(base, dst_ip, src_port, dst_port, our_seq, our_ack, cur, peer_fin_seen).0
}

/// Send a TCP SYN to `dst_ip`:`dst_port`, complete the three-way handshake, and
/// stop there. Returns true if the peer answered at all (SYN/ACK or RST).
pub fn net_tcp(dst_ip: [u8; 4], dst_port: u16) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        let base = match tcp_ready("tcp") {
            Some(b) => b,
            None => return false,
        };
        match tcp_open(base, &dst_ip, 40001, dst_port, 0x1234_5678) {
            TcpOpen::Established { .. } | TcpOpen::Refused => true,
            TcpOpen::NoReply => false,
        }
    }
}

/// What a passive open ended as. The failures are kept apart because they name
/// different faults: nobody dialed at all, and somebody dialed but never finished
/// the handshake, are not the same defect and must not print the same line.
enum TcpAccept {
    /// The three-way handshake completed from the LISTENING side. `peer_ip` and
    /// `peer_port` are the caller's address and source port — the parts of the
    /// connection this end did not choose, and until 2.769 the address was not
    /// carried at all — and `cur` is the RX cursor to continue from, the same
    /// discipline as `tcp_open`, so the close does not re-read or skip a frame.
    Established { peer_ip: [u8; 4], peer_port: u16, our_seq: u32, our_ack: u32, cur: u16 },
    /// The listen window elapsed with no SYN. Nothing arrived to answer.
    NoSyn,
    /// A SYN was answered with SYN/ACK, but the handshake's third segment never
    /// came — even after the SYN/ACK was retransmitted. Half-open, not open.
    NoAck,
}

/// PASSIVE OPEN: wait `wait_ms` for an inbound SYN on `port`, answer it with
/// SYN/ACK, and complete the handshake when the caller's ACK arrives.
///
/// This is the mirror of `tcp_open`, and the asymmetry is the whole point: an
/// active open chooses both ends of the connection and knows what reply it is
/// waiting for, while a listener is answering a stranger — the peer's port, its
/// initial sequence number and the moment it dials are all its choice, and this
/// end has to read them off the wire rather than assume them.
///
/// The SYN/ACK is RETRANSMITTED on the same timer the data path uses
/// (`TCP_RTO_MS`, doubled, `TCP_MAX_RETX` attempts). It has to be: the SYN/ACK is
/// the one segment in a passive open that only THIS end can resend, and a
/// listener that sent it once sits in SYN_RECEIVED forever when it is lost, while
/// the caller keeps retransmitting a SYN it believes was never heard.
///
/// One connection at a time, still. A second caller arriving mid-handshake is
/// named and ignored rather than silently overwriting the first — this stack has
/// no connection table to put it in, and quietly switching would hand the first
/// caller's ACK to the second caller's handshake.
unsafe fn tcp_accept(base: u64, port: u16, iss: u32, wait_ms: u64, mut cur: u16) -> TcpAccept {
    // `cur` is handed IN rather than read here, and that is not a style choice —
    // it is the 2.41 ARP bug in the one place it bites hardest. Reaching this
    // function costs an ARP probe (`tcp_ready`), and a caller that dials the
    // moment it sees our ARP reply has its SYN in the ring BEFORE this line would
    // have run: a cursor read here starts past that SYN and the listener sits
    // through the whole window with the connection already waiting for it.
    // MEASURED, not feared: with the cursor taken here the first SYN was missed
    // every time and only the peer's RETRANSMITTED SYN was answered (4.0s later).
    // A listener must be listening from before the thing that announced it.
    crate::kprintln!(
        "[net] TCP LISTEN on 10.0.2.15:{}, state LISTEN (waiting {} ms for a SYN)",
        port, wait_ms
    );

    // 0 when the counter frequency is unknown; the spin cap below is then the
    // only bound, exactly as in the retransmit path.
    let listen_until = tcp_rto_deadline(wait_ms);
    let want_ack = iss.wrapping_add(1); // our SYN consumes one sequence number
    // The caller's address, learned from its SYN (2.769). Initialised to the
    // gateway because that is what every reply site assumed before this change,
    // so a path that somehow replied before a SYN was seen behaves exactly as it
    // did — and there is no such path: nothing is sent until `syn_seen`.
    let mut peer_ip = GW_IP;
    let mut peer_port = 0u16;
    let mut our_ack = 0u32;
    let mut syn_seen = false;
    let mut rto_ms = TCP_RTO_MS;
    let mut rto_at = 0u64;
    let mut retx = 0u32;
    let mut spins = 0u64;

    // design-3.529 §3(a). This is the one place the skip belongs: every listener
    // (`listen`, `serve`, `sink`) samples its cursor before `tcp_ready` and then
    // hands it here, so `tcp_ready`'s ARP has already run and published its mark
    // by the time this line executes — and every one of them gets the fix from
    // this single site. The header comment above still holds unchanged: `cur` is
    // not RE-READ here (that would start past the SYN); it is advanced only over
    // slots a nested consumer has already recycled.
    cur = rx_skip_consumed(cur);

    loop {
        if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
            let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
            let elem = read_volatile(uring.add((cur as usize) % QSZ));
            cur = cur.wrapping_add(1);
            RX_RAW_COUNT += 1;
            spins = 0;
            let _slot = RxSlot(elem.id);
            let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
            let s = match parse_tcp(fr) {
                Some(s) => s,
                None => {
                    RX_ACC_UNPARSED += 1;
                    continue;
                }
            };
            // Addressed to the port we are listening on, or it is somebody else's
            // conversation — including our own outbound ones, which are still
            // running through the same ring.
            if s.dport != port {
                RX_ACC_OFFPORT += 1;
                continue;
            }
            // 2.769 step (c): the port is the whole test only while there is no
            // connection yet. Once a caller has been accepted, this connection is
            // the full 4-tuple, and a segment from any other address or source
            // port is not part of it — measure-first watched an unrelated host's
            // bytes get read, echoed AND counted as an exchange. A fresh SYN is
            // deliberately let through to the second-SYN guard below, which has
            // its own line to print for it; everything else — RST, ACK, data —
            // belongs to this connection or to nobody.
            let is_syn = s.flags & 0x02 != 0 && s.flags & 0x10 == 0;
            if syn_seen && !is_syn && (s.src != peer_ip || s.sport != peer_port) {
                crate::kprintln!(
                    "[net] TCP segment from {}.{}.{}.{}:{} ignored on port {} (not this connection)",
                    s.src[0], s.src[1], s.src[2], s.src[3], s.sport, port
                );
                RX_ACC_OFFCONN += 1;
                continue;
            }
            RX_COUNT += 1;
            if s.flags & 0x04 != 0 {
                crate::kprintln!(
                    "[net] TCP RST on listening port {} (the caller aborted before the handshake finished)",
                    port
                );
                return if syn_seen { TcpAccept::NoAck } else { TcpAccept::NoSyn };
            }
            // A SYN with no ACK is a caller dialing. The same caller's SYN again
            // means our SYN/ACK was lost, and is answered the same way.
            if is_syn {
                if syn_seen && (s.src != peer_ip || s.sport != peer_port) {
                    crate::kprintln!(
                        "[net] TCP: a second SYN from {}.{}.{}.{}:{} ignored (one connection at a time)",
                        s.src[0], s.src[1], s.src[2], s.src[3], s.sport
                    );
                    continue;
                }
                peer_ip = s.src;
                peer_port = s.sport;
                // Their SYN consumes one sequence number, so the first byte of
                // their stream is seq + 1 — the arithmetic an end that only ever
                // dialed out has never had to do for somebody else's ISN.
                our_ack = s.seq.wrapping_add(1);
                if send_tcp_segment(base, &peer_ip, port, peer_port, iss, our_ack, 0x12, &[]) == 0 {
                    crate::kprintln!("[net] tcp listen: could not build the SYN/ACK");
                    return TcpAccept::NoAck;
                }
                if syn_seen {
                    crate::kprintln!(
                        "[net] TCP SYN repeated by {}.{}.{}.{}:{}, SYN/ACK sent again",
                        peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port
                    );
                } else {
                    crate::kprintln!(
                        "[net] TCP SYN received from {}.{}.{}.{}:{} (their seq {:#010x}), state SYN_RECEIVED",
                        peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port, s.seq
                    );
                    crate::kprintln!(
                        "[net] TCP SYN/ACK sent (seq {:#010x} ack {:#010x}, expecting ack {:#010x})",
                        iss, our_ack, want_ack
                    );
                }
                syn_seen = true;
                rto_ms = TCP_RTO_MS;
                rto_at = tcp_rto_deadline(rto_ms);
                retx = 0;
                continue;
            }
            // The handshake's third segment: an ACK from the caller that covers
            // our SYN. Anything else on this port before that is not the open.
            if syn_seen && s.sport == peer_port && s.flags & 0x10 != 0 && seq_ge(s.ack, want_ack) {
                // A caller is allowed to put data in that ACK. Those bytes are
                // received whether or not this end wanted them yet, so they are
                // accounted for — an ack that ignored them would name a sequence
                // number the peer has already moved past.
                if s.dlen > 0 {
                    our_ack = s.seq.wrapping_add(s.dlen as u32);
                    crate::kprintln!(
                        "[net] TCP {} byte(s) arrived with the handshake ACK (ack {:#010x})",
                        s.dlen, our_ack
                    );
                }
                crate::kprintln!(
                    "[net] TCP ACK received (their ack {:#010x}), connection ESTABLISHED (seq {:#010x} ack {:#010x})",
                    s.ack, want_ack, our_ack
                );
                return TcpAccept::Established { peer_ip, peer_port, our_seq: want_ack, our_ack, cur };
            }
        }
        spins += 1;
        // The SYN/ACK retransmit timer. Only armed once a SYN has been answered:
        // there is nothing to resend while still listening.
        if syn_seen && rto_at != 0 && crate::timer::cycles() >= rto_at {
            if retx >= TCP_MAX_RETX {
                crate::kprintln!(
                    "[net] TCP listen: SYN/ACK unacknowledged after {} retransmissions, state SYN_RECEIVED (the caller is gone)",
                    retx
                );
                return TcpAccept::NoAck;
            }
            retx += 1;
            if send_tcp_segment(base, &peer_ip, port, peer_port, iss, our_ack, 0x12, &[]) == 0 {
                crate::kprintln!("[net] TCP listen: retransmit {} could not rebuild the SYN/ACK", retx);
                return TcpAccept::NoAck;
            }
            crate::kprintln!(
                "[net] TCP retransmit {}/{}: SYN/ACK seq {:#010x} sent again after {} ms with no ACK",
                retx, TCP_MAX_RETX, iss, rto_ms
            );
            rto_ms = rto_ms.saturating_mul(2);
            rto_at = tcp_rto_deadline(rto_ms);
            spins = 0;
        }
        // Nobody has dialed and the window is over. Reported as the wait it was,
        // not as a failure of the listener: no SYN is a statement about the peer.
        if !syn_seen && ((listen_until != 0 && crate::timer::cycles() >= listen_until) || spins > 60_000_000) {
            crate::kprintln!(
                "[net] TCP listen: no SYN arrived on port {} within the window, state LISTEN",
                port
            );
            return TcpAccept::NoSyn;
        }
        // A handshake in flight with a dead timer still has to end somewhere.
        if syn_seen && rto_at == 0 && spins > 60_000_000 {
            crate::kprintln!("[net] TCP listen: no ACK for our SYN/ACK, state SYN_RECEIVED");
            return TcpAccept::NoAck;
        }
    }
}

/// Accept ONE inbound TCP connection on `port`, then close it from this end.
///
/// The first thing this stack does that it did not start. Every connection until
/// now was one this kernel dialed; a peer could not reach the guest at all, which
/// is the half of TCP a server is made of.
///
/// Returns true only if the handshake actually completed. The close is reported
/// on its own line and does not change that verdict, for the same reason as in
/// `net_tcp_data`: a connection that was accepted and then shut down badly is a
/// different defect from one that was never accepted.
pub fn net_tcp_listen(port: u16, wait_ms: u64) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        // FIRST, before anything that puts a frame on the wire: `tcp_ready` may
        // ARP for the gateway, and that ARP is the only announcement a peer gets
        // that this end is about to listen. Everything from here on is fair game
        // for a caller that reacts to it.
        let cur = read_volatile(addr_of!(NET_RX_USED.idx));
        let base = match tcp_ready("tcp listen") {
            Some(b) => b,
            None => return false,
        };
        if port == 0 {
            crate::kprintln!("[net] tcp listen: port must be 1..=65535");
            return false;
        }
        match tcp_accept(base, port, 0x00fa_ce00, wait_ms, cur) {
            TcpAccept::Established { peer_ip, peer_port, our_seq, our_ack, cur } => {
                crate::kprintln!(
                    "[net] TCP inbound connection ACCEPTED on port {} from {}.{}.{}.{}:{}",
                    port, peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port
                );
                // Ended rather than abandoned — 2.754's rule, and it applies to a
                // connection somebody else opened just as much as to one we did.
                match tcp_close(base, &peer_ip, port, peer_port, our_seq, our_ack, cur, false) {
                    TcpClose::Done => {}
                    TcpClose::FinWait2 => {
                        crate::kprintln!("[net] tcp listen: closed this end; the caller did not")
                    }
                    TcpClose::FinWait1 | TcpClose::NotSent => {
                        crate::kprintln!("[net] tcp listen: the connection did NOT close cleanly")
                    }
                }
                true
            }
            TcpAccept::NoSyn | TcpAccept::NoAck => false,
        }
    }
}

/// The bytes of the reply currently awaiting an ACK, kept so a retransmission can
/// resend the SAME bytes at the SAME sequence number (design §4(e)).
///
/// This buffer exists because an echo cannot be resent from the frame it arrived
/// in: that descriptor goes back to the device the instant the `RxSlot` guard
/// drops (2.760), so by the time an RTO fires those bytes are somewhere the
/// device may already have refilled. A retransmission reading them from there
/// would resend whatever landed since — the same sequence number carrying
/// different bytes, which is worse than not resending at all.
///
/// One reply is outstanding at a time, so one buffer is enough: this is a strict
/// request/response service and the `unacked` slot below holds at most one entry.
const TCP_ECHO_CAP: usize = TCP_MAX_PAYLOAD;
static mut TCP_ECHO_BUF: [u8; TCP_ECHO_CAP] = [0; TCP_ECHO_CAP];

/// ONE out-of-order run of bytes, held until the hole in front of it closes
/// (2.766, patch note M1). Not a reassembly queue: one hole covers reordering,
/// which is what this rig can produce and `scripts/reorder-probe-2766.sh` does
/// produce; N holes would be code with no falsifier behind it.
///
/// Capped at `TCP_ECHO_CAP` because the echo path already refuses to echo more
/// than that — holding more would buy latency and no delivery. The `(seq, len)`
/// that says what is in here lives in the serve loop beside `unacked`, so it is
/// re-armed per connection: a hold carried across connections would deliver
/// caller 1's stranded bytes into caller 2's stream.
static mut TCP_OOO_BUF: [u8; TCP_ECHO_CAP] = [0; TCP_ECHO_CAP];

/// Accept ONE inbound TCP connection on `port` and then STAY on it, echoing every
/// segment the caller sends until it FINs or the window closes — then close from
/// this end.
///
/// This is the join of two halves that had never met. `net_tcp_listen` accepts and
/// falls straight into `tcp_close`: the accepted connection's lifetime is zero
/// data segments. `net_tcp_data` converses, but as the DIALLING side and for
/// exactly one reply. So a connection this guest did not start has never carried a
/// conversation, and that — not the echo — is what this function is for.
///
/// Echo is the service body because it is the smallest server whose answer is
/// DERIVABLE from the question, so a test asserts the reply byte-for-byte instead
/// of asserting that something came back.
///
/// A new verb rather than a changed one: `net tcp listen` is suite 678 and green.
/// Changing the behaviour behind a green suite's command converts a regression
/// check into a rename check.
///
/// Returns true if the connection carried at least one exchange. The close is
/// reported on its own line and does not change that verdict — the same split as
/// `net_tcp_listen` and `net_tcp_data`, because a conversation that happened and
/// then shut down badly is a different defect from one that never happened.
pub fn net_tcp_serve(port: u16, wait_ms: u64, conns: u32) -> bool {
    net_tcp_serve_with(port, wait_ms, conns, None)
}

/// The reply a `net_tcp_serve_with` handler builds for each in-order run of bytes (mesh Phase 0, architect
/// review "minimal TCP 4401 hook"). It gets the received bytes AFTER the out-of-order hold, appends its reply
/// (at most `TCP_MAX_PAYLOAD` bytes, one segment) and returns false to close from this end after that reply.
pub type TcpHandler<'a> = &'a mut dyn FnMut(&[u8], &mut alloc::vec::Vec<u8>) -> bool;

/// `net_tcp_serve` with the echo step replaced by `handler`. `None` IS the echo, byte for byte: the bytes are
/// already in `TCP_ECHO_BUF` and are sent from there, exactly as before (suites tcp-serve, tcp-serve2, tcp-ooo).
pub fn net_tcp_serve_with(port: u16, wait_ms: u64, conns: u32, mut handler: Option<TcpHandler>) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        // The cursor is taken BEFORE anything that can put a frame on the wire,
        // for the reason spelled out in `tcp_accept`'s header: `tcp_ready` may ARP
        // for the gateway, that ARP is the peer's cue to dial, and a cursor read
        // after it starts PAST the SYN it provoked. Measured in 2.759, not feared:
        // 4.0 s (waiting for the caller's retransmitted SYN) versus 115 ms.
        let cur0 = read_volatile(addr_of!(NET_RX_USED.idx));
        let base = match tcp_ready("tcp serve") {
            Some(b) => b,
            None => return false,
        };
        if port == 0 {
            crate::kprintln!("[net] tcp serve: port must be 1..=65535");
            return false;
        }

        // CARRIED across connections and never re-read. This is the milestone:
        // every accept after the first is handed a ring POSITION, so the second
        // caller is looked for from where the first one's close stopped reading
        // rather than from `NET_RX_USED.idx` as it happens to stand now. Re-reading
        // the cursor at a new site is the 2.41/2.759 bug, and this stack does not
        // get to reintroduce it. (Honest scope, docs/patch-2.764-serve-loop.md §4:
        // caller 2's first SYN is eaten inside the close either way, so what this
        // buys is the narrow race where the SYN lands after the close finished —
        // worth the tuple, worth ~1 s of latency, NOT a pass/fail suite 681 sees.)
        let mut cur = cur0;
        let mut served = 0u32;
        let mut ex_total = 0u32;
        let mut bytes_total = 0usize;

        for n in 0..conns {
            // Said out loud BEFORE the wait, not after it: a connection whose SYN
            // was eaten inside the previous close is then visibly WAITING here for
            // the retransmission, instead of ambiguously hung (design §5).
            crate::kprintln!(
                "[net] serve: waiting for connection {}/{} on port {}",
                n + 1, conns, port
            );
            // A NEW ISS per connection (design §4). Back-to-back connections on the
            // same port pair sharing one ISS make a stale segment from the previous
            // connection indistinguishable from a live one.
            let (peer_ip, peer_port, mut our_seq, mut our_ack) = match tcp_accept(
                base,
                port,
                0x00fa_ce00u32.wrapping_add(n << 16),
                wait_ms,
                cur,
            ) {
                TcpAccept::Established { peer_ip, peer_port, our_seq, our_ack, cur: c } => {
                    cur = c;
                    (peer_ip, peer_port, our_seq, our_ack)
                }
                // Finding C: BREAK, do not return. The summary below is what names
                // how many connections actually landed, and a `return` here makes a
                // caller that never arrived look like a loop that never ran.
                TcpAccept::NoSyn | TcpAccept::NoAck => break,
            };
            served += 1;
            crate::kprintln!(
                "[net] TCP inbound connection ACCEPTED on port {} from {}.{}.{}.{}:{}",
                port, peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port
            );

            // RE-ARMED per connection (design §4). `wait_ms` bounds "how long will
            // you wait for a caller" and then "how long will you talk to it"
            // separately, because a window that carried its REMAINDER into the next
            // connection would be shortest exactly when the previous caller talked
            // longest. (§4 again: suite 681 finishes in ~2 s and cannot see this.)
            let serve_until = tcp_rto_deadline(wait_ms);

            // All per-connection, all reset here: an exchange count that carried
            // over would make two connections read as one long one, which is the
            // exact thing this milestone claims to have stopped doing.
            let mut exchanges = 0u32;
            let mut echoed_total = 0usize;
            let mut peer_fin = false;
            let mut spins = 0u64;

            // The reply awaiting an ACK: (its sequence number, its length). Its
            // bytes are in TCP_ECHO_BUF, and there is only ever one — a second
            // request cannot be served before the first reply is built, so the
            // buffer and this slot are written in the same breath and cleared in the
            // same breath. A stale one carried into connection 2 would retransmit
            // connection 1's echo into a stranger's stream.
            let mut unacked: Option<(u32, usize)> = None;
            // The one held out-of-order run: (its sequence number, its length), its
            // bytes in TCP_OOO_BUF. Declared HERE, beside `unacked` and inside the
            // per-connection block, not beside `cur` above — `cur` is carried across
            // connections on purpose and a hold that did the same would hand
            // connection 2 the bytes connection 1 never managed to deliver.
            let mut ooo: Option<(u32, usize)> = None;
            let mut rto_ms = TCP_RTO_MS;
            let mut rto_at = 0u64;
            let mut retx = 0u32;
            let mut rst = false;

            loop {
                if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                    let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                    let elem = read_volatile(uring.add((cur as usize) % QSZ));
                    cur = cur.wrapping_add(1);
                    RX_RAW_COUNT += 1;
                    spins = 0;
                    // 2.760's guard, and THIS is the call site where it is load-bearing
                    // rather than prudent: the serve loop is the first path in this
                    // kernel that consumes more than the 8 descriptors the pool starts
                    // with, so a missing recycle here is not a latent leak — it is a
                    // listener that goes deaf at frame 9, mid-conversation, every time.
                    let _slot = RxSlot(elem.id);
                    let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                    let s = match parse_tcp(fr) {
                        Some(s) => s,
                        None => continue,
                    };
                    // This connection only, and since 2.769 step (c) that means the
                    // full 4-tuple. Our own outbound conversations and anybody else's
                    // still run through the same ring — and a host that never
                    // handshook is not this caller merely because it borrowed the
                    // port pair. Silent on purpose: `tcp_accept` prints the ignored
                    // line once for the connection, and this loop reads every frame
                    // on the link for the whole serve window.
                    if s.dport != port || s.sport != peer_port || s.src != peer_ip {
                        continue;
                    }
                    RX_COUNT += 1;
                    if s.flags & 0x04 != 0 {
                        crate::kprintln!(
                            "[net] TCP RST from {}.{}.{}.{}:{} after {} exchange(s) — the caller aborted",
                            peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port, exchanges
                        );
                        // Finding B. END THIS CONNECTION, not the command. A `return`
                        // here would skip the end-of-serve summary entirely, so the
                        // suite would report a missing `serve: N connection(s) served`
                        // — which reads as "the loop never ran" when what happened is
                        // "caller 1 aborted". Two different defects, one symptom, and
                        // that is the failure mode this chain keeps paying for.
                        rst = true;
                        break;
                    }
                    // Their ACK retires the outstanding reply, and with it the timer.
                    if let Some((seq, len)) = unacked {
                        if s.flags & 0x10 != 0 && seq_ge(s.ack, seq.wrapping_add(len as u32)) {
                            unacked = None;
                            rto_at = 0;
                            rto_ms = TCP_RTO_MS;
                            retx = 0;
                        }
                    }
                    if s.dlen > 0 {
                        let end = s.seq.wrapping_add(s.dlen as u32);
                        if !seq_ge(our_ack, s.seq) {
                            // A GAP. These bytes begin past what we have acknowledged,
                            // so something before them is not here yet. The ACK goes
                            // out FIRST and unchanged — still `our_ack`, the cumulative
                            // point, never the held run's end (hazard 4). Telling the
                            // peer we have bytes we cannot yet deliver in order is the
                            // one thing worse than dropping them.
                            send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                            // 2.766: and THEN we keep them. Measured cost of not
                            // keeping them (design §2): the seven bytes were on our own
                            // wire at 0.830s, we threw them away, and the request came
                            // back whole 2.19 s later on the PEER's retransmission timer
                            // — a timer this stack cannot see, tune or bound.
                            let (off, hseq) = match ooo {
                                // The new run continues the held one: extend, keeping
                                // the held run's sequence number as the start.
                                Some((oseq, olen)) if s.seq == oseq.wrapping_add(olen as u32) => {
                                    (olen, oseq)
                                }
                                // Anything else REPLACES (design §3). Merging two
                                // disjoint runs is the general reassembly queue this
                                // milestone declined to build; a second `(seq, len)`
                                // would be code no falsifier in this rig can reach.
                                _ => (0usize, s.seq),
                            };
                            if off + s.dlen <= TCP_ECHO_CAP {
                                let dst = addr_of_mut!(TCP_OOO_BUF) as *mut u8;
                                for i in 0..s.dlen {
                                    *dst.add(off + i) = *fr.add(s.doff + i);
                                }
                                ooo = Some((hseq, off + s.dlen));
                                crate::kprintln!(
                                    "[net] serve: out-of-order {} byte(s) at seq {:#010x} HELD, ack {:#010x} repeated",
                                    s.dlen, s.seq, our_ack
                                );
                            } else {
                                // Too big to hold: behave exactly as the kernel did
                                // before this patch, and say so rather than truncating.
                                crate::kprintln!(
                                    "[net] serve: out-of-order {} byte(s) at seq {:#010x} dropped — past the {}-byte hold",
                                    s.dlen, s.seq, TCP_ECHO_CAP
                                );
                            }
                        } else if seq_ge(our_ack, end) {
                            // HAZARD (c) — the genuinely new problem. Every path in
                            // this stack before this one read at most ONE data segment,
                            // so "have I already seen these bytes?" has never had to be
                            // asked. A segment whose last byte is at or behind our ack
                            // is bytes already served: it is RE-ACKED, because the peer
                            // resent it doubting we heard, and NOT echoed, because a
                            // doubled reply fails a byte-for-byte assertion
                            // INTERMITTENTLY — which this project has three times read
                            // as harness flake rather than as the accounting bug it is.
                            send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                            crate::kprintln!(
                                "[net] serve: duplicate {} byte(s) at seq {:#010x} re-ACKed, not echoed (ack {:#010x})",
                                s.dlen, s.seq, our_ack
                            );
                        } else {
                            // Partially-seen segments are real: the new bytes start
                            // where our ack does, not where the segment does.
                            let skip = our_ack.wrapping_sub(s.seq) as usize;
                            let mut n = s.dlen - skip;
                            if n > TCP_ECHO_CAP {
                                our_ack = end;
                                send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                                crate::kprintln!(
                                    "[net] serve: {} byte(s) ACKed but NOT echoed — past the {}-byte echo cap",
                                    n, TCP_ECHO_CAP
                                );
                            } else {
                                let dst = addr_of_mut!(TCP_ECHO_BUF) as *mut u8;
                                for i in 0..n {
                                    *dst.add(i) = *fr.add(s.doff + skip + i);
                                }
                                our_ack = end;
                                // THE DRAIN (2.766, patch note M4). Here, between the
                                // ack advancing and the echo being built — not after
                                // the send — because building the echo first is
                                // precisely the measured defect: one request came back
                                // as `exchange 3` (6 bytes) and `exchange 4` (7 bytes),
                                // so the guest reported 5 exchanges for 4 requests and
                                // tcp-serve-test.sh:156 asserts that count.
                                if let Some((oseq, olen)) = ooo {
                                    let oend = oseq.wrapping_add(olen as u32);
                                    if seq_ge(our_ack, oend) {
                                        // Hazard 3: the peer said it again and its
                                        // retransmission covered what we were holding
                                        // (design §2 measured exactly this — 13 bytes
                                        // covering both halves). Stale, so DISCARD; a
                                        // held run delivered after its bytes have
                                        // already been served is a doubled echo.
                                        ooo = None;
                                        crate::kprintln!(
                                            "[net] serve: held run at seq {:#010x} discarded — already covered up to ack {:#010x}",
                                            oseq, our_ack
                                        );
                                    } else if seq_ge(our_ack, oseq) {
                                        // The hole is closed. The held run starts at or
                                        // before where we now are, so the new bytes
                                        // begin `our_ack - oseq` into it — the same
                                        // overlap arithmetic as `skip` above, which
                                        // design §2 (d) measured to be already right.
                                        let oskip = our_ack.wrapping_sub(oseq) as usize;
                                        let add = olen - oskip;
                                        if n + add <= TCP_ECHO_CAP {
                                            let dst = addr_of_mut!(TCP_ECHO_BUF) as *mut u8;
                                            let src = addr_of!(TCP_OOO_BUF) as *const u8;
                                            for i in 0..add {
                                                *dst.add(n + i) = *src.add(oskip + i);
                                            }
                                            n += add;
                                            our_ack = oend;
                                            // Hazard 1, cleared in the SAME breath as
                                            // the copy: a hold left set is appended
                                            // again to the next in-order segment, and a
                                            // doubled reply fails byte-for-byte
                                            // assertions INTERMITTENTLY — the failure
                                            // this project has three times misread as
                                            // harness flake.
                                            ooo = None;
                                            crate::kprintln!(
                                                "[net] serve: hole closed — {} held byte(s) from seq {:#010x} delivered in order",
                                                add, oseq
                                            );
                                        } else {
                                            // Dropped, never truncated: advancing
                                            // `our_ack` past bytes we did not echo
                                            // would lie about the stream, and the peer
                                            // already knows how to recover from a stall.
                                            ooo = None;
                                            crate::kprintln!(
                                                "[net] serve: held run at seq {:#010x} dropped — {} + {} byte(s) past the {}-byte echo cap",
                                                oseq, n, add, TCP_ECHO_CAP
                                            );
                                        }
                                    }
                                    // else: still a gap between here and the hold. Keep
                                    // holding; the ACK already told the peer where we
                                    // are.
                                }
                                // The handler hook (mesh Phase 0). With no handler this block
                                // does nothing and the echo below is unchanged.
                                let mut close_after = false;
                                if let Some(h) = handler.as_deref_mut() {
                                    // F6 (docs/design-f6-handshake-resend.md): acknowledge the bytes
                                    // BEFORE the handler runs. A handler (the mesh handshake) can take
                                    // longer than the peer's 300 ms RTO, and with the reply as the only
                                    // ACK the peer resent its message every time. Handler paths only:
                                    // the echo below keeps its one-segment-per-exchange shape.
                                    send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                                    let input = core::slice::from_raw_parts(addr_of!(TCP_ECHO_BUF) as *const u8, n).to_vec();
                                    let mut out = alloc::vec::Vec::new();
                                    close_after = !h(&input, &mut out);
                                    if out.len() > TCP_ECHO_CAP {
                                        crate::kprintln!(
                                            "[net] serve: handler reply of {} byte(s) is past the {}-byte cap, not sent; closing",
                                            out.len(), TCP_ECHO_CAP
                                        );
                                        out.clear();
                                        close_after = true;
                                    }
                                    let dst = addr_of_mut!(TCP_ECHO_BUF) as *mut u8;
                                    for (i, &b) in out.iter().enumerate() {
                                        *dst.add(i) = b;
                                    }
                                    n = out.len();
                                    if n == 0 {
                                        // Nothing to say: their bytes were acknowledged above; carry on
                                        // (or close).
                                        exchanges += 1;
                                        if close_after {
                                            break;
                                        }
                                        continue;
                                    }
                                }
                                let body =
                                    core::slice::from_raw_parts(addr_of!(TCP_ECHO_BUF) as *const u8, n);
                                // PSH|ACK: the echo IS the acknowledgement. One segment
                                // per exchange, not an ACK followed by data — the suite
                                // reads the pcap, and two frames where one belongs is a
                                // conversation that no longer looks like what it is.
                                if send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x18, body)
                                    == 0
                                {
                                    crate::kprintln!(
                                        "[net] serve: could not build the echo for exchange {}",
                                        exchanges + 1
                                    );
                                    break;
                                }
                                exchanges += 1;
                                echoed_total += n;
                                crate::kprintln!(
                                    "[net] serve: exchange {}, {} byte(s) echoed (seq {:#010x} ack {:#010x})",
                                    exchanges, n, our_seq, our_ack
                                );
                                unacked = Some((our_seq, n));
                                our_seq = our_seq.wrapping_add(n as u32);
                                rto_ms = TCP_RTO_MS;
                                rto_at = tcp_rto_deadline(rto_ms);
                                retx = 0;
                                if close_after {
                                    // The handler is done: close from this end now. The
                                    // reply goes out ahead of our FIN in sequence order.
                                    break;
                                }
                            }
                        }
                    }
                    if !peer_fin && s.flags & 0x01 != 0 {
                        // Their FIN occupies one sequence number of its own, on top of
                        // any payload it carried (already counted above). The caller
                        // went first, so our own close runs CLOSE_WAIT -> LAST_ACK.
                        peer_fin = true;
                        our_ack = our_ack.wrapping_add(1);
                        send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                        crate::kprintln!(
                            "[net] TCP caller sent FIN after {} exchange(s), ACKed — state CLOSE_WAIT",
                            exchanges
                        );
                        break;
                    }
                    continue;
                }
                spins += 1;
                // Reply retransmission — hazard (e). The SAME bytes at the SAME seq: a
                // reply resent at a new sequence number is a second, different segment,
                // and the peer stalls forever on the gap where the first one was. This
                // is also what rescues an exchange whose echo was lost: the peer will
                // resend its request, that request is a DUPLICATE and must not be
                // echoed again, so the only thing that can complete the exchange is
                // this timer.
                if let Some((seq, len)) = unacked {
                    if rto_at != 0 && crate::timer::cycles() >= rto_at {
                        if retx >= TCP_MAX_RETX {
                            crate::kprintln!(
                                "[net] serve: echo seq {:#010x} unacknowledged after {} retransmissions — the caller is gone",
                                seq, retx
                            );
                            break;
                        }
                        retx += 1;
                        let body =
                            core::slice::from_raw_parts(addr_of!(TCP_ECHO_BUF) as *const u8, len);
                        send_tcp_segment(base, &peer_ip, port, peer_port, seq, our_ack, 0x18, body);
                        crate::kprintln!(
                            "[net] TCP retransmit {}/{}: echo of {} byte(s), seq {:#010x} sent again after {} ms with no ACK",
                            retx, TCP_MAX_RETX, len, seq, rto_ms
                        );
                        rto_ms = rto_ms.saturating_mul(2);
                        rto_at = tcp_rto_deadline(rto_ms);
                        spins = 0;
                    }
                }
                if (serve_until != 0 && crate::timer::cycles() >= serve_until) || spins > 60_000_000 {
                    if exchanges == 0 {
                        crate::kprintln!(
                            "[net] serve: the caller connected but sent nothing before the window closed"
                        );
                    } else if unacked.is_some() {
                        crate::kprintln!(
                            "[net] serve: the window closed with a reply still unacknowledged after {} exchange(s)",
                            exchanges
                        );
                    } else {
                        // The 8-descriptor signature, named so it cannot be mistaken
                        // for a slow caller: served some, then heard nothing more and
                        // was never hung up on. If the RX pool were not being recycled
                        // this is exactly the line that would print.
                        crate::kprintln!(
                            "[net] serve: RX starved — {} exchange(s) served, then nothing arrived and the caller never FINed",
                            exchanges
                        );
                    }
                    break;
                }
            }
            crate::kprintln!(
                "[net] serve: {} exchange(s) on one connection, {} byte(s) echoed to {}.{}.{}.{}:{}",
                exchanges, echoed_total,
                peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port
            );
            ex_total += exchanges;
            bytes_total += echoed_total;
            // Ended rather than abandoned — 2.754's rule. `peer_fin` picks the path:
            // CLOSE_WAIT -> LAST_ACK when the caller hung up first (the expected one
            // here), FIN_WAIT_1 -> FIN_WAIT_2 -> TIME_WAIT when the window closed on
            // a caller that was still holding the line. Skipped after an RST: the
            // connection is already gone, and a FIN into a reset connection earns
            // another RST rather than a close.
            if !rst {
                let (r, c) = tcp_close_cur(base, &peer_ip, port, peer_port, our_seq, our_ack, cur, peer_fin);
                cur = c;
                match r {
                    TcpClose::Done => {}
                    TcpClose::FinWait2 => {
                        crate::kprintln!("[net] tcp serve: closed this end; the caller did not")
                    }
                    TcpClose::FinWait1 | TcpClose::NotSent => {
                        crate::kprintln!("[net] tcp serve: the connection did NOT close cleanly")
                    }
                }
            }
        }

        crate::kprintln!(
            "[net] serve: {} connection(s) served on port {}, {} exchange(s), {} byte(s) echoed",
            served, port, ex_total, bytes_total
        );
        ex_total > 0
    }
}

/// `net tcp sink <port> [secs]` — the INGRESS half, and a NEW verb beside
/// `serve` rather than a flag on it (the 2.762 rule): `serve` is green suites
/// 680/681, and a listener that stops echoing is a rewrite of what they assert,
/// not a regression check.
///
/// What this buys that no path in this stack has ever had: inbound bytes
/// ACCEPTED AND COUNTED with no reply built. 3.487 measured why that matters —
/// the only inbound verb ECHOES, and echo throughput is not ingest throughput;
/// a rate read off a path that must build a reply per segment measures the
/// reply builder.
///
/// Deliberately NOT here: `serve`'s one-hole out-of-order hold (TCP_OOO_BUF).
/// A sink re-ACKs its cumulative point and counts only what arrives IN ORDER,
/// so a gap makes its number too SMALL, never too large — the number it prints
/// is a floor on what was ingested, which is the only direction a throughput
/// claim may err in.
pub fn net_tcp_sink(port: u16, wait_ms: u64) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        // Cursor BEFORE anything that can ARP (tcp_accept's header, measured in
        // 2.759): the ARP is the peer's cue to dial, so a cursor read after it
        // starts PAST the SYN it provoked.
        let cur0 = read_volatile(addr_of!(NET_RX_USED.idx));
        let base = match tcp_ready("tcp sink") {
            Some(b) => b,
            None => return false,
        };
        if port == 0 {
            crate::kprintln!("[net] tcp sink: port must be 1..=65535");
            return false;
        }
        let mut cur = cur0;
        crate::kprintln!("[net] sink: waiting for a connection on port {}", port);
        // Its own ISS space, disjoint from `serve`'s 0x00face00 family, so a
        // stale segment from a previous serve on the same port pair cannot be
        // mistaken for this connection's.
        let (peer_ip, peer_port, our_seq, mut our_ack) =
            match tcp_accept(base, port, 0x00fa_5e00u32, wait_ms, cur) {
                TcpAccept::Established { peer_ip, peer_port, our_seq, our_ack, cur: c } => {
                    cur = c;
                    (peer_ip, peer_port, our_seq, our_ack)
                }
                TcpAccept::NoSyn | TcpAccept::NoAck => {
                    crate::kprintln!(
                        "[net] sink: nobody completed a handshake on port {} before the window closed",
                        port
                    );
                    return false;
                }
            };
        crate::kprintln!(
            "[net] TCP inbound connection ACCEPTED on port {} from {}.{}.{}.{}:{} — SINKING, not echoing",
            port, peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port
        );

        let sink_until = tcp_rto_deadline(wait_ms);
        // The clock starts at ESTABLISHED, not at the command: waiting for a
        // caller is not ingest time, and folding it in would understate the rate
        // by whatever the peer's dial cost happened to be.
        let t0 = crate::timer::cycles();
        let mut sunk = 0usize;
        let mut segs = 0u32;
        let mut dups = 0u32;
        let mut gaps = 0u32;
        let mut peer_fin = false;
        let mut rst = false;
        let mut spins = 0u64;
        let mut t_end = t0;

        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                spins = 0;
                // 2.760's recycle, load-bearing here for the same reason as in
                // `serve`: a bulk sink consumes far more than the 8 descriptors
                // the pool starts with, so a missing recycle is a listener that
                // goes deaf mid-stream rather than a latent leak.
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let s = match parse_tcp(fr) {
                    Some(s) => s,
                    None => {
                        // Escape (A). Counted, not classified: ARP / malformed /
                        // short all arrive here as one `None` (3.520).
                        RX_UNPARSED += 1;
                        continue;
                    }
                };
                // The full 4-tuple (2.769 step (c)). Silent on purpose: this loop
                // reads every frame on the link for the whole sink window.
                if s.dport != port || s.sport != peer_port || s.src != peer_ip {
                    // Escape (B): parsed TCP, addressed to someone else.
                    RX_OFFTUPLE += 1;
                    continue;
                }
                RX_COUNT += 1;
                if s.flags & 0x04 != 0 {
                    crate::kprintln!(
                        "[net] TCP RST from {}.{}.{}.{}:{} after {} byte(s) sunk — the caller aborted",
                        peer_ip[0], peer_ip[1], peer_ip[2], peer_ip[3], peer_port, sunk
                    );
                    rst = true;
                    break;
                }
                if s.dlen > 0 {
                    let end = s.seq.wrapping_add(s.dlen as u32);
                    if !seq_ge(our_ack, s.seq) {
                        // A GAP: these bytes begin past our cumulative point. The
                        // ACK repeats `our_ack` UNCHANGED — telling the peer we
                        // hold bytes we have not accounted for is the one error
                        // worse than dropping them — and they are NOT counted.
                        gaps += 1;
                        // 3.491 discriminator (observability only): the ack value
                        // AT THE HAND-OFF, so the guest's own sequence can be set
                        // against the pcap. M-A vs M-B, nothing else changed.
                        crate::kprintln!("[sink] ack-out {:08x} gap", our_ack);
                        send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                    } else if seq_ge(our_ack, end) {
                        // Already sunk. RE-ACKed because the peer resent doubting
                        // us, and NOT counted twice — double-counting a
                        // retransmission is exactly how a throughput number
                        // flatters itself.
                        dups += 1;
                        crate::kprintln!("[sink] ack-out {:08x} dup", our_ack);
                        send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                    } else {
                        // Partially-seen segments are real: the new bytes start
                        // where our ack does, not where the segment does.
                        let skip = our_ack.wrapping_sub(s.seq) as usize;
                        sunk += s.dlen - skip;
                        our_ack = end;
                        segs += 1;
                        t_end = crate::timer::cycles();
                        // A BARE ACK, never PSH|ACK with a body. That absence IS
                        // the verb: `serve` answers 0x18 with the caller's own
                        // bytes, and the probe grades any echoed byte as
                        // SINK-ECHOES precisely so this cannot drift back into
                        // being `serve` with a different name.
                        //
                        // 3.598: this ACK is NOT announced per segment any more.
                        // The line that used to stand here cost ~238,800
                        // synchronous framebuffer word ops per 512 B segment
                        // (3.595's count, 3.597's 18.6x measurement) — the send
                        // below is ~15 words. The discriminator is not lost, it
                        // is DEFERRED: the final ack and the advance count are
                        // printed ONCE beside the account below. dup/gap keep
                        // their per-occurrence lines on purpose — they are the
                        // rare arm (1-14 per run against 60-211 advances) and
                        // they are what a reader is diffing when a run goes odd.
                        send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                    }
                }
                if !peer_fin && s.flags & 0x01 != 0 {
                    // Their FIN occupies one sequence number of its own, on top of
                    // any payload it carried (already counted above).
                    peer_fin = true;
                    our_ack = our_ack.wrapping_add(1);
                    crate::kprintln!("[sink] ack-out {:08x} fin", our_ack);
                    send_tcp_segment(base, &peer_ip, port, peer_port, our_seq, our_ack, 0x10, &[]);
                    crate::kprintln!(
                        "[net] TCP caller sent FIN after {} byte(s) sunk, ACKed — state CLOSE_WAIT",
                        sunk
                    );
                    break;
                }
                continue;
            }
            spins += 1;
            // No retransmission timer, because a sink never has an outstanding
            // segment of its own to retransmit: every frame it sends is a bare
            // ACK carrying no sequence space.
            if (sink_until != 0 && crate::timer::cycles() >= sink_until) || spins > 60_000_000 {
                crate::kprintln!(
                    "[net] sink: the window closed with {} byte(s) sunk and no FIN from the caller",
                    sunk
                );
                break;
            }
        }

        // Elapsed is measured to the LAST COUNTED BYTE, not to the end of the
        // window: a 20 s window that finished ingesting at 3 s would otherwise
        // report a rate 6.7x below what the path actually did. Both numbers are
        // printed so the reader can see which one a rate was taken over.
        let hz = crate::timer::hz().max(1);
        let ms = t_end.wrapping_sub(t0).saturating_mul(1000) / hz;
        let win_ms = crate::timer::cycles().wrapping_sub(t0).saturating_mul(1000) / hz;
        // 3.598: the per-segment `ack-out ... adv` line, printed ONCE. Same two
        // facts it used to carry (the count of advancing ACKs, and the ack value
        // at the last hand-off) at O(1) console cost instead of O(segments).
        crate::kprintln!("[sink] ack-out {:08x} adv x{}", our_ack, segs);
        crate::kprintln!(
            "[net] sink: {} byte(s) sunk in {} segment(s) over {} ms (window {} ms; {} dup, {} gap) — nothing echoed",
            sunk, segs, ms, win_ms, dups, gaps
        );
        if ms > 0 {
            crate::kprintln!(
                "[net] sink: rate {} bytes/s",
                (sunk as u64).saturating_mul(1000) / ms
            );
        }
        if !rst {
            let (r, c) = tcp_close_cur(base, &peer_ip, port, peer_port, our_seq, our_ack, cur, peer_fin);
            cur = c;
            let _ = cur;
            match r {
                TcpClose::Done => {}
                TcpClose::FinWait2 => {
                    crate::kprintln!("[net] tcp sink: closed this end; the caller did not")
                }
                TcpClose::FinWait1 | TcpClose::NotSent => {
                    crate::kprintln!("[net] tcp sink: the connection did NOT close cleanly")
                }
            }
        }
        sunk > 0
    }
}

/// Where a reply is kept long enough to be printed. Small on purpose: this is a
/// request/response exchange, not a receive buffer, and a reply longer than this
/// is reported as truncated rather than silently cut.
const TCP_RX_CAP: usize = 256;
static mut TCP_RX_DATA: [u8; TCP_RX_CAP] = [0; TCP_RX_CAP];

/// The initial retransmission timeout, in milliseconds. Short on purpose: this
/// is a loopback-class link where a round trip is sub-millisecond, so 300 ms is
/// already two orders of magnitude of slack, and a timer long enough to be
/// "safe" is one no test can afford to wait for.
///
/// A real stack derives this from measured round-trip times (RFC 6298); this one
/// starts from a constant and doubles it on each attempt, which is the half of
/// that algorithm that matters when a segment is lost — back off, do not hammer.
const TCP_RTO_MS: u64 = 300;

/// How many times a segment is sent again before the send is declared failed.
/// Four attempts at 300/600/1200/2400 ms is 4.5 s of trying, after which the
/// honest answer is that the peer is gone rather than slow.
const TCP_MAX_RETX: u32 = 4;

/// Arm the retransmit timer: the CNTPCT value at which `ms` will have elapsed.
///
/// The counter is 64-bit and runs at tens of MHz, so it does not wrap inside any
/// uptime this kernel will ever see; a plain comparison against the returned
/// deadline is therefore safe. Returns 0 when the counter frequency is unknown,
/// which callers read as "no timer available" and fall back to their spin cap —
/// a stack that cannot tell the time must not retransmit blindly.
fn tcp_rto_deadline(ms: u64) -> u64 {
    let hz = crate::timer::hz();
    if hz == 0 {
        return 0;
    }
    crate::timer::cycles().wrapping_add(hz.saturating_mul(ms) / 1000)
}

/// Open a connection to `dst_ip`:`dst_port`, send `payload` on it, and read what
/// the peer sends back — the first thing this stack does on an ESTABLISHED
/// connection rather than merely reaching one.
///
/// The connection is then ACTIVELY CLOSED (`tcp_close`) rather than abandoned
/// when this returns.
///
/// Returns true only if the peer BOTH acknowledged our bytes and answered with
/// bytes of its own. An ACK alone proves the send arrived; it does not prove
/// this end can read a reply, and those are the two halves of a data path. The
/// close is reported on its own line and does not change that verdict: a round
/// trip that completed and then failed to shut down cleanly is a different
/// defect from one whose bytes never arrived, and collapsing them into one bool
/// would make the second unreadable.
pub fn net_tcp_data(dst_ip: [u8; 4], dst_port: u16, payload: &[u8]) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        let base = match tcp_ready("tcp send") {
            Some(b) => b,
            None => return false,
        };
        if payload.is_empty() || payload.len() > TCP_MAX_PAYLOAD {
            crate::kprintln!(
                "[net] tcp send: payload must be 1..={} bytes (got {})",
                TCP_MAX_PAYLOAD, payload.len()
            );
            return false;
        }
        let src_port: u16 = 40002;
        // Bound once and passed by reference from here down, so the SYN, the
        // data segment, every retransmission and the FIN cannot disagree about
        // where the connection goes.
        let dst_ip = &dst_ip;
        let (mut our_seq, mut our_ack, mut cur) = match tcp_open(base, dst_ip, src_port, dst_port, 0x00c0_ffee) {
            TcpOpen::Established { our_seq, our_ack, cur } => (our_seq, our_ack, cur),
            TcpOpen::Refused => {
                crate::kprintln!("[net] tcp send: nothing is listening on port {}", dst_port);
                return false;
            }
            TcpOpen::NoReply => return false,
        };

        // PSH|ACK: hand the bytes up on arrival instead of sitting in the peer's
        // buffer waiting for more that will never come.
        if send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x18, payload) == 0 {
            crate::kprintln!("[net] tcp send: could not build the data segment");
            return false;
        }
        // The sequence number the payload occupies. Kept, because a retransmission
        // that invented a new one would be a second, different segment: the peer
        // would see a gap where the lost bytes were and stall forever waiting for
        // them. Sending the SAME bytes at the SAME place is the whole mechanism.
        let data_seq = our_seq;
        let want_ack = our_seq.wrapping_add(payload.len() as u32);
        crate::kprintln!(
            "[net] TCP data sent: {} bytes (seq {:#010x}, expecting ack {:#010x})",
            payload.len(), our_seq, want_ack
        );
        our_seq = want_ack;

        // Arm the retransmit timer the moment the segment is on the wire.
        let mut rto_ms = TCP_RTO_MS;
        let mut rto_at = tcp_rto_deadline(rto_ms);
        let mut retx = 0u32;

        let mut acked = false;
        let mut got = 0usize;
        let mut peer_fin = false;
        let mut spins = 0u64;
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                spins = 0;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let s = match parse_tcp(fr) {
                    Some(s) => s,
                    None => continue,
                };
                if s.dport != src_port || s.sport != dst_port || s.src != *dst_ip {
                    continue;
                }
                RX_COUNT += 1;
                if s.flags & 0x04 != 0 {
                    crate::kprintln!("[net] TCP RST from the peer mid-exchange");
                    return false;
                }
                if !acked && s.flags & 0x10 != 0 && seq_ge(s.ack, want_ack) {
                    acked = true;
                    crate::kprintln!(
                        "[net] TCP peer ACKed our {} bytes (their ack {:#010x})",
                        payload.len(), s.ack
                    );
                }
                if s.dlen > 0 {
                    let n = if s.dlen > TCP_RX_CAP { TCP_RX_CAP } else { s.dlen };
                    let dst = addr_of_mut!(TCP_RX_DATA) as *mut u8;
                    for i in 0..n {
                        *dst.add(i) = *fr.add(s.doff + i);
                    }
                    // Acknowledge the whole segment, including the part we did
                    // not keep: the peer sent those bytes and they are consumed.
                    our_ack = s.seq.wrapping_add(s.dlen as u32);
                    got += s.dlen;
                    crate::kprint!("[net] TCP received {} bytes: \"", s.dlen);
                    let src = addr_of!(TCP_RX_DATA) as *const u8;
                    for i in 0..n {
                        let b = *src.add(i);
                        let c = if (0x20..0x7f).contains(&b) { b as char } else { '.' };
                        crate::kprint!("{}", c);
                    }
                    if n < s.dlen {
                        crate::kprintln!("\" (+{} more, not kept)", s.dlen - n);
                    } else {
                        crate::kprintln!("\"");
                    }
                    send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                    crate::kprintln!("[net] TCP ACK sent for their data (ack {:#010x})", our_ack);
                }
                if !peer_fin && s.flags & 0x01 != 0 {
                    // FIN occupies one sequence number of its own. The peer went
                    // first; our own close then runs the LAST_ACK path.
                    peer_fin = true;
                    our_ack = our_ack.wrapping_add(1);
                    send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                    crate::kprintln!("[net] TCP peer sent FIN (ACKed; closing this end next)");
                }
                if acked && got > 0 {
                    crate::kprintln!(
                        "[net] TCP data round-trip OK: {} bytes out, {} bytes in",
                        payload.len(), got
                    );
                    // End the connection instead of walking away from it.
                    match tcp_close(base, dst_ip, src_port, dst_port, our_seq, our_ack, cur, peer_fin) {
                        TcpClose::Done => {}
                        TcpClose::FinWait2 => {
                            crate::kprintln!("[net] tcp send: closed this end; the peer did not")
                        }
                        TcpClose::FinWait1 | TcpClose::NotSent => {
                            crate::kprintln!("[net] tcp send: the connection did NOT close cleanly")
                        }
                    }
                    return true;
                }
            }
            spins += 1;
            // The retransmit timer. It governs only the UNACKNOWLEDGED case: once
            // the peer has acknowledged the bytes, sending them again would be
            // noise, and the wait for its reply falls back to the spin cap below.
            if !acked && rto_at != 0 && crate::timer::cycles() >= rto_at {
                if retx >= TCP_MAX_RETX {
                    crate::kprintln!(
                        "[net] TCP data: seq {:#010x} unacknowledged after {} retransmissions — the peer is gone",
                        data_seq, retx
                    );
                    return false;
                }
                retx += 1;
                if send_tcp_segment(base, dst_ip, src_port, dst_port, data_seq, our_ack, 0x18, payload) == 0 {
                    crate::kprintln!("[net] TCP retransmit {}: could not rebuild the segment", retx);
                    return false;
                }
                crate::kprintln!(
                    "[net] TCP retransmit {}/{}: {} bytes, seq {:#010x} sent again after {} ms with no ACK",
                    retx, TCP_MAX_RETX, payload.len(), data_seq, rto_ms
                );
                // Back off before the next attempt, and restart the spin cap: a
                // connection that is retransmitting is not a connection that has
                // heard nothing, and it must not be timed out as if it were.
                rto_ms = rto_ms.saturating_mul(2);
                rto_at = tcp_rto_deadline(rto_ms);
                spins = 0;
            }
            if spins > 60_000_000 {
                crate::kprintln!(
                    "[net] TCP data: timed out (acked={} received={} bytes, retransmissions {})",
                    acked, got, retx
                );
                return false;
            }
        }
    }
}

/// Dial `dst_ip`:`dst_port`, send `first`, then CONVERSE: every in-order run of bytes the peer sends goes to
/// `handler`, whose non-empty reply is sent as the next segment (one outstanding reply at a time, resent on the
/// same RTO schedule as `net_tcp_data`). The handler returns false when it is done; the connection is then
/// closed from this end. New code beside `net_tcp_data` (mesh Phase 0, architect review "minimal TCP 4401
/// hook"): no existing caller changes. Returns true if the handler said it was done before anything failed.
pub fn net_tcp_dial_with(dst_ip: [u8; 4], dst_port: u16, first: &[u8], wait_ms: u64, handler: TcpHandler) -> bool {
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        let base = match tcp_ready("tcp dial") {
            Some(b) => b,
            None => return false,
        };
        if first.is_empty() || first.len() > TCP_MAX_PAYLOAD {
            crate::kprintln!("[net] tcp dial: first segment must be 1..={} bytes (got {})", TCP_MAX_PAYLOAD, first.len());
            return false;
        }
        let src_port: u16 = 41000 + (crate::rng::next_u32() % 20000) as u16;
        let iss = crate::rng::next_u32();
        let dst_ip = &dst_ip;
        let (mut our_seq, mut our_ack, mut cur) = match tcp_open(base, dst_ip, src_port, dst_port, iss) {
            TcpOpen::Established { our_seq, our_ack, cur } => (our_seq, our_ack, cur),
            TcpOpen::Refused => {
                crate::kprintln!("[net] tcp dial: nothing is listening on port {}", dst_port);
                return false;
            }
            TcpOpen::NoReply => return false,
        };
        // The one outstanding segment: its bytes (kept for retransmission) and its sequence number.
        let mut pending: alloc::vec::Vec<u8> = first.to_vec();
        let mut pending_seq = our_seq;
        if send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x18, &pending) == 0 {
            crate::kprintln!("[net] tcp dial: could not build the first segment");
            return false;
        }
        our_seq = our_seq.wrapping_add(pending.len() as u32);
        let mut unacked = true;
        let mut rto_ms = TCP_RTO_MS;
        let mut rto_at = tcp_rto_deadline(rto_ms);
        let mut retx = 0u32;
        let until = tcp_rto_deadline(wait_ms);
        let mut peer_fin = false;
        let mut done = false;
        let mut segs_in = 0u32;
        let mut segs_out = 1u32;
        let mut spins = 0u64;
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                spins = 0;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                let s = match parse_tcp(fr) {
                    Some(s) => s,
                    None => continue,
                };
                if s.dport != src_port || s.sport != dst_port || s.src != *dst_ip {
                    continue;
                }
                RX_COUNT += 1;
                if s.flags & 0x04 != 0 {
                    crate::kprintln!("[net] tcp dial: RST from the peer after {} segment(s) in", segs_in);
                    return false;
                }
                if unacked && s.flags & 0x10 != 0 && seq_ge(s.ack, our_seq) {
                    unacked = false;
                    retx = 0;
                    rto_ms = TCP_RTO_MS;
                }
                if s.dlen > 0 {
                    if s.seq != our_ack {
                        // A duplicate (behind us) or a gap (ahead of us): re-ACK where we are and let the
                        // peer resend in order. A run is never handed to the handler twice.
                        send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                    } else if done || unacked {
                        // Not ready for a new run (our previous segment is still unacknowledged, or the
                        // handler is finished): leave these bytes unacknowledged, the peer resends them.
                    } else {
                        let input = core::slice::from_raw_parts(fr.add(s.doff), s.dlen).to_vec();
                        our_ack = s.seq.wrapping_add(s.dlen as u32);
                        segs_in += 1;
                        let mut out = alloc::vec::Vec::new();
                        let more = handler(&input, &mut out);
                        if out.len() > TCP_MAX_PAYLOAD {
                            crate::kprintln!(
                                "[net] tcp dial: handler reply of {} byte(s) is past the {}-byte cap, not sent",
                                out.len(), TCP_MAX_PAYLOAD
                            );
                            out.clear();
                            done = true;
                        }
                        if out.is_empty() {
                            send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                        } else {
                            pending = out;
                            pending_seq = our_seq;
                            if send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x18, &pending) == 0 {
                                crate::kprintln!("[net] tcp dial: could not build a reply segment");
                                return false;
                            }
                            our_seq = our_seq.wrapping_add(pending.len() as u32);
                            segs_out += 1;
                            unacked = true;
                            rto_ms = TCP_RTO_MS;
                            rto_at = tcp_rto_deadline(rto_ms);
                            retx = 0;
                        }
                        if !more {
                            done = true;
                        }
                    }
                }
                if !peer_fin && s.flags & 0x01 != 0 && s.seq.wrapping_add(s.dlen as u32) == our_ack {
                    peer_fin = true;
                    our_ack = our_ack.wrapping_add(1);
                    send_tcp_segment(base, dst_ip, src_port, dst_port, our_seq, our_ack, 0x10, &[]);
                    if !done {
                        crate::kprintln!("[net] tcp dial: the peer closed before the handler was done");
                    }
                }
                if (done || peer_fin) && !unacked {
                    break;
                }
                continue;
            }
            spins += 1;
            if unacked && rto_at != 0 && crate::timer::cycles() >= rto_at {
                if retx >= TCP_MAX_RETX {
                    crate::kprintln!(
                        "[net] tcp dial: seq {:#010x} unacknowledged after {} retransmissions, the peer is gone",
                        pending_seq, retx
                    );
                    return false;
                }
                retx += 1;
                send_tcp_segment(base, dst_ip, src_port, dst_port, pending_seq, our_ack, 0x18, &pending);
                crate::kprintln!(
                    "[net] tcp dial: retransmit {}/{}: {} byte(s), seq {:#010x}",
                    retx, TCP_MAX_RETX, pending.len(), pending_seq
                );
                rto_ms = rto_ms.saturating_mul(2);
                rto_at = tcp_rto_deadline(rto_ms);
                spins = 0;
            }
            if (until != 0 && crate::timer::cycles() >= until) || spins > 60_000_000 {
                crate::kprintln!(
                    "[net] tcp dial: window closed ({} segment(s) out, {} in, done={})",
                    segs_out, segs_in, done
                );
                break;
            }
        }
        crate::kprintln!("[net] tcp dial: {} segment(s) out, {} in, done={}", segs_out, segs_in, done);
        match tcp_close(base, dst_ip, src_port, dst_port, our_seq, our_ack, cur, peer_fin) {
            TcpClose::Done => {}
            TcpClose::FinWait2 => crate::kprintln!("[net] tcp dial: closed this end; the peer did not"),
            TcpClose::FinWait1 | TcpClose::NotSent => {
                crate::kprintln!("[net] tcp dial: the connection did NOT close cleanly")
            }
        }
        done
    }
}

// ---------------------------------------------------------------------------
// The field report's sender (PC only: `report <ip>[:port]`, pc/report.rs).
//
// Every other path in this stack was written for QEMU's user-mode network: it
// sends from 10.0.2.15 and addresses every frame to the gateway's MAC. A spare
// PC on a home router has its own leased address, and the owner's main PC sits
// on the same wire, so the report needs three things the rest of the stack does
// not have: the LEASED source address, the next hop's OWN MAC (ARP for the
// destination when it is on the same subnet, else for the router), and a sender
// that streams many segments rather than one. It is ONE-WAY: it opens one
// connection to the address the owner typed, sends, and closes. Nothing the
// other end sends is read as data, let alone as a command.
// ---------------------------------------------------------------------------

/// While a report is being sent: (our leased address, the next hop's MAC). Read
/// by `send_tcp_segment`; `None` (the default) leaves every other path as it was.
#[cfg(feature = "pc")]
static mut REPORT_ROUTE: Option<([u8; 4], [u8; 6])> = None;

/// Put `sip` in a built frame's IPv4 source and recompute both checksums (the IP
/// header's and the TCP one, whose pseudo-header carries the source too).
#[cfg(feature = "pc")]
fn set_src_ip(frame: &mut [u8], sip: &[u8; 4]) {
    if frame.len() < TCP_FRAME_MIN {
        return;
    }
    frame[26..30].copy_from_slice(sip);
    frame[24] = 0;
    frame[25] = 0;
    let ipck = inet_checksum(&frame[14..34]);
    frame[24] = (ipck >> 8) as u8;
    frame[25] = ipck as u8;
    let seg_len = frame.len() - 34;
    let mut pseudo = [0u8; 12];
    pseudo[0..8].copy_from_slice(&frame[26..34]);
    pseudo[9] = 6;
    pseudo[10] = (seg_len >> 8) as u8;
    pseudo[11] = seg_len as u8;
    frame[50] = 0;
    frame[51] = 0;
    let mut sum = inet_sum_add(0, &pseudo);
    sum = inet_sum_add(sum, &frame[34..]);
    let tck = inet_sum_fold(sum);
    frame[50] = (tck >> 8) as u8;
    frame[51] = tck as u8;
}

/// Clears `REPORT_ROUTE` on every way out of `report_send`.
#[cfg(feature = "pc")]
struct RouteGuard;
#[cfg(feature = "pc")]
impl Drop for RouteGuard {
    fn drop(&mut self) {
        // SAFETY: a plain store to a static only this file uses, kernel context.
        unsafe { write_volatile(addr_of_mut!(REPORT_ROUTE), None) }
    }
}

/// Hand receive buffers that nobody read back to the card. Frames that arrive
/// while the shell is idle (a home network's broadcasts: printers, phones, the
/// router) sit in the used ring with no reader, and each one is a buffer the
/// card can no longer fill; after `NBUF` of them the card is deaf. Every buffer
/// not currently offered to the device is, in kernel context between commands,
/// one of those. Returns how many were reclaimed.
#[cfg(feature = "pc")]
unsafe fn rx_reclaim() -> usize {
    if NET_BASE == 0 {
        return 0;
    }
    let avail = read_volatile(addr_of!(NET_RX_AVAIL.idx));
    let used = read_volatile(addr_of!(NET_RX_USED.idx));
    let offered = (avail.wrapping_sub(used) as usize).min(NBUF);
    let ring = addr_of!(NET_RX_AVAIL.ring) as *const u16;
    let mut owned = [false; NBUF];
    for j in 0..offered {
        let pos = avail.wrapping_sub(1 + j as u16) as usize % QSZ;
        owned[read_volatile(ring.add(pos)) as usize % NBUF] = true;
    }
    let mut n = 0;
    for (id, o) in owned.iter().enumerate() {
        if !o {
            net_rx_recycle(id as u32);
            n += 1;
        }
    }
    n
}

/// Hand one raw Ethernet frame to the TX queue and wait (briefly) for the device
/// to take it, so the next frame cannot overwrite `TX_BUF` under it.
#[cfg(feature = "pc")]
unsafe fn tx_raw(frame: &[u8]) {
    let tx = addr_of_mut!(TX_BUF) as *mut u8;
    for i in 0..VNET_HDR {
        *tx.add(i) = 0;
    }
    let n = frame.len().min(2048 - VNET_HDR);
    core::ptr::copy_nonoverlapping(frame.as_ptr(), tx.add(VNET_HDR), n);
    let txd = addr_of_mut!(NET_TX_DESC) as *mut Desc;
    write_volatile(txd.add(0), Desc { addr: tx as u64, len: (VNET_HDR + n) as u32, flags: 0, next: 0 });
    let tidx = read_volatile(addr_of!(NET_TX_AVAIL.idx));
    let tring = addr_of_mut!(NET_TX_AVAIL.ring) as *mut u16;
    write_volatile(tring.add((tidx as usize) % QSZ), 0);
    dsb();
    write_volatile(addr_of_mut!(NET_TX_AVAIL.idx), tidx.wrapping_add(1));
    dsb();
    w(NET_BASE, R_QUEUE_NOTIFY, 1);
    tx_settle();
}

/// Wait up to 50 ms for the device to have used every queued TX descriptor.
#[cfg(feature = "pc")]
unsafe fn tx_settle() {
    let end = tcp_rto_deadline(50);
    while read_volatile(addr_of!(NET_TX_USED.idx)) != read_volatile(addr_of!(NET_TX_AVAIL.idx)) {
        if end == 0 || crate::timer::cycles() >= end {
            break;
        }
        core::hint::spin_loop();
    }
}

/// An ARP frame: `op` 1 (who-has `tip`) or 2 (`sip` is-at our MAC, to `tmac`).
#[cfg(feature = "pc")]
unsafe fn arp_frame(op: u8, sip: &[u8; 4], tmac: &[u8; 6], tip: &[u8; 4]) -> [u8; 42] {
    let me = read_volatile(addr_of!(NET_MAC));
    let mut f = [0u8; 42];
    f[0..6].copy_from_slice(if op == 1 { &[0xff; 6] } else { tmac });
    f[6..12].copy_from_slice(&me);
    f[12] = 0x08;
    f[13] = 0x06;
    f[14..22].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, op]);
    f[22..28].copy_from_slice(&me);
    f[28..32].copy_from_slice(sip);
    f[32..38].copy_from_slice(if op == 1 { &[0; 6] } else { tmac });
    f[38..42].copy_from_slice(tip);
    f
}

/// If `fr` is an ARP request for `our_ip`, answer it (the receiver's PC asks who
/// we are before it replies, and asks again whenever its cache entry expires).
#[cfg(feature = "pc")]
unsafe fn arp_answer(fr: *const u8, our_ip: &[u8; 4]) {
    if *fr.add(12) != 0x08 || *fr.add(13) != 0x06 || *fr.add(21) != 1 {
        return;
    }
    let tip = [*fr.add(38), *fr.add(39), *fr.add(40), *fr.add(41)];
    if tip != *our_ip {
        return;
    }
    let mut smac = [0u8; 6];
    let mut sip = [0u8; 4];
    for i in 0..6 {
        smac[i] = *fr.add(22 + i);
    }
    for i in 0..4 {
        sip[i] = *fr.add(28 + i);
    }
    let f = arp_frame(2, our_ip, &smac, &sip);
    tx_raw(&f);
}

/// The MAC of `target` on this wire, asking up to three times a second apart.
#[cfg(feature = "pc")]
unsafe fn arp_resolve(target: &[u8; 4], our_ip: &[u8; 4]) -> Option<[u8; 6]> {
    for _ in 0..3 {
        let mut cur = read_volatile(addr_of!(NET_RX_USED.idx));
        let f = arp_frame(1, our_ip, &[0; 6], target);
        tx_raw(&f);
        let end = tcp_rto_deadline(1000);
        loop {
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                if *fr.add(12) == 0x08 && *fr.add(13) == 0x06 {
                    if *fr.add(21) == 2 && [*fr.add(28), *fr.add(29), *fr.add(30), *fr.add(31)] == *target {
                        let mut m = [0u8; 6];
                        for i in 0..6 {
                            m[i] = *fr.add(22 + i);
                        }
                        RX_COUNT += 1;
                        return Some(m);
                    }
                    arp_answer(fr, our_ip);
                }
                continue;
            }
            if end == 0 || crate::timer::cycles() >= end {
                break;
            }
            core::hint::spin_loop();
        }
    }
    None
}

/// Why a report could not be delivered, in words for the owner.
#[cfg(feature = "pc")]
pub enum ReportErr {
    NoCard,
    NoLease,
    NoRoute,
    NoArp([u8; 4]),
    Refused,
    NoAnswer,
    Stalled(usize),
    Reset(usize),
}

/// What a delivered report took.
#[cfg(feature = "pc")]
pub struct ReportSent {
    pub segments: u32,
    pub retransmits: u32,
    pub closed: bool,
    pub ms: u64,
}

/// Our leased address, mask and gateway, after a DHCP exchange if there is no
/// lease yet. `None` if the router never answered.
#[cfg(feature = "pc")]
pub fn report_lease() -> Option<([u8; 4], [u8; 4], [u8; 4])> {
    // SAFETY: kernel context; the DHCP statics are written only by net_dhcp.
    unsafe {
        if !DHCP_HAVE {
            rx_reclaim();
            net_dhcp();
        }
        if DHCP_HAVE {
            Some((DHCP_IP, DHCP_MASK, DHCP_GW))
        } else {
            None
        }
    }
}

/// Open one TCP connection to `dst_ip`:`dst_port`, send all of `data`, close.
/// Up to 8 segments of `TCP_MAX_PAYLOAD` are in flight; a silent second (then
/// two, four, eight ...) sends again from the first unacknowledged byte. Whatever
/// the far end sends as data is ignored: this is a one-way channel.
#[cfg(feature = "pc")]
pub fn report_send(dst_ip: [u8; 4], dst_port: u16, data: &[u8]) -> Result<ReportSent, ReportErr> {
    const WIN: usize = 8 * TCP_MAX_PAYLOAD;
    const MAX_TRIES: u32 = 6;
    if !net_present() {
        return Err(ReportErr::NoCard);
    }
    let (our_ip, mask, gw) = match report_lease() {
        Some(l) => l,
        None => return Err(ReportErr::NoLease),
    };
    let t0 = crate::timer::cycles();
    // SAFETY: exclusive kernel-context access to the device + rings/buffers.
    unsafe {
        let back = rx_reclaim();
        if back > 0 {
            crate::kprintln!("[report] {} unread network buffer(s) handed back to the card", back);
        }
        let mut on_link = true;
        for i in 0..4 {
            if dst_ip[i] & mask[i] != our_ip[i] & mask[i] {
                on_link = false;
            }
        }
        let hop = if on_link { dst_ip } else { gw };
        if hop == [0; 4] {
            return Err(ReportErr::NoRoute);
        }
        let mac = match arp_resolve(&hop, &our_ip) {
            Some(m) => m,
            None => return Err(ReportErr::NoArp(hop)),
        };
        crate::kprintln!(
            "[report] from {}.{}.{}.{}, {} {}.{}.{}.{} is at {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            our_ip[0], our_ip[1], our_ip[2], our_ip[3],
            if on_link { "receiver" } else { "router" },
            hop[0], hop[1], hop[2], hop[3], mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );
        write_volatile(addr_of_mut!(REPORT_ROUTE), Some((our_ip, mac)));
        let _guard = RouteGuard;
        let base = NET_BASE;
        let dst = &dst_ip;
        let sport: u16 = 41000 + (crate::rng::next_u32() % 20000) as u16;
        let iss = crate::rng::next_u32();
        let mut cur = read_volatile(addr_of!(NET_RX_USED.idx));

        // --- open: SYN, resent after 1, 2, 4 ... s of silence ---
        let mut our_ack = 0u32;
        let mut open = false;
        let mut tries = 0;
        let mut rto = 1000u64;
        while !open {
            if tries >= MAX_TRIES {
                return Err(ReportErr::NoAnswer);
            }
            if tries > 0 {
                // say so: a silent minute reads as a frozen PC
                crate::kprintln!(
                    "[report] no answer from {}.{}.{}.{}:{} yet, asking again ({}/{})",
                    dst[0], dst[1], dst[2], dst[3], dst_port, tries + 1, MAX_TRIES
                );
            }
            tries += 1;
            send_tcp_segment(base, dst, sport, dst_port, iss, 0, 0x02, &[]);
            tx_settle();
            let end = tcp_rto_deadline(rto);
            rto = (rto * 2).min(4000); // 1+2+4+4+4+4 s: about 19 s in all
            loop {
                if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                    let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                    let elem = read_volatile(uring.add((cur as usize) % QSZ));
                    cur = cur.wrapping_add(1);
                    RX_RAW_COUNT += 1;
                    let _slot = RxSlot(elem.id);
                    let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                    arp_answer(fr, &our_ip);
                    if let Some(s) = parse_tcp(fr) {
                        if s.dport == sport && s.sport == dst_port && s.src == *dst {
                            RX_COUNT += 1;
                            if s.flags & 0x04 != 0 {
                                return Err(ReportErr::Refused);
                            }
                            if s.flags & 0x12 == 0x12 && s.ack == iss.wrapping_add(1) {
                                our_ack = s.seq.wrapping_add(1);
                                open = true;
                                break;
                            }
                        }
                    }
                    continue;
                }
                if end == 0 || crate::timer::cycles() >= end {
                    break;
                }
                core::hint::spin_loop();
            }
        }
        let seq0 = iss.wrapping_add(1);
        send_tcp_segment(base, dst, sport, dst_port, seq0, our_ack, 0x10, &[]);
        tx_settle();
        crate::kprintln!(
            "[report] connected to {}.{}.{}.{}:{}, sending {} bytes",
            dst[0], dst[1], dst[2], dst[3], dst_port, data.len()
        );

        // --- stream: go-back-N over a fixed window ---
        let len = data.len();
        let (mut acked, mut next) = (0usize, 0usize);
        let (mut segments, mut retransmits, mut silent) = (0u32, 0u32, 0u32);
        let mut rto = 1000u64;
        let mut end = 0u64;
        let mut peer_fin = false;
        while acked < len {
            while next < len && next - acked < WIN {
                let n = (len - next).min(TCP_MAX_PAYLOAD);
                let seq = seq0.wrapping_add(next as u32);
                send_tcp_segment(base, dst, sport, dst_port, seq, our_ack, 0x18, &data[next..next + n]);
                tx_settle();
                segments += 1;
                next += n;
                end = tcp_rto_deadline(rto);
            }
            if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                let elem = read_volatile(uring.add((cur as usize) % QSZ));
                cur = cur.wrapping_add(1);
                RX_RAW_COUNT += 1;
                let _slot = RxSlot(elem.id);
                let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                arp_answer(fr, &our_ip);
                if let Some(s) = parse_tcp(fr) {
                    if s.dport == sport && s.sport == dst_port && s.src == *dst {
                        RX_COUNT += 1;
                        if s.flags & 0x04 != 0 {
                            return Err(ReportErr::Reset(acked));
                        }
                        if s.flags & 0x01 != 0 && !peer_fin {
                            peer_fin = true;
                            our_ack = s.seq.wrapping_add(s.dlen as u32).wrapping_add(1);
                        }
                        if s.flags & 0x10 != 0 {
                            let a = s.ack.wrapping_sub(seq0) as usize;
                            if a > acked && a <= next {
                                acked = a;
                                silent = 0;
                                rto = 1000;
                                end = tcp_rto_deadline(rto);
                            }
                        }
                    }
                }
                continue;
            }
            if end != 0 && crate::timer::cycles() >= end {
                silent += 1;
                if silent >= MAX_TRIES {
                    return Err(ReportErr::Stalled(acked));
                }
                retransmits += 1;
                crate::kprintln!("[report] no acknowledgement for {} ms, sending again from byte {}", rto, acked);
                next = acked; // go back to the first byte not acknowledged
                rto = (rto * 2).min(8000);
                end = tcp_rto_deadline(rto);
            }
            core::hint::spin_loop();
        }

        // --- close: FIN, then the peer's ACK and FIN (not needed for delivery) ---
        let fin_seq = seq0.wrapping_add(len as u32);
        let mut fin_acked = false;
        let mut tries = 0;
        while tries < 3 && !(fin_acked && peer_fin) {
            tries += 1;
            send_tcp_segment(base, dst, sport, dst_port, fin_seq, our_ack, 0x11, &[]);
            tx_settle();
            let end = tcp_rto_deadline(1000);
            loop {
                if read_volatile(addr_of!(NET_RX_USED.idx)) != cur {
                    let uring = addr_of!(NET_RX_USED.ring) as *const UsedElem;
                    let elem = read_volatile(uring.add((cur as usize) % QSZ));
                    cur = cur.wrapping_add(1);
                    RX_RAW_COUNT += 1;
                    let _slot = RxSlot(elem.id);
                    let fr = (addr_of!(RX_BUFS[(elem.id as usize) % NBUF]) as *const u8).add(VNET_HDR);
                    arp_answer(fr, &our_ip);
                    if let Some(s) = parse_tcp(fr) {
                        if s.dport == sport && s.sport == dst_port && s.src == *dst {
                            RX_COUNT += 1;
                            if s.flags & 0x04 != 0 {
                                break;
                            }
                            if s.flags & 0x10 != 0 && s.ack == fin_seq.wrapping_add(1) {
                                fin_acked = true;
                            }
                            if s.flags & 0x01 != 0 && !peer_fin {
                                peer_fin = true;
                                our_ack = s.seq.wrapping_add(s.dlen as u32).wrapping_add(1);
                            }
                            if peer_fin {
                                send_tcp_segment(base, dst, sport, dst_port, fin_seq.wrapping_add(1), our_ack, 0x10, &[]);
                                tx_settle();
                            }
                            if fin_acked && peer_fin {
                                break;
                            }
                        }
                    }
                    continue;
                }
                if end == 0 || crate::timer::cycles() >= end {
                    break;
                }
                core::hint::spin_loop();
            }
        }
        let hz = crate::timer::hz().max(1);
        Ok(ReportSent {
            segments,
            retransmits,
            closed: fin_acked && peer_fin,
            ms: crate::timer::cycles().wrapping_sub(t0) * 1000 / hz,
        })
    }
}

// ---------------------------------------------------------------------------
// Segment-layer self-test. Every case runs `build_tcp_frame` against synthetic
// inputs and verifies the frame the way a PEER would — recomputing both
// checksums over the built bytes and requiring them to fold to zero — so it
// checks the bytes rather than re-deriving them the way the builder did.
// This runs on any boot, network or not.
// ---------------------------------------------------------------------------

/// Recompute a built frame's IPv4 and TCP checksums the receiver's way. Both
/// must fold to zero over a frame that already carries its checksum.
fn verify_tcp_frame(frame: &[u8]) -> bool {
    if frame.len() < TCP_FRAME_MIN {
        return false;
    }
    if inet_sum_fold(inet_sum_add(0, &frame[14..34])) != 0 {
        return false;
    }
    let ip_total = ((frame[16] as usize) << 8) | (frame[17] as usize);
    if ip_total + 14 != frame.len() {
        return false;
    }
    let seg_len = ip_total - 20;
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(&frame[26..30]);
    pseudo[4..8].copy_from_slice(&frame[30..34]);
    pseudo[9] = 6;
    pseudo[10] = (seg_len >> 8) as u8;
    pseudo[11] = seg_len as u8;
    let mut sum = inet_sum_add(0, &pseudo);
    sum = inet_sum_add(sum, &frame[34..34 + 20]);
    sum = inet_sum_add(sum, &frame[54..]);
    inet_sum_fold(sum) == 0
}

/// Run the segment builder against synthetic segments. Prints one line per case
/// and a tally; returns (passed, total).
pub fn tcp_seg_selftest() -> (u32, u32) {
    let dmac = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02];
    let smac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    let mut buf = [0u8; 12 + TCP_FRAME_MIN + TCP_MAX_PAYLOAD];
    let mut pass = 0u32;
    let mut total = 0u32;
    let case = |name: &str, ok: bool, pass: &mut u32, total: &mut u32| {
        *total += 1;
        if ok {
            *pass += 1;
        }
        crate::kprintln!("[tcpseg] {:<34} {}", name, if ok { "ok" } else { "FAILED" });
    };

    // 1. A data-less SYN is exactly the 54-byte frame the handshake sends.
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40001, 80, 0x1234_5678, 0, 0x02, &[]);
    let ok = n == 54
        && buf[12] == 0x08
        && buf[13] == 0x00
        && buf[14 + 9] == 6
        && buf[34 + 13] == 0x02
        && ((buf[34] as u16) << 8 | buf[35] as u16) == 40001
        && ((buf[36] as u16) << 8 | buf[37] as u16) == 80
        && verify_tcp_frame(&buf[..n]);
    case("SYN, no payload", ok, &mut pass, &mut total);

    // 2. Sequence and ack land where the peer reads them.
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40001, 80, 0xdead_beef, 0xfeed_face, 0x10, &[]);
    let seq = (buf[38] as u32) << 24 | (buf[39] as u32) << 16 | (buf[40] as u32) << 8 | buf[41] as u32;
    let ack = (buf[42] as u32) << 24 | (buf[43] as u32) << 16 | (buf[44] as u32) << 8 | buf[45] as u32;
    case(
        "ACK seq/ack big-endian",
        n == 54 && seq == 0xdead_beef && ack == 0xfeed_face && verify_tcp_frame(&buf[..n]),
        &mut pass, &mut total,
    );

    // 3. An even payload: length, IP total length, and the bytes themselves.
    let data = b"AFRISOVERA";
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 1, 1, 0x18, data);
    let ip_total = (buf[16] as usize) << 8 | buf[17] as usize;
    case(
        "10-byte payload (even)",
        n == 64 && ip_total == 50 && buf[54..64] == data[..] && verify_tcp_frame(&buf[..n]),
        &mut pass, &mut total,
    );

    // 4. An ODD payload — the case the pad byte exists for, and the one a
    //    data-less builder never had to be right about.
    let data = b"odd!";
    let odd: [u8; 5] = [data[0], data[1], data[2], data[3], b'?'];
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 7, 9, 0x18, &odd);
    case(
        "5-byte payload (odd, padded)",
        n == 59 && buf[54..59] == odd[..] && verify_tcp_frame(&buf[..n]),
        &mut pass, &mut total,
    );

    // 5. A payload whose bytes sum to zero in ones-complement is the shape that
    //    makes a checksum look right for the wrong reason.
    let ffff = [0xffu8, 0xff, 0x00, 0x00];
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 3, 4, 0x18, &ffff);
    case(
        "payload of 0xffff words",
        n == 58 && verify_tcp_frame(&buf[..n]),
        &mut pass, &mut total,
    );

    // 6. The largest payload still builds and still verifies.
    let big = [0x5au8; TCP_MAX_PAYLOAD];
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 11, 12, 0x18, &big);
    case(
        "TCP_MAX_PAYLOAD builds",
        n == TCP_FRAME_MIN + TCP_MAX_PAYLOAD && verify_tcp_frame(&buf[..n]),
        &mut pass, &mut total,
    );

    // 7. One byte over is refused, not truncated.
    let over = [0x5au8; TCP_MAX_PAYLOAD + 1];
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 11, 12, 0x18, &over);
    case("payload over the cap refused", n == 0, &mut pass, &mut total);

    // 8. So is a buffer that cannot hold the frame.
    let mut small = [0u8; 53];
    let n = build_tcp_frame(&mut small, &dmac, &smac, &GW_IP, 40002, 9099, 1, 1, 0x02, &[]);
    case("undersized buffer refused", n == 0, &mut pass, &mut total);

    // 9. The verifier has to be able to FAIL, or cases 1-6 prove nothing: flip
    //    one payload byte after the checksum was computed over the old one.
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 1, 1, 0x18, b"AFRISOVERA");
    let good = verify_tcp_frame(&buf[..n]);
    buf[54] ^= 0x01;
    let bad = verify_tcp_frame(&buf[..n]);
    case("a flipped byte fails the check", good && !bad, &mut pass, &mut total);

    // 10. And a corrupted IP header is caught by the other checksum.
    let n = build_tcp_frame(&mut buf, &dmac, &smac, &GW_IP, 40002, 9099, 1, 1, 0x18, b"AFRISOVERA");
    buf[22] ^= 0x08; // TTL
    case("a flipped IP header byte fails", !verify_tcp_frame(&buf[..n]), &mut pass, &mut total);

    crate::kprintln!("[tcpseg] {}/{} passed", pass, total);
    (pass, total)
}

// ===========================================================================
// virtio-blk (device id 2): persistent block storage. Reuses the split-virtqueue
// machinery. A request is a 3-descriptor chain: a 16-byte header (device-read), a
// 512-byte data buffer, and a 1-byte status (device-write). One request queue (0).
// ===========================================================================

const BLK_T_IN: u32 = 0; // read (device -> memory)
const BLK_T_OUT: u32 = 1; // write (memory -> device)

static mut BLK_BASE: u64 = 0;
static mut BLK_CAP: u64 = 0; // capacity in 512-byte sectors
static mut BLK_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut BLK_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut BLK_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };
static mut BLK_HDR: [u8; 16] = [0; 16];
static mut BLK_DATA: [u8; 512] = [0; 512];
static mut BLK_STATUS: [u8; 1] = [0; 1];

fn blk_setup(base: u64) -> bool {
    // SAFETY: MMIO on a detected virtio-blk device; single-threaded bring-up.
    unsafe {
        w(base, R_STATUS, 0);
        let mut st = S_ACK;
        w(base, R_STATUS, st);
        st |= S_DRIVER;
        w(base, R_STATUS, st);
        w(base, R_DRIVER_FEATURES_SEL, 0);
        w(base, R_DRIVER_FEATURES, 0);
        w(base, R_DRIVER_FEATURES_SEL, 1);
        w(base, R_DRIVER_FEATURES, 1); // VIRTIO_F_VERSION_1
        st |= S_FEATURES_OK;
        w(base, R_STATUS, st);
        if r(base, R_STATUS) & S_FEATURES_OK == 0 {
            crate::kprintln!("[virtio-blk] FEATURES_OK rejected");
            w(base, R_STATUS, S_FAILED);
            return false;
        }
        // Capacity: 8-byte LE sector count at config offset 0.
        let mut cap = 0u64;
        for i in 0..8 {
            cap |= (read_volatile((base + R_CONFIG + i as u64) as *const u8) as u64) << (i * 8);
        }
        BLK_CAP = cap;
        queue_setup(
            base,
            0,
            addr_of_mut!(BLK_DESC) as u64,
            addr_of_mut!(BLK_AVAIL) as u64,
            addr_of_mut!(BLK_USED) as u64,
        );
        st |= S_DRIVER_OK;
        w(base, R_STATUS, st);
        BLK_BASE = base;
        crate::kprintln!(
            "[virtio-blk] up @ {:#010x}, capacity {} sectors ({} KiB)",
            base, cap, cap * 512 / 1024
        );
        true
    }
}

/// Issue one block request (read or write) for `sector`; returns the status byte
/// (0 = OK). Data flows through BLK_DATA.
unsafe fn blk_request(kind: u32, sector: u64) -> u8 {
    let base = BLK_BASE;
    let hdr = addr_of_mut!(BLK_HDR) as *mut u8;
    wr32(hdr, 0, kind);
    wr32(hdr, 4, 0);
    let sb = sector.to_le_bytes();
    for i in 0..8 {
        *hdr.add(8 + i) = sb[i];
    }
    *(addr_of_mut!(BLK_STATUS) as *mut u8) = 0xff; // sentinel

    let d = addr_of_mut!(BLK_DESC) as *mut Desc;
    write_volatile(d.add(0), Desc { addr: addr_of_mut!(BLK_HDR) as u64, len: 16, flags: F_NEXT, next: 1 });
    // read: device writes the data buffer; write: device reads it.
    let data_flags = if kind == BLK_T_IN { F_WRITE | F_NEXT } else { F_NEXT };
    write_volatile(d.add(1), Desc { addr: addr_of_mut!(BLK_DATA) as u64, len: 512, flags: data_flags, next: 2 });
    write_volatile(d.add(2), Desc { addr: addr_of_mut!(BLK_STATUS) as u64, len: 1, flags: F_WRITE, next: 0 });

    let avail = addr_of_mut!(BLK_AVAIL);
    let idx = read_volatile(addr_of!((*avail).idx));
    let ring = addr_of_mut!((*avail).ring) as *mut u16;
    write_volatile(ring.add((idx as usize) % QSZ), 0);
    dsb();
    write_volatile(addr_of_mut!((*avail).idx), idx.wrapping_add(1));
    dsb();
    let prev = read_volatile(addr_of!(BLK_USED.idx));
    w(base, R_QUEUE_NOTIFY, 0);
    let mut spins = 0u64;
    while read_volatile(addr_of!(BLK_USED.idx)) == prev {
        spins += 1;
        if spins > 60_000_000 {
            crate::kprintln!("[virtio-blk] request timeout");
            return 0xff;
        }
    }
    read_volatile(addr_of!(BLK_STATUS) as *const u8)
}

/// Read sector `n` into `out`. Returns true on success.
pub fn read_sector(n: u64, out: &mut [u8; 512]) -> bool {
    if !blk_present() {
        return false;
    }
    // SAFETY: exclusive kernel-context access to the device + buffers.
    unsafe {
        if blk_request(BLK_T_IN, n) != 0 {
            return false;
        }
        let src = addr_of!(BLK_DATA) as *const u8;
        for i in 0..512 {
            out[i] = *src.add(i);
        }
        true
    }
}

/// Write `data` to sector `n`. Returns true on success.
pub fn write_sector(n: u64, data: &[u8; 512]) -> bool {
    if !blk_present() {
        return false;
    }
    // SAFETY: exclusive kernel-context access to the device + buffers.
    unsafe {
        let dst = addr_of_mut!(BLK_DATA) as *mut u8;
        for i in 0..512 {
            *dst.add(i) = data[i];
        }
        blk_request(BLK_T_OUT, n) == 0
    }
}

pub fn blk_present() -> bool {
    // SAFETY: scalar read.
    unsafe { BLK_BASE != 0 }
}
pub fn blk_capacity() -> u64 {
    // SAFETY: scalar read.
    unsafe { BLK_CAP }
}

/// Write a known pattern to a sector, read it back, and confirm it matches —
/// proof that persistent block I/O works end to end.
pub fn blk_selftest() -> bool {
    if !blk_present() {
        return false;
    }
    let sector = 100u64;
    let mut pat = [0u8; 512];
    for i in 0..512 {
        pat[i] = (i as u8) ^ 0x5a;
    }
    if !write_sector(sector, &pat) {
        crate::kprintln!("[virtio-blk] self-test: write failed");
        return false;
    }
    let mut rb = [0u8; 512];
    if !read_sector(sector, &mut rb) {
        crate::kprintln!("[virtio-blk] self-test: read failed");
        return false;
    }
    if rb == pat {
        crate::kprintln!("[virtio-blk] blk self-test OK (wrote+read sector {})", sector);
        true
    } else {
        crate::kprintln!("[virtio-blk] blk self-test FAILED (data mismatch)");
        false
    }
}

// ===========================================================================
// virtio-rng (device id 4): a hardware entropy source. One virtqueue; each
// request is a single device-WRITABLE buffer the device fills with random
// bytes. Reuses the split-virtqueue machinery. Used to strengthen the RNG seed
// (rng.rs) with real device entropy when the host provides -device
// virtio-rng-device; the OS boots fine without it (falls back to timer entropy).
// ===========================================================================

static mut RNG_BASE: u64 = 0;
static mut RNG_DESC: DescTable = DescTable { d: [DESC0; QSZ] };
static mut RNG_AVAIL: AvailRing = AvailRing { flags: 0, idx: 0, ring: [0; QSZ] };
static mut RNG_USED: UsedRing = UsedRing { flags: 0, idx: 0, ring: [UsedElem { id: 0, len: 0 }; QSZ] };
static mut RNG_BUF: [u8; 64] = [0; 64];

/// Bring up the virtio entropy device: negotiate VERSION_1, arm one queue.
fn rng_setup(base: u64) -> bool {
    // SAFETY: MMIO on a detected virtio-rng device; single-threaded bring-up.
    unsafe {
        w(base, R_STATUS, 0);
        let mut st = S_ACK;
        w(base, R_STATUS, st);
        st |= S_DRIVER;
        w(base, R_STATUS, st);
        w(base, R_DRIVER_FEATURES_SEL, 0);
        w(base, R_DRIVER_FEATURES, 0);
        w(base, R_DRIVER_FEATURES_SEL, 1);
        w(base, R_DRIVER_FEATURES, 1); // VIRTIO_F_VERSION_1
        st |= S_FEATURES_OK;
        w(base, R_STATUS, st);
        if r(base, R_STATUS) & S_FEATURES_OK == 0 {
            crate::kprintln!("[virtio-rng] FEATURES_OK rejected");
            w(base, R_STATUS, S_FAILED);
            return false;
        }
        queue_setup(
            base,
            0,
            addr_of_mut!(RNG_DESC) as u64,
            addr_of_mut!(RNG_AVAIL) as u64,
            addr_of_mut!(RNG_USED) as u64,
        );
        st |= S_DRIVER_OK;
        w(base, R_STATUS, st);
        RNG_BASE = base;
        crate::kprintln!("[virtio-rng] up @ {:#010x} (hardware entropy source)", base);
        true
    }
}

/// True if a virtio entropy device was negotiated.
pub fn rng_present() -> bool {
    // SAFETY: scalar read.
    unsafe { RNG_BASE != 0 }
}

/// Fill `buf` (up to 64 bytes) with random bytes from the virtio entropy device.
/// Returns the number of bytes obtained, or 0 if there is no device / on timeout.
pub fn virtio_entropy(buf: &mut [u8]) -> usize {
    if !rng_present() {
        return 0;
    }
    // SAFETY: exclusive kernel-context access to the device + its ring/buffer.
    unsafe {
        let base = RNG_BASE;
        let want = buf.len().min(64) as u32;
        // A single device-writable descriptor pointing at RNG_BUF.
        let d = addr_of_mut!(RNG_DESC) as *mut Desc;
        write_volatile(
            d.add(0),
            Desc { addr: addr_of_mut!(RNG_BUF) as u64, len: want, flags: F_WRITE, next: 0 },
        );
        let avail = addr_of_mut!(RNG_AVAIL);
        let idx = read_volatile(addr_of!((*avail).idx));
        let ring = addr_of_mut!((*avail).ring) as *mut u16;
        write_volatile(ring.add((idx as usize) % QSZ), 0);
        dsb();
        write_volatile(addr_of_mut!((*avail).idx), idx.wrapping_add(1));
        dsb();
        let prev = read_volatile(addr_of!(RNG_USED.idx));
        w(base, R_QUEUE_NOTIFY, 0);
        let mut spins = 0u64;
        while read_volatile(addr_of!(RNG_USED.idx)) == prev {
            spins += 1;
            if spins > 60_000_000 {
                return 0;
            }
        }
        // The used element's `len` is how many bytes the device wrote.
        let uring = addr_of!(RNG_USED.ring) as *const UsedElem;
        let elem = read_volatile(uring.add((prev as usize) % QSZ));
        let got = (elem.len as usize).min(buf.len()).min(64);
        let src = addr_of!(RNG_BUF) as *const u8;
        for i in 0..got {
            buf[i] = *src.add(i);
        }
        got
    }
}
