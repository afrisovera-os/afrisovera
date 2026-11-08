//! Intel High Definition Audio on the PC: roadmap PC step 3, sound (7 Nov 2026).
//!
//! Nearly every PC and laptop since 2004 has an HD Audio controller (PCI class 04/03) with
//! one or more codec chips behind it (Realtek ALC, Conexant, IDT, Cirrus ... on real PCs;
//! QEMU's `hda-output` / `hda-duplex` in the emulator). This driver:
//!   * resets the controller and finds its codecs (STATESTS),
//!   * talks to them through the CORB/RIRB command rings (polled, interrupts left off),
//!   * walks the codec's audio function group, picks an output pin (speaker, then
//!     headphone, then line out; digital/HDMI pins are skipped) and follows its connection
//!     list down to a DAC, powering, selecting and unmuting every node on the way,
//!   * plays 44.1 kHz 16-bit stereo through the first output stream: a 64 KiB cyclic
//!     buffer of 16 periods that is refilled behind the DMA position (LPIB).
//! The shared sound calls (`crate::virtio::sound_*`, used by `audio`, `say` and Nia's
//! `speak()`) come here through pc/disk.rs when the PC has no virtio-sound device.
//! Intel "High Definition Audio Specification" rev 1.0a: registers 3.3, CORB/RIRB 4.4,
//! streams 3.3.35 / 4.5, verbs and parameters 7.3.

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

// controller registers (spec 3.3)
const GCAP: u64 = 0x00; // u16
const VMIN: u64 = 0x02; // u8
const VMAJ: u64 = 0x03; // u8
const GCTL: u64 = 0x08; // u32, bit 0 CRST
const STATESTS: u64 = 0x0E; // u16, one bit per codec that answered the reset
const INTCTL: u64 = 0x20; // u32
const CORBLBASE: u64 = 0x40;
const CORBUBASE: u64 = 0x44;
const CORBWP: u64 = 0x48; // u16
const CORBRP: u64 = 0x4A; // u16, bit 15 = reset
const CORBCTL: u64 = 0x4C; // u8, bit 1 = run
const CORBSIZE: u64 = 0x4E; // u8
const RIRBLBASE: u64 = 0x50;
const RIRBUBASE: u64 = 0x54;
const RIRBWP: u64 = 0x58; // u16, bit 15 = reset
const RINTCNT: u64 = 0x5A; // u16
const RIRBCTL: u64 = 0x5C; // u8, bit 0 response interrupt, bit 1 DMA run
const RIRBSTS: u64 = 0x5D; // u8
const RIRBSIZE: u64 = 0x5E; // u8
// stream descriptor registers, from the descriptor's base (spec 3.3.35)
const SD_CTL: u64 = 0x00; // CTL (3 bytes) + STS (1 byte), used as one u32
const SD_LPIB: u64 = 0x04;
const SD_CBL: u64 = 0x08;
const SD_LVI: u64 = 0x0C; // u16
const SD_FMT: u64 = 0x12; // u16
const SD_BDPL: u64 = 0x18;
const SD_BDPU: u64 = 0x1C;

// verbs (spec 7.3.3) and parameters (7.3.4)
const GET_PARAM: u32 = 0xF00;
const GET_CONN: u32 = 0xF02;
const SET_SEL: u32 = 0x701;
const SET_POWER: u32 = 0x705;
const SET_STREAM: u32 = 0x706;
const SET_PINCTL: u32 = 0x707;
const SET_EAPD: u32 = 0x70C;
const GET_CFG: u32 = 0xF1C;
const SET_FMT4: u32 = 0x2; // 4-bit verb, 16-bit payload
const SET_AMP4: u32 = 0x3;
const P_VENDOR: u32 = 0x00;
const P_NODES: u32 = 0x04;
const P_FGTYPE: u32 = 0x05;
const P_WCAPS: u32 = 0x09;
const P_PCM: u32 = 0x0A;
const P_PINCAPS: u32 = 0x0C;
const P_INAMP: u32 = 0x0D;
const P_CONNLEN: u32 = 0x0E;
const P_OUTAMP: u32 = 0x12;

const W_OUT: u32 = 0; // audio output (DAC)
const W_MIXER: u32 = 2;
const W_SELECTOR: u32 = 3;
const W_PIN: u32 = 4;

/// 44.1 kHz (base 44.1, x1, /1), 16 bits, 2 channels -- what voice.rs and the tone make.
const FMT_44K1_16_STEREO: u16 = 0x4011;
const FMT_48K_16_STEREO: u16 = 0x0011;
const RATE_HZ: u32 = 44100;
const STREAM_TAG: u32 = 1;
const PERIOD: usize = 4096; // bytes = 1024 stereo S16 frames, as virtio-sound's period
const NPERIODS: usize = 16;
const RING: usize = PERIOD * NPERIODS;

#[repr(C, align(1024))]
struct Corb([u32; 256]);
#[repr(C, align(2048))]
struct Rirb([u64; 256]);
#[derive(Clone, Copy)]
#[repr(C)]
struct Bde {
    addr: u64,
    len: u32,
    ioc: u32,
}
#[repr(C, align(128))]
struct Bdl([Bde; NPERIODS]);
#[repr(C, align(4096))]
struct Ring([u8; RING]);

static mut CORB: Corb = Corb([0; 256]);
static mut RIRB: Rirb = Rirb([0; 256]);
static mut BDL: Bdl = Bdl([Bde { addr: 0, len: 0, ioc: 0 }; NPERIODS]);
static mut RINGBUF: Ring = Ring([0; RING]);
static mut STAGE: [u8; PERIOD] = [0; PERIOD];

static mut BAR: u64 = 0;
static mut READY: bool = false;
static mut CORB_N: u16 = 256;
static mut RIRB_N: u16 = 256;
static mut RIRB_RP: u16 = 0;
static mut SD: u64 = 0; // the output stream descriptor's offset
static mut PCI_AT: (u8, u8, u8) = (0, 0, 0);
static mut PCI_ID: (u16, u16) = (0, 0);
static mut VER: (u8, u8) = (0, 0);
static mut STREAMS: (u8, u8) = (0, 0); // output, input
static mut CODECS: u16 = 0;
static mut CAD: u32 = 0;
static mut CODEC_ID: u32 = 0;
static mut AFG: u8 = 0;
static mut FMT: u16 = FMT_44K1_16_STEREO;
static mut WHY: &str = "";

// what the codec walk found (for lshda) and the chosen path, pin first
const MAXW: usize = 96;
#[derive(Clone, Copy)]
struct Widget {
    nid: u8,
    caps: u32,
    pincaps: u32,
    cfg: u32,
    nconn: u8,
    conn: [u8; 8],
}
const W0: Widget = Widget { nid: 0, caps: 0, pincaps: 0, cfg: 0, nconn: 0, conn: [0; 8] };
static mut WS: [Widget; MAXW] = [W0; MAXW];
static mut NW: usize = 0;
static mut PATH: [(u8, u8); 6] = [(0, 0); 6]; // (node, connection index used to reach the next)
static mut PATH_LEN: usize = 0;

// the stream
static mut RUNNING: bool = false;
static mut WRITTEN: u64 = 0; // bytes put in the ring since the stream began
static mut WRAPS: u64 = 0;
static mut LAST_LPIB: u64 = 0;
static mut FILL: usize = 0; // stereo frames staged in STAGE
static mut PERIODS: u32 = 0; // periods of real sound this stream
static mut SAMPLES: u64 = 0;

fn r8(off: u64) -> u8 {
    // SAFETY: BAR0 is mapped uncached by init before any register access.
    unsafe { read_volatile((BAR + off) as *const u8) }
}
fn r16(off: u64) -> u16 {
    // SAFETY: as r8.
    unsafe { read_volatile((BAR + off) as *const u16) }
}
fn r32(off: u64) -> u32 {
    // SAFETY: as r8.
    unsafe { read_volatile((BAR + off) as *const u32) }
}
fn w8(off: u64, v: u8) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u8, v) }
}
fn w16(off: u64, v: u16) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u16, v) }
}
fn w32(off: u64, v: u32) {
    // SAFETY: as r8.
    unsafe { write_volatile((BAR + off) as *mut u32, v) }
}

fn us(n: u64) -> u64 {
    crate::timer::hz().max(1) * n / 1_000_000
}
fn wait_us(n: u64, mut done: impl FnMut() -> bool) -> bool {
    let t0 = crate::timer::cycles();
    let lim = us(n);
    loop {
        if done() {
            return true;
        }
        if crate::timer::cycles().wrapping_sub(t0) > lim {
            return done();
        }
        core::hint::spin_loop();
    }
}
fn delay_us(n: u64) {
    wait_us(n, || false);
}

fn controller_name(v: u16, d: u16) -> &'static str {
    match (v, d) {
        (0x8086, 0x2668) => "Intel ICH6 HD Audio",
        (0x8086, 0x27D8) => "Intel ICH7 HD Audio",
        (0x8086, 0x284B) => "Intel ICH8 HD Audio",
        (0x8086, 0x293E) => "Intel ICH9 HD Audio",
        (0x8086, 0x3A3E) | (0x8086, 0x3A6E) => "Intel ICH10 HD Audio",
        (0x8086, _) => "Intel HD Audio",
        (0x1022, _) | (0x1002, _) => "AMD HD Audio",
        (0x10DE, _) => "NVIDIA HD Audio (HDMI)",
        (0x10EC, _) => "Realtek HD Audio",
        _ => "HD Audio",
    }
}

fn codec_vendor(id: u32) -> &'static str {
    match id >> 16 {
        0x10EC => "Realtek",
        0x14F1 => "Conexant",
        0x111D | 0x8384 => "IDT/SigmaTel",
        0x1013 => "Cirrus Logic",
        0x11D4 => "Analog Devices",
        0x1106 => "VIA",
        0x8086 => "Intel (HDMI)",
        0x1002 | 0x1022 => "AMD",
        0x10DE => "NVIDIA (HDMI)",
        0x1AF4 => "QEMU",
        _ => "unknown maker",
    }
}

/// Send one verb to the codec at CAD and wait for its answer (CORB/RIRB, polled).
fn cmd(nid: u8, verb: u32, payload: u32) -> Option<u32> {
    // SAFETY: single core; the rings are ours, the controller reads/writes them by DMA.
    unsafe {
        let v = if verb <= 0xF { CAD << 28 | (nid as u32) << 20 | verb << 16 | (payload & 0xFFFF) } else {
            CAD << 28 | (nid as u32) << 20 | verb << 8 | (payload & 0xFF)
        };
        let wp = ((r16(CORBWP) & 0xFF) + 1) % CORB_N;
        write_volatile(addr_of_mut!(CORB.0[wp as usize]), v);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        w16(CORBWP, wp);
        let t0 = crate::timer::cycles();
        while crate::timer::cycles().wrapping_sub(t0) < us(100_000) {
            if r16(RIRBWP) & 0xFF == RIRB_RP {
                core::hint::spin_loop();
                continue;
            }
            let rp = (RIRB_RP + 1) % RIRB_N;
            let e = read_volatile(addr_of!(RIRB.0[rp as usize]));
            RIRB_RP = rp;
            w8(RIRBSTS, 0x05); // response + overrun flags cleared (QEMU waits for this)
            let ex = (e >> 32) as u32;
            if ex & 0x10 != 0 || ex & 0xF != CAD {
                continue; // an unsolicited response, or another codec's
            }
            return Some(e as u32);
        }
        None
    }
}
fn param(nid: u8, p: u32) -> u32 {
    cmd(nid, GET_PARAM, p).unwrap_or(0)
}

/// Find the first HD Audio controller with a codec that has an analog output, reset it
/// and set up the output path and stream. True when sound can be played.
pub fn init() -> bool {
    let mut found = false;
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
                if cfg32(b, d, f, 8) >> 16 != 0x0403 {
                    continue;
                }
                found = true;
                // SAFETY: boot, single core; the device is ours from here
                if unsafe { bring_up(b, d, f, id as u16, (id >> 16) as u16) } {
                    break 'scan;
                }
            }
        }
    }
    // SAFETY: boot, single core.
    unsafe {
        if READY {
            let (b, d, f) = PCI_AT;
            crate::kprintln!(
                "[hda] {} at {:02x}:{:02x}.{} ({:04x}:{:04x}), codec {} {:04x}:{:04x}, sound out through {}",
                controller_name(PCI_ID.0, PCI_ID.1), b, d, f, PCI_ID.0, PCI_ID.1, CAD,
                CODEC_ID >> 16, CODEC_ID & 0xFFFF, PathText
            );
        } else if found {
            crate::kprintln!("[hda] HD Audio controller found but not usable: {}", WHY);
        }
        READY
    }
}

/// The chosen path as text: "node 3 (line out) <- DAC 2".
#[allow(non_upper_case_globals)]
const PathText: PathFmt = PathFmt;
struct PathFmt;
impl core::fmt::Display for PathFmt {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        // SAFETY: read of boot-time state.
        unsafe {
            for i in 0..PATH_LEN {
                let n = PATH[i].0;
                let w = widget(n);
                if i > 0 {
                    write!(f, " <- ")?;
                }
                match (w.caps >> 20) & 0xF {
                    W_PIN => write!(f, "pin {} ({})", n, pin_kind(w.cfg))?,
                    W_OUT => write!(f, "DAC {}", n)?,
                    W_MIXER => write!(f, "mixer {}", n)?,
                    W_SELECTOR => write!(f, "selector {}", n)?,
                    _ => write!(f, "node {}", n)?,
                }
            }
        }
        Ok(())
    }
}

fn widget(nid: u8) -> Widget {
    // SAFETY: read of boot-time state.
    unsafe {
        for i in 0..NW {
            if WS[i].nid == nid {
                return WS[i];
            }
        }
    }
    W0
}

fn pin_kind(cfg: u32) -> &'static str {
    match (cfg >> 20) & 0xF {
        0 => "line out",
        1 => "speaker",
        2 => "headphones",
        3 => "CD",
        4 => "S/PDIF out",
        5 => "digital out",
        6 => "modem line",
        7 => "modem handset",
        8 => "line in",
        9 => "aux",
        0xA => "microphone",
        0xB => "telephony",
        0xC => "S/PDIF in",
        0xD => "digital in",
        _ => "other",
    }
}

/// Lower is better; None = not an analog output we use.
fn pin_rank(w: &Widget) -> Option<u8> {
    if w.pincaps & (1 << 4) == 0 || w.pincaps & (1 << 7 | 1 << 24) != 0 {
        return None; // not output capable, or HDMI / DisplayPort
    }
    if w.cfg >> 30 == 1 {
        return None; // "no physical connection" (an unused pin on this board)
    }
    match (w.cfg >> 20) & 0xF {
        1 => Some(0),
        2 => Some(1),
        0 => Some(2),
        _ => None,
    }
}

unsafe fn bring_up(b: u8, d: u8, f: u8, ven: u16, dev: u16) -> bool {
    let lo = cfg32(b, d, f, 0x10);
    let mut bar = (lo & !0xF) as u64;
    if lo & 0x6 == 0x4 {
        bar |= (cfg32(b, d, f, 0x14) as u64) << 32;
    }
    if bar == 0 || !mmu::map_mmio(bar, 0x4000) {
        WHY = "no usable BAR0";
        return false;
    }
    cfg_write32(b, d, f, 4, cfg32(b, d, f, 4) | 0x406); // memory + bus mastering, INTx off (polled)
    if ven == 0x8086 {
        // Intel: traffic class 0 for the stream DMA (TCSEL, as Linux does), or some boards crackle
        cfg_write32(b, d, f, 0x44, cfg32(b, d, f, 0x44) & !0x7);
    }
    BAR = bar;
    PCI_AT = (b, d, f);
    PCI_ID = (ven, dev);
    READY = false;
    // stop the rings, then reset the controller (CRST 0 -> 1) so every codec announces itself
    w8(CORBCTL, 0);
    w8(RIRBCTL, 0);
    wait_us(10_000, || r8(CORBCTL) & 2 == 0 && r8(RIRBCTL) & 2 == 0);
    w32(INTCTL, 0);
    w32(GCTL, r32(GCTL) & !1);
    if !wait_us(100_000, || r32(GCTL) & 1 == 0) {
        WHY = "did not go into reset";
        return false;
    }
    delay_us(1000);
    w32(GCTL, r32(GCTL) | 1);
    if !wait_us(100_000, || r32(GCTL) & 1 == 1) {
        WHY = "did not come out of reset";
        return false;
    }
    delay_us(2000); // codecs ask for an address within 521 us of the reset ending
    let gcap = r16(GCAP);
    VER = (r8(VMAJ), r8(VMIN));
    STREAMS = (((gcap >> 12) & 0xF) as u8, ((gcap >> 8) & 0xF) as u8);
    CODECS = r16(STATESTS) & 0x7FFF;
    w16(STATESTS, CODECS);
    if CODECS == 0 {
        WHY = "no codec answered the reset";
        return false;
    }
    if STREAMS.0 == 0 {
        WHY = "the controller has no output stream";
        return false;
    }
    let (corb, rirb, bdl, ring) = (addr_of!(CORB) as u64, addr_of!(RIRB) as u64, addr_of!(BDL) as u64, addr_of!(RINGBUF) as u64);
    if gcap & 1 == 0 && (corb | rirb | bdl | ring) >> 32 != 0 {
        WHY = "buffers above 4 GiB and the controller has only 32-bit DMA";
        return false;
    }
    // CORB: the command ring (spec 4.4.1)
    let sz = |cap: u8| -> (u8, u16) {
        if cap & 0x4 != 0 { (2, 256) } else if cap & 0x2 != 0 { (1, 16) } else { (0, 2) }
    };
    w32(CORBLBASE, corb as u32);
    w32(CORBUBASE, (corb >> 32) as u32);
    let (code, n) = sz(r8(CORBSIZE) >> 4);
    w8(CORBSIZE, (r8(CORBSIZE) & !3) | code);
    CORB_N = n;
    w16(CORBWP, 0);
    w16(CORBRP, 0x8000);
    wait_us(1000, || r16(CORBRP) & 0x8000 != 0); // some controllers never show the bit
    w16(CORBRP, 0);
    if !wait_us(10_000, || r16(CORBRP) & 0x8000 == 0) {
        WHY = "the command ring pointer did not reset";
        return false;
    }
    // RIRB: the response ring (spec 4.4.2)
    w32(RIRBLBASE, rirb as u32);
    w32(RIRBUBASE, (rirb >> 32) as u32);
    let (code, n) = sz(r8(RIRBSIZE) >> 4);
    w8(RIRBSIZE, (r8(RIRBSIZE) & !3) | code);
    RIRB_N = n;
    w16(RIRBWP, 0x8000);
    RIRB_RP = 0;
    w16(RINTCNT, 1);
    w8(RIRBSTS, 0x05);
    w8(CORBCTL, 2);
    // DMA on and the response flag on: QEMU only moves on to the next command once that
    // flag was raised and cleared; with INTCTL 0 no interrupt ever reaches the CPU
    w8(RIRBCTL, 3);
    wait_us(10_000, || r8(CORBCTL) & 2 != 0);
    for cad in 0..15u32 {
        if CODECS & (1 << cad) != 0 {
            CAD = cad;
            if walk_codec() {
                break;
            }
        }
    }
    if PATH_LEN == 0 {
        if WHY.is_empty() {
            WHY = "no codec with an analog output (speaker, headphones or line out)";
        }
        return false;
    }
    if !setup_path() || !setup_stream() {
        return false;
    }
    READY = true;
    true
}

/// Read the codec's widgets and choose the best analog output path. True when found.
unsafe fn walk_codec() -> bool {
    let Some(vid) = cmd(0, GET_PARAM, P_VENDOR) else {
        WHY = "the codec did not answer";
        return false;
    };
    CODEC_ID = vid;
    let fg = param(0, P_NODES);
    NW = 0;
    PATH_LEN = 0;
    AFG = 0;
    for nid in ((fg >> 16) & 0xFF)..((fg >> 16) & 0xFF) + (fg & 0xFF) {
        if param(nid as u8, P_FGTYPE) & 0xFF == 1 {
            AFG = nid as u8;
            break;
        }
    }
    if AFG == 0 {
        return false;
    }
    cmd(AFG, SET_POWER, 0); // D0
    delay_us(10_000);
    let sub = param(AFG, P_NODES);
    let start = ((sub >> 16) & 0xFF) as u8;
    for k in 0..(sub & 0xFF) as u8 {
        if NW == MAXW {
            break;
        }
        let nid = start.wrapping_add(k);
        let caps = param(nid, P_WCAPS);
        let mut w = Widget { nid, caps, ..W0 };
        if (caps >> 20) & 0xF == W_PIN {
            w.pincaps = param(nid, P_PINCAPS);
            w.cfg = cmd(nid, GET_CFG, 0).unwrap_or(0);
        }
        if caps & (1 << 8) != 0 {
            let len = param(nid, P_CONNLEN);
            let long = len & 0x80 != 0;
            let count = (len & 0x7F) as usize;
            let per = if long { 2 } else { 4 };
            let mut prev = 0u16;
            let mut i = 0;
            while i < count && (w.nconn as usize) < 8 {
                let resp = cmd(nid, GET_CONN, i as u32).unwrap_or(0);
                for j in 0..per {
                    if i + j >= count || w.nconn as usize >= 8 {
                        break;
                    }
                    let (e, range) = if long {
                        let e = (resp >> (16 * j)) as u16;
                        (e & 0x7FFF, e & 0x8000 != 0)
                    } else {
                        let e = (resp >> (8 * j)) as u16 & 0xFF;
                        (e & 0x7F, e & 0x80 != 0)
                    };
                    if range && prev != 0 && e > prev {
                        let mut x = prev + 1;
                        while x <= e && (w.nconn as usize) < 8 {
                            w.conn[w.nconn as usize] = x as u8;
                            w.nconn += 1;
                            x += 1;
                        }
                    } else {
                        w.conn[w.nconn as usize] = e as u8;
                        w.nconn += 1;
                    }
                    prev = e;
                }
                i += per;
            }
        }
        WS[NW] = w;
        NW += 1;
    }
    // best pin with a path to a DAC
    let mut best: Option<(u8, [(u8, u8); 6], usize)> = None;
    for i in 0..NW {
        let w = WS[i];
        if (w.caps >> 20) & 0xF != W_PIN {
            continue;
        }
        let Some(rank) = pin_rank(&w) else { continue };
        let mut path = [(0u8, 0u8); 6];
        if let Some(len) = to_dac(w.nid, &mut path, 0) {
            if best.map_or(true, |(r, _, _)| rank < r) {
                best = Some((rank, path, len));
            }
        }
    }
    if let Some((_, p, len)) = best {
        PATH = p;
        PATH_LEN = len;
        return true;
    }
    false
}

/// Depth-first from `nid` to an audio output; fills path[depth..] and returns its length.
fn to_dac(nid: u8, path: &mut [(u8, u8); 6], depth: usize) -> Option<usize> {
    if depth >= path.len() {
        return None;
    }
    let w = widget(nid);
    if w.nid == 0 {
        return None;
    }
    let t = (w.caps >> 20) & 0xF;
    if t == W_OUT {
        if w.caps & (1 << 9) != 0 {
            return None; // a digital converter (S/PDIF, HDMI)
        }
        path[depth] = (nid, 0);
        return Some(depth + 1);
    }
    if depth > 0 && t != W_MIXER && t != W_SELECTOR {
        return None;
    }
    for k in 0..w.nconn as usize {
        path[depth] = (nid, k as u8);
        if let Some(n) = to_dac(w.conn[k], path, depth + 1) {
            return Some(n);
        }
    }
    None
}

/// Power, select and unmute every node on the path, and point the DAC at our stream.
unsafe fn setup_path() -> bool {
    for i in 0..PATH_LEN {
        let (nid, k) = PATH[i];
        let w = widget(nid);
        let t = (w.caps >> 20) & 0xF;
        cmd(nid, SET_POWER, 0);
        if w.nconn > 1 && t != W_MIXER {
            cmd(nid, SET_SEL, k as u32);
        }
        // the amplifiers' 0 dB step ("offset"); a node may override the AFG's caps (bit 3)
        let ampcaps = |p: u32| if w.caps & (1 << 3) != 0 { param(nid, p) } else { param(AFG, p) };
        if t == W_MIXER && w.caps & (1 << 1) != 0 {
            let off = ampcaps(P_INAMP) & 0x7F;
            cmd(nid, SET_AMP4, 1 << 14 | 3 << 12 | (k as u32) << 8 | off);
        }
        if w.caps & (1 << 2) != 0 {
            let off = ampcaps(P_OUTAMP) & 0x7F;
            cmd(nid, SET_AMP4, 1 << 15 | 3 << 12 | off);
        }
        if t == W_PIN {
            let hp = if (w.cfg >> 20) & 0xF == 2 { 0x80 } else { 0 };
            cmd(nid, SET_PINCTL, 0x40 | hp);
            if w.pincaps & (1 << 16) != 0 {
                cmd(nid, SET_EAPD, 0x02); // the external amplifier on (laptop speakers)
            }
        }
        if t == W_OUT {
            let pcm = if param(nid, P_PCM) != 0 { param(nid, P_PCM) } else { param(AFG, P_PCM) };
            // 44.1 kHz (bit 5) and 16-bit (bit 17), else 48 kHz (bit 6): the pitch is then 9% high
            FMT = if pcm & (1 << 5) != 0 || pcm == 0 { FMT_44K1_16_STEREO } else { FMT_48K_16_STEREO };
            cmd(nid, SET_FMT4, FMT as u32);
            cmd(nid, SET_STREAM, STREAM_TAG << 4);
        }
    }
    true
}

/// The first output stream: its buffer list over the 16 periods of the ring.
unsafe fn setup_stream() -> bool {
    SD = 0x80 + 0x20 * STREAMS.1 as u64; // output streams follow the input ones
    let bdl = &mut (*addr_of_mut!(BDL)).0;
    for (i, e) in bdl.iter_mut().enumerate() {
        *e = Bde { addr: addr_of!(RINGBUF) as u64 + (i * PERIOD) as u64, len: PERIOD as u32, ioc: 0 };
    }
    stream_reset()
}

unsafe fn stream_reset() -> bool {
    RUNNING = false;
    w32(SD + SD_CTL, STREAM_TAG << 20);
    wait_us(10_000, || r32(SD + SD_CTL) & 2 == 0);
    w32(SD + SD_CTL, 1);
    wait_us(10_000, || r32(SD + SD_CTL) & 1 != 0);
    w32(SD + SD_CTL, 0);
    if !wait_us(10_000, || r32(SD + SD_CTL) & 1 == 0) {
        WHY = "the output stream did not come out of reset";
        return false;
    }
    let bdl = addr_of!(BDL) as u64;
    w32(SD + SD_BDPL, bdl as u32);
    w32(SD + SD_BDPU, (bdl >> 32) as u32);
    w32(SD + SD_CBL, RING as u32);
    w16(SD + SD_LVI, (NPERIODS - 1) as u16);
    w16(SD + SD_FMT, FMT);
    w32(SD + SD_CTL, STREAM_TAG << 20 | 0x1C << 24); // stream tag, old status flags cleared
    true
}

pub fn present() -> bool {
    // SAFETY: scalar read of boot-time state.
    unsafe { READY }
}

/// Bytes the DMA engine has read from the ring since the stream began.
unsafe fn played() -> u64 {
    let lpib = (r32(SD + SD_LPIB) as u64).min(RING as u64);
    if lpib < LAST_LPIB {
        WRAPS += 1;
    }
    LAST_LPIB = lpib;
    WRAPS * RING as u64 + lpib
}

unsafe fn run() {
    w32(SD + SD_CTL, STREAM_TAG << 20 | 2);
    RUNNING = true;
}

/// Put one period (STAGE) into the ring behind the DMA position. False if the DMA stalls.
unsafe fn put_period() -> bool {
    if !RUNNING && WRITTEN + PERIOD as u64 > RING as u64 {
        run();
    }
    if RUNNING {
        let t0 = crate::timer::cycles();
        let mut last = played();
        let mut moved = t0;
        while WRITTEN + PERIOD as u64 > played() + RING as u64 {
            let p = played();
            if p != last {
                last = p;
                moved = crate::timer::cycles();
            } else if crate::timer::cycles().wrapping_sub(moved) > us(2_000_000) {
                crate::kprintln!("[hda] the output stream stopped moving at byte {} (LPIB {:#x})", p, LAST_LPIB);
                stream_reset();
                return false;
            }
            core::hint::spin_loop();
        }
    }
    let at = (WRITTEN % RING as u64) as usize;
    core::ptr::copy_nonoverlapping(addr_of!(STAGE) as *const u8, (addr_of_mut!(RINGBUF) as *mut u8).add(at), PERIOD);
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    WRITTEN += PERIOD as u64;
    true
}

/// Open a playback stream (pairs with stream_write and stream_end).
pub fn stream_begin() -> bool {
    // SAFETY: single core, kernel context; the stream is ours.
    unsafe {
        if !READY || !stream_reset() {
            return false;
        }
        core::ptr::write_bytes(addr_of_mut!(RINGBUF) as *mut u8, 0, RING);
        WRITTEN = 0;
        WRAPS = 0;
        LAST_LPIB = 0;
        FILL = 0;
        PERIODS = 0;
        SAMPLES = 0;
        true
    }
}

/// Mono 44.1 kHz samples, any number, duplicated to both channels.
pub fn stream_write(samples: &[i16]) -> bool {
    // SAFETY: as stream_begin.
    unsafe {
        if !READY {
            return false;
        }
        let frames = PERIOD / 4;
        let st = addr_of_mut!(STAGE) as *mut u8;
        let mut i = 0;
        while i < samples.len() {
            let take = core::cmp::min(frames - FILL, samples.len() - i);
            for k in 0..take {
                let b = samples[i + k].to_le_bytes();
                let o = (FILL + k) * 4;
                *st.add(o) = b[0];
                *st.add(o + 1) = b[1];
                *st.add(o + 2) = b[0];
                *st.add(o + 3) = b[1];
            }
            FILL += take;
            i += take;
            if FILL == frames {
                FILL = 0;
                if !put_period() {
                    return false;
                }
                PERIODS += 1;
            }
        }
        SAMPLES += samples.len() as u64;
        true
    }
}

/// Play what is staged, wait until the DMA has played all of it, then stop the stream.
pub fn stream_end() -> bool {
    // SAFETY: as stream_begin.
    unsafe {
        if !READY {
            return false;
        }
        let st = addr_of_mut!(STAGE) as *mut u8;
        if FILL > 0 {
            core::ptr::write_bytes(st.add(FILL * 4), 0, PERIOD - FILL * 4);
            FILL = 0;
            if !put_period() {
                return false;
            }
            PERIODS += 1;
        }
        let end = WRITTEN;
        if !RUNNING {
            run();
        }
        // keep silence going in behind the sound until the DMA is past its end
        core::ptr::write_bytes(st, 0, PERIOD);
        let t0 = crate::timer::cycles();
        while played() < end {
            if WRITTEN + PERIOD as u64 <= played() + RING as u64 && !put_period() {
                return false;
            }
            if crate::timer::cycles().wrapping_sub(t0) > us(30_000_000) {
                crate::kprintln!("[hda] the output stream did not finish");
                stream_reset();
                return false;
            }
            core::hint::spin_loop();
        }
        stream_reset();
        crate::kprintln!(
            "[hda] played {} samples ({} periods, ~{} ms) at {} Hz through {}",
            SAMPLES,
            PERIODS,
            SAMPLES * 1000 / RATE_HZ as u64,
            if FMT == FMT_44K1_16_STEREO { 44100 } else { 48000 },
            PathText
        );
        true
    }
}

/// A 441 Hz square tone, 16 periods (~371 ms), as virtio-sound's `audio tone`.
pub fn tone() -> bool {
    if !stream_begin() {
        return false;
    }
    let half = (RATE_HZ / 441 / 2) as usize;
    let mut buf = [0i16; 1024];
    let mut phase = 0usize;
    for _ in 0..16 {
        for s in buf.iter_mut() {
            *s = if (phase / half) & 1 == 0 { 8000 } else { -8000 };
            phase += 1;
        }
        if !stream_write(&buf) {
            return false;
        }
    }
    stream_end()
}

/// `audio` on a PC whose sound is HD Audio.
pub fn audio(rest: &str) {
    if rest == "tone" {
        crate::kprintln!("[audio] streaming a test tone through Intel HD Audio...");
        if tone() {
            crate::kprintln!("[audio] done — captured by the wav audiodev if attached");
        }
    } else {
        // SAFETY: reads of boot-time state.
        unsafe {
            crate::kprintln!(
                "[audio] HD Audio online: {} codec {:04x}:{:04x}, out through {}; 'audio tone' to play",
                codec_vendor(CODEC_ID), CODEC_ID >> 16, CODEC_ID & 0xFFFF, PathText
            );
        }
    }
}

/// `lshda`: the controller, its codecs and the output path (reads only).
pub fn list() {
    // SAFETY: reads of boot-time state.
    unsafe {
        if BAR == 0 {
            crate::kprintln!("lshda: no HD Audio controller on this PC");
            return;
        }
        let (b, d, f) = PCI_AT;
        crate::kprintln!(
            "lshda: {} at {:02x}:{:02x}.{} ({:04x}:{:04x}), HD Audio {}.{}, {} output / {} input streams",
            controller_name(PCI_ID.0, PCI_ID.1), b, d, f, PCI_ID.0, PCI_ID.1, VER.0, VER.1, STREAMS.0, STREAMS.1
        );
        let mut n = 0;
        for c in 0..15 {
            if CODECS & (1 << c) != 0 {
                n += 1;
            }
        }
        crate::kprintln!("lshda: {} codec(s) answered (mask {:#06x})", n, CODECS);
        if CODEC_ID != 0 {
            crate::kprintln!(
                "lshda: codec {}: {} {:04x}:{:04x}, audio function group at node {}, {} widgets",
                CAD, codec_vendor(CODEC_ID), CODEC_ID >> 16, CODEC_ID & 0xFFFF, AFG, NW
            );
        }
        for i in 0..NW {
            let w = WS[i];
            match (w.caps >> 20) & 0xF {
                W_PIN if w.pincaps & (1 << 4) != 0 => crate::kprintln!(
                    "  node {} pin: {}{} (config {:#010x}){}",
                    w.nid,
                    pin_kind(w.cfg),
                    if w.cfg >> 30 == 1 { ", not connected on this board" } else { "" },
                    w.cfg,
                    match pin_rank(&w) { Some(_) => "", None => " -- not used" }
                ),
                W_OUT => crate::kprintln!("  node {} audio output (DAC){}", w.nid, if w.caps & (1 << 9) != 0 { ", digital" } else { "" }),
                _ => {}
            }
        }
        if READY {
            crate::kprintln!(
                "lshda: sound goes out through {}, stream {}, {} 16-bit stereo",
                PathText, STREAM_TAG, if FMT == FMT_44K1_16_STEREO { "44.1 kHz" } else { "48 kHz" }
            );
        } else {
            crate::kprintln!("lshda: not usable: {}", WHY);
        }
    }
}
