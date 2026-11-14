//! USB Video Class webcams, slice 1 (13 Nov 2026, UVC_PLAN.md and the architect's verdict of 13 Nov): a YUY2
//! picture in a box on the screen, started only by `camera on` typed at the local keyboard or the serial console.
//!
//! Written from the USB-IF "Universal Serial Bus Device Class Definition for Video Devices" 1.5 (and 1.1, which
//! most laptop cameras report): the descriptors of sections 3.7 to 3.9, the payload header of the "Payload Format
//! for Uncompressed Formats" 2.4, and the probe and commit controls of 4.3.1.1. xhci.rs does the USB part (the
//! configuration read, the controls, the isochronous IN ring); this file has the parsers, which take only byte
//! slices, the frame buffers and what the screen shows.
//!
//! The camera is not trusted (A4, a hostile USB device): every descriptor length is checked against the bytes
//! that came back, at most 64 frame sizes and 16 alternate settings are kept, sizes use checked maths and only a
//! size of 640x480 or below is ever matched to a buffer, a payload header must be 2 to 12 bytes and inside its
//! packet, and a frame is handed on only when it is whole and of the exact size.
//!
//! The person's privacy (the architect's conditions 1 to 3): the camera starts only on `camera on` from the
//! local console (the field report's probes, which go to the network, are refused); while it runs the kernel
//! draws a red CAMERA ON mark after every picture, in a strip the picture is clipped away from, until the stream
//! has stopped; the frames go only to that box, and the frame buffers are zeroed on stop, unplug and error.

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

/// The largest picture slice 1 takes (the architect's Q2: 640x480 YUY2). The box on the screen is at most
/// 320x240 (every second pixel of a bigger frame), so .bss stays near the 1.4 MiB agreed: two frames of 600 KiB,
/// the box 300 KiB, the packet buffers 156 KiB.
pub const BOX_W: usize = 320;
pub const BOX_H: usize = 240;
pub const MAX_W: usize = 640;
pub const MAX_H: usize = 480;
pub const FRAME_MAX: usize = MAX_W * MAX_H * 2;
pub const MAX_FRAMES: usize = 64;
pub const MAX_ALTS: usize = 16;

/// The YUY2 format GUID (32595559-0000-0010-8000-00AA00389B71) as it lies in a format descriptor.
const YUY2: [u8; 16] = [0x59, 0x55, 0x59, 0x32, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71];

/// One frame size the camera offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub format: u8,
    pub frame: u8,
    pub w: u16,
    pub h: u16,
    /// the slowest frame interval it offers, in 100 ns units
    pub interval: u32,
    /// w x h x 2, or 0 when that does not fit 32 bits
    pub bytes: u32,
}
const SIZE0: Size = Size { format: 0, frame: 0, w: 0, h: 0, interval: 0, bytes: 0 };

/// One alternate setting of the streaming interface with an isochronous IN endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alt {
    pub alt: u8,
    pub ep: u8,
    /// bytes per transaction (wMaxPacketSize bits 10:0)
    pub mps: u16,
    /// transactions per microframe (1 to 3)
    pub mult: u8,
    pub interval: u8,
}
const ALT0: Alt = Alt { alt: 0, ep: 0, mps: 0, mult: 0, interval: 0 };

impl Alt {
    /// Bytes this setting moves in one service interval.
    pub fn bytes(&self) -> u32 {
        self.mps as u32 * self.mult as u32
    }
}

/// What the configuration descriptor says about the first video function.
#[derive(Clone, Copy, Debug)]
pub struct Cam {
    pub vc_if: u8,
    pub vs_if: u8,
    /// bcdUVC from the VideoControl header (0x0100, 0x0110, 0x0150)
    pub uvc: u16,
    pub sizes: [Size; MAX_FRAMES],
    pub nsizes: usize,
    /// frame descriptors seen past the 64 kept
    pub more: usize,
    pub formats: u8,
    pub mjpeg: bool,
    pub other: bool,
    pub alts: [Alt; MAX_ALTS],
    pub nalts: usize,
}

/// Why a camera's descriptors were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// a descriptor of length 0 or 1
    ZeroLength,
    /// a descriptor reaching past the bytes that came back
    PastEnd,
    /// no VideoControl and VideoStreaming interface pair
    NoVideo,
    /// an endpoint whose packet size says 4 transactions per microframe (value 3 is reserved)
    ReservedMult,
}

impl core::fmt::Display for Bad {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(match self {
            Bad::ZeroLength => "a descriptor has length 0 or 1",
            Bad::PastEnd => "a descriptor reaches past the end of the configuration",
            Bad::NoVideo => "no video control and streaming interfaces",
            Bad::ReservedMult => "an endpoint's packet size uses the reserved transaction count",
        })
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Walk a whole configuration descriptor: the first VideoControl interface (class 0x0E, subclass 1) with its
/// header, the VideoStreaming interface (subclass 2) after it, its uncompressed YUY2 formats and their frame
/// sizes, and its alternate settings with an isochronous IN endpoint. MJPEG and other formats are noted, not kept.
pub fn descriptors(cfg: &[u8]) -> Result<Cam, Bad> {
    let mut c = Cam {
        vc_if: 0xFF,
        vs_if: 0xFF,
        uvc: 0,
        sizes: [SIZE0; MAX_FRAMES],
        nsizes: 0,
        more: 0,
        formats: 0,
        mjpeg: false,
        other: false,
        alts: [ALT0; MAX_ALTS],
        nalts: 0,
    };
    // where we are: 1 inside the VideoControl interface, 2 inside our VideoStreaming one (with its alt), 0 other
    let (mut at, mut inside, mut alt, mut yuy2_fmt) = (0usize, 0u8, 0u8, 0u8);
    while at < cfg.len() {
        let len = cfg[at] as usize;
        if len < 2 {
            return Err(Bad::ZeroLength);
        }
        if at + len > cfg.len() {
            return Err(Bad::PastEnd);
        }
        let d = &cfg[at..at + len];
        match d[1] {
            4 if len >= 9 => {
                let (num, a, class, sub) = (d[2], d[3], d[5], d[6]);
                inside = 0;
                if class == 0x0E && sub == 1 && c.vc_if == 0xFF {
                    c.vc_if = num;
                    inside = 1;
                } else if class == 0x0E && sub == 2 && c.vc_if != 0xFF && (c.vs_if == 0xFF || c.vs_if == num) {
                    c.vs_if = num;
                    inside = 2;
                    alt = a;
                }
            }
            // class-specific interface descriptors
            0x24 if inside == 1 && len >= 5 && d[2] == 1 => c.uvc = le16(d, 3),
            0x24 if inside == 2 && alt == 0 && len >= 3 => match d[2] {
                0x04 if len >= 27 => {
                    c.formats = c.formats.saturating_add(1);
                    yuy2_fmt = if d[5..21] == YUY2 { d[3] } else { 0 };
                    if yuy2_fmt == 0 {
                        c.other = true;
                    }
                }
                0x06 => {
                    c.formats = c.formats.saturating_add(1);
                    c.mjpeg = true;
                    yuy2_fmt = 0;
                }
                0x10 => {
                    c.formats = c.formats.saturating_add(1);
                    c.other = true; // frame-based (H.264 and others)
                    yuy2_fmt = 0;
                }
                0x05 if yuy2_fmt != 0 && len >= 26 => {
                    if c.nsizes == MAX_FRAMES {
                        c.more += 1;
                    } else {
                        let (w, h) = (le16(d, 5), le16(d, 7));
                        let kind = d[25];
                        // the slowest interval: the largest listed, or the top of a continuous range
                        let interval = if kind == 0 {
                            if len >= 34 { le32(d, 30) } else { 0 }
                        } else {
                            let n = (kind as usize).min((len - 26) / 4);
                            (0..n).map(|k| le32(d, 26 + 4 * k)).max().unwrap_or(0)
                        };
                        let bytes = (w as u32).checked_mul(h as u32).and_then(|p| p.checked_mul(2)).unwrap_or(0);
                        c.sizes[c.nsizes] = Size { format: yuy2_fmt, frame: d[3], w, h, interval, bytes };
                        c.nsizes += 1;
                    }
                }
                _ => {}
            },
            5 if inside == 2 && alt != 0 && len >= 7 => {
                let (addr, attr, raw) = (d[2], d[3], le16(d, 4));
                if addr & 0x80 != 0 && attr & 3 == 1 {
                    let extra = (raw >> 11) & 3;
                    if extra == 3 {
                        return Err(Bad::ReservedMult);
                    }
                    if c.nalts < MAX_ALTS && !c.alts[..c.nalts].iter().any(|x| x.alt == alt) {
                        c.alts[c.nalts] = Alt { alt, ep: addr, mps: raw & 0x7FF, mult: extra as u8 + 1, interval: d[6] };
                        c.nalts += 1;
                    }
                }
            }
            _ => {}
        }
        at += len;
    }
    if c.vc_if == 0xFF || c.vs_if == 0xFF {
        return Err(Bad::NoVideo);
    }
    Ok(c)
}

/// The size slice 1 shows: the largest YUY2 size up to 640x480 (by pixels), at the slowest interval it offers.
pub fn pick(c: &Cam) -> Option<Size> {
    c.sizes[..c.nsizes]
        .iter()
        .filter(|s| s.w > 0 && s.h > 0 && (s.w as usize) <= MAX_W && (s.h as usize) <= MAX_H && s.bytes as usize <= FRAME_MAX)
        .filter(|s| s.w % 2 == 0 && s.bytes == s.w as u32 * s.h as u32 * 2)
        .max_by_key(|s| s.w as u32 * s.h as u32)
        .copied()
}

/// The smallest alternate setting that carries `need` bytes per service interval.
pub fn alt_for(c: &Cam, need: u32) -> Option<Alt> {
    c.alts[..c.nalts].iter().filter(|a| a.bytes() >= need && a.mps > 0).min_by_key(|a| a.bytes()).copied()
}

// --- frames from isochronous packets ---

/// The frame being put together from packets, and the counters `camera stats` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asm {
    /// the frame's size in bytes (w x h x 2)
    pub size: usize,
    pub fill: usize,
    /// the frame toggle of the frame being filled; None before the first frame boundary
    pub fid: Option<bool>,
    /// a frame boundary has been seen (data before it is the end of a frame we joined late)
    pub synced: bool,
    /// the frame in progress is already lost (error bit, too long, a bad header)
    pub bad: Option<Lost>,
    pub frames: u64,
    pub drop_err: u64,
    pub drop_short: u64,
    pub drop_long: u64,
    pub drop_header: u64,
    pub bad_headers: u64,
    pub empty: u64,
    pub packets: u64,
    /// the packet that ended a frame by its toggle also starts the next: run it again after the swap
    pub again: bool,
}

/// Why a frame was not handed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    Error,
    Long,
    Header,
}

/// What one packet did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    /// a whole frame of the exact size is in the buffer: hand it on, give `payload` a fresh buffer
    Whole,
}

impl Asm {
    pub const fn new(size: usize) -> Asm {
        Asm {
            size,
            fill: 0,
            fid: None,
            synced: false,
            bad: None,
            frames: 0,
            drop_err: 0,
            drop_short: 0,
            drop_long: 0,
            drop_header: 0,
            bad_headers: 0,
            empty: 0,
            packets: 0,
            again: false,
        }
    }

    /// Frames lost, all reasons.
    pub fn dropped(&self) -> u64 {
        self.drop_err + self.drop_short + self.drop_long + self.drop_header
    }

    /// The frame in progress ends here (a boundary): count it as whole (true) or as dropped.
    fn end(&mut self) -> bool {
        let whole = self.synced && self.bad.is_none() && self.fill == self.size;
        if self.synced && !whole {
            match self.bad {
                Some(Lost::Error) => self.drop_err += 1,
                Some(Lost::Long) => self.drop_long += 1,
                Some(Lost::Header) => self.drop_header += 1,
                None if self.fill > 0 => self.drop_short += 1,
                None => {}
            }
        }
        if whole {
            self.frames += 1;
        }
        self.fill = 0;
        self.bad = None;
        self.synced = true;
        whole
    }

    /// The packet in progress was lost by the controller (missed service and the like): the frame is short.
    pub fn lost_packet(&mut self) {
        self.empty += 1;
    }
}

/// One isochronous packet (payload header, then data) into `buf`, the frame being filled (at least `a.size`).
pub fn payload(a: &mut Asm, pkt: &[u8], buf: &mut [u8]) -> Step {
    let rerun = core::mem::replace(&mut a.again, false);
    if !rerun {
        a.packets += 1;
    }
    if pkt.is_empty() {
        a.empty += 1;
        return Step::Nothing;
    }
    let hl = pkt[0] as usize;
    if !(2..=12).contains(&hl) || hl > pkt.len() {
        a.bad_headers += 1;
        if a.synced && a.bad.is_none() {
            a.bad = Some(Lost::Header);
        }
        return Step::Nothing;
    }
    let info = pkt[1];
    let (fid, eof, err) = (info & 1 != 0, info & 2 != 0, info & 0x40 != 0);
    let data = &pkt[hl..];
    if a.fid.is_some_and(|f| f != fid) {
        // a new frame starts with this packet: end the one before it
        a.fid = Some(fid);
        if a.end() {
            a.again = true; // the caller swaps buffers and runs this packet again
            return Step::Whole;
        }
    }
    a.fid = Some(fid);
    if err && a.bad.is_none() {
        a.bad = Some(Lost::Error);
    }
    if a.synced && a.bad.is_none() && !data.is_empty() {
        if a.fill + data.len() > a.size || buf.len() < a.size {
            a.bad = Some(Lost::Long);
        } else {
            buf[a.fill..a.fill + data.len()].copy_from_slice(data);
            a.fill += data.len();
        }
    }
    if eof {
        if a.end() {
            return Step::Whole;
        }
    }
    Step::Nothing
}

/// One YUY2 pixel pair's colour (BT.601, video range, integer maths) as 0x00RRGGBB.
pub fn rgb(y: u8, u: u8, v: u8) -> u32 {
    let (c, d, e) = (y as i32 - 16, u as i32 - 128, v as i32 - 128);
    let k = |x: i32| x.clamp(0, 255) as u32;
    let r = k((298 * c + 409 * e + 128) >> 8);
    let g = k((298 * c - 100 * d - 208 * e + 128) >> 8);
    let b = k((298 * c + 516 * d + 128) >> 8);
    r << 16 | g << 8 | b
}

/// FNV-1a, 64 bits (the test camera's frame check; the grader computes the same in Python).
pub fn fnv64(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// --- the frame buffers, the picture box and the mark ---

#[repr(C, align(4096))]
struct Yuv([u8; FRAME_MAX]);
/// Two frames: one filling (interrupt), one whole (shown by the idle loop).
static mut FRAMES: [Yuv; 2] = [const { Yuv([0; FRAME_MAX]) }; 2];
/// The picture as the screen shows it (at most 320x240 pixels).
static mut BOX: [u32; BOX_W * BOX_H] = [0; BOX_W * BOX_H];
/// The box's size: the frame's, or half of it for a frame wider than 320 or taller than 240.
static mut BOXSZ: (usize, usize) = (0, 0);
static mut ASM: Asm = Asm::new(0);
static mut FILL: usize = 0;
static mut READY_IDX: usize = 0;
static NEW: AtomicBool = AtomicBool::new(false);
static SHOWING: AtomicBool = AtomicBool::new(false);
/// The tick after which a stop that could not finish is tried again.
static NEXT_TRY: AtomicU64 = AtomicU64::new(0);
/// The mark is on the screen: from `camera on` until the stream has stopped (after Stop Endpoint).
static MARK: AtomicBool = AtomicBool::new(false);
/// The picture box has a frame in it.
static PICTURE: AtomicBool = AtomicBool::new(false);
static mut SIZE: (usize, usize) = (0, 0);
static SHOWN: AtomicU64 = AtomicU64::new(0);
static SKIPPED: AtomicU64 = AtomicU64::new(0);
/// The test camera's last frame shown: (its own frame number, fnv64 of its bytes).
static mut LAST_SUM: Option<(u32, u64)> = None;
/// The last stream error the interrupt saw (stops the stream at the next idle moment): 0 none.
static FAILED: AtomicU8 = AtomicU8::new(0);
static mut FAIL_WHY: &str = "";

/// The mark's colour (graded from a screenshot by pc-usbcam) and its strip's height.
pub const MARK_RGB: u32 = 0xD0_1C1C;
const STRIP: usize = 20;
const MARGIN: usize = 8;

/// Start assembling frames of w x h (stream start, interrupts not yet coming).
pub fn begin(w: usize, h: usize) {
    // SAFETY: the stream is not running yet; the interrupt does not touch these until xhci queues packets.
    unsafe {
        ASM = Asm::new(w * h * 2);
        (FILL, READY_IDX, SIZE, LAST_SUM) = (0, 1, (w, h), None);
        let step = if w > BOX_W || h > BOX_H { 2 } else { 1 };
        BOXSZ = ((w / step).min(BOX_W), (h / step).min(BOX_H));
        FAIL_WHY = "";
    }
    NEW.store(false, Ordering::Relaxed);
    PICTURE.store(false, Ordering::Relaxed);
    FAILED.store(0, Ordering::Relaxed);
    SHOWN.store(0, Ordering::Relaxed);
    SKIPPED.store(0, Ordering::Relaxed);
    MARK.store(true, Ordering::Relaxed);
    crate::screen::mark();
}

/// One packet's bytes from the isochronous ring (timer interrupt, xhci::handle).
pub fn packet(pkt: &[u8]) {
    // SAFETY: only the timer interrupt runs this (one core); the idle loop reads FRAMES[READY_IDX] only while
    // SHOWING is set, and then this never swaps, so it never writes that buffer.
    unsafe {
        loop {
            let a = &mut *addr_of_mut!(ASM);
            let buf = &mut (*addr_of_mut!(FRAMES))[FILL].0;
            if payload(a, pkt, buf) == Step::Whole {
                if SHOWING.load(Ordering::Relaxed) {
                    SKIPPED.fetch_add(1, Ordering::Relaxed); // the next frame fills the same buffer
                } else {
                    READY_IDX = FILL;
                    FILL ^= 1;
                    NEW.store(true, Ordering::Release);
                }
                if a.again {
                    continue;
                }
            }
            return;
        }
    }
}

/// The controller lost a packet (missed service and the like).
pub fn lost_packet() {
    // SAFETY: as `packet`.
    unsafe { (*addr_of_mut!(ASM)).lost_packet() }
}

/// The stream hit an error that ends it (a halt or a transaction error): the idle loop stops it.
pub fn fail(why: &'static str) {
    if FAILED.swap(1, Ordering::Relaxed) == 0 {
        // SAFETY: written once per stream, read by the idle loop after FAILED is seen.
        unsafe { FAIL_WHY = why };
    }
}

/// The idle loop (cpu::wfi): stop a failed stream; show a new frame.
pub fn idle() {
    if crate::xhci::cam_stopping() {
        // a stop that could not finish: try again about once a second, the mark on until it does
        let t = crate::sched::ticks();
        if !crate::xhci::busy() && t >= NEXT_TRY.load(Ordering::Relaxed) {
            NEXT_TRY.store(t + 100, Ordering::Relaxed);
            if crate::xhci::cam_stop() {
                crate::kprintln!("[camera] stopped now (the mark is off, the frames are cleared)");
            }
        }
        return;
    }
    if FAILED.load(Ordering::Relaxed) != 0 && crate::xhci::cam_running() && !crate::xhci::busy() {
        // SAFETY: set once by `fail` before FAILED.
        let why = unsafe { FAIL_WHY };
        NEXT_TRY.store(crate::sched::ticks() + 100, Ordering::Relaxed);
        if crate::xhci::cam_stop() {
            crate::kprintln!("[camera] the stream stopped: {} (the mark is off, the frames are cleared)", why);
        } else {
            crate::kprintln!("[camera] the stream failed: {}", why);
        }
        return;
    }
    // Acquire: the frame's bytes the interrupt wrote before NEW are seen here
    if !NEW.swap(false, Ordering::Acquire) || !MARK.load(Ordering::Relaxed) {
        return;
    }
    SHOWING.store(true, Ordering::Relaxed);
    // one core: only the compiler could move the frame reads above SHOWING
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    // SAFETY: SHOWING is set, so the interrupt does not swap: FRAMES[READY_IDX] stays whole while it is read.
    unsafe {
        let (w, h) = SIZE;
        let whole: &[u8; FRAME_MAX] = &(*addr_of!(FRAMES))[READY_IDX].0;
        let f = &whole[..w * h * 2];
        if crate::xhci::cam_is_test() {
            // the AfriSovera test camera only: its own frame number (the first 4 bytes) and a checksum, so the
            // grader can rebuild that frame in Python; a real camera's frames are never summed or printed
            LAST_SUM = Some((u32::from_le_bytes([f[0], f[1], f[2], f[3]]), fnv64(f)));
        }
        let out = &mut *addr_of_mut!(BOX);
        let (bw, bh) = BOXSZ;
        let step = if w > BOX_W || h > BOX_H { 2 } else { 1 };
        for by in 0..bh {
            for bx in 0..bw {
                let (x, y) = (bx * step, by * step);
                // a YUY2 pair is Y0 U Y1 V; x is the left (Y0) or right (Y1) pixel of its pair
                let at = (y * w + (x & !1)) * 2;
                let p = &f[at..at + 4];
                out[by * bw + bx] = rgb(if x & 1 == 0 { p[0] } else { p[2] }, p[1], p[3]);
            }
        }
    }
    core::sync::atomic::compiler_fence(Ordering::SeqCst);
    SHOWING.store(false, Ordering::Relaxed);
    PICTURE.store(true, Ordering::Relaxed);
    SHOWN.fetch_add(1, Ordering::Relaxed);
    crate::screen::cam_draw();
}

/// The stream has stopped (Stop Endpoint done, or the camera unplugged): frames zeroed, mark and box gone.
pub fn stopped() {
    MARK.store(false, Ordering::Relaxed);
    PICTURE.store(false, Ordering::Relaxed);
    NEW.store(false, Ordering::Relaxed);
    // SAFETY: the stream no longer runs, so the interrupt writes no frame.
    unsafe {
        core::ptr::write_bytes(addr_of_mut!(FRAMES) as *mut u8, 0, core::mem::size_of::<[Yuv; 2]>());
        core::ptr::write_bytes(addr_of_mut!(BOX) as *mut u8, 0, core::mem::size_of::<[u32; BOX_W * BOX_H]>());
        LAST_SUM = None;
    }
    crate::screen::repaint_all();
}

/// True while the mark is on the screen.
pub fn mark_on() -> bool {
    MARK.load(Ordering::Relaxed)
}

/// Draw the picture box and the mark on a GOP screen (base, width, height, stride, BGR order): the strip with
/// the mark at the top right, the picture under it, clipped away from the strip. Called after every picture
/// (idle loop) and at the end of every screen redraw, so nothing drawn before can cover the mark.
/// SAFETY: `base` is the attached GOP framebuffer of w x h pixels, `stride` pixels to a row.
pub unsafe fn draw(base: u64, w: usize, h: usize, stride: usize, bgr: bool) {
    if !MARK.load(Ordering::Relaxed) {
        return;
    }
    let (bw, bh) = BOXSZ;
    let pw = bw.max(160).min(w.saturating_sub(2 * MARGIN));
    let ph = bh.min(h.saturating_sub(STRIP + 2 * MARGIN));
    let x0 = w.saturating_sub(pw + MARGIN);
    let px = |v: u32| if bgr { v } else { (v & 0xFF) << 16 | (v & 0xFF00) | (v >> 16) & 0xFF };
    let dst = base as *mut u32;
    // the strip: red, "CAMERA ON" in white, 2x the 8x8 font
    for y in MARGIN..MARGIN + STRIP {
        for x in x0..x0 + pw {
            core::ptr::write_volatile(dst.add(y * stride + x), px(MARK_RGB));
        }
    }
    for (k, &ch) in b"CAMERA ON".iter().enumerate() {
        let g = crate::font::glyph(ch);
        for (r, bits) in g.iter().enumerate() {
            for c in 0..8 {
                if bits & (0x80 >> c) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let (x, y) = (x0 + 6 + k * 16 + c * 2 + dx, MARGIN + 2 + r * 2 + dy);
                            if x < x0 + pw {
                                core::ptr::write_volatile(dst.add(y * stride + x), px(0xFF_FFFF));
                            }
                        }
                    }
                }
            }
        }
    }
    // the picture, below the strip only
    let y0 = MARGIN + STRIP;
    if !PICTURE.load(Ordering::Relaxed) {
        for y in y0..y0 + ph {
            for x in x0..x0 + pw {
                core::ptr::write_volatile(dst.add(y * stride + x), 0);
            }
        }
        return;
    }
    let src = &*addr_of!(BOX);
    for y in 0..ph {
        for x in 0..pw {
            let v = if x < bw { src[y * bw + x] } else { 0 };
            core::ptr::write_volatile(dst.add((y0 + y) * stride + x0 + x), px(v));
        }
    }
}

// --- the shell command ---

/// Where a shell command came from: the local keyboard or serial console, or the field report's probes (whose
/// output goes to the network). Nothing else runs shell commands on the PC (no remote shell, no scripts at boot,
/// no app call into the shell); a new way must say what it is here first, and the camera refuses anything not local.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Local,
    Network,
}

fn origin() -> Origin {
    if crate::report::capturing() { Origin::Network } else { Origin::Local }
}

/// `camera on` from `from`: refused unless typed locally (the architect's condition 1).
fn start(from: Origin) -> Result<(), &'static str> {
    if from != Origin::Local {
        return Err("refused: the camera starts only from the local keyboard or serial console, never from the network");
    }
    if !crate::screen::on() {
        return Err("refused: there is no screen to show the CAMERA ON mark");
    }
    crate::xhci::cam_start()
}

/// `camera`, `camera on|off|stats|test`.
pub fn command(arg: &str) {
    match arg.trim() {
        "" => crate::xhci::cam_list(),
        "on" => match start(origin()) {
            Ok(()) => {}
            Err(e) => crate::kprintln!("camera: {}", e),
        },
        "off" => {
            if crate::xhci::cam_running() {
                // when it cannot stop yet, cam_stop has said so and the idle loop tries again; the mark stays
                if crate::xhci::cam_stop() {
                    crate::kprintln!("camera: off (the stream has stopped, the mark is gone, the frames are cleared)");
                }
            } else {
                crate::kprintln!("camera: not on");
            }
        }
        "stats" => stats(),
        "test" => selftest(),
        _ => crate::kprintln!("camera: camera | camera on | camera off | camera stats | camera test"),
    }
}

fn stats() {
    // SAFETY: plain copies of counters the interrupt updates (a torn read only skews a count).
    let (a, sum) = unsafe { (*addr_of!(ASM), LAST_SUM) };
    crate::kprintln!(
        "camera stats: {} whole frames, {} dropped (error bit {}, short {}, too long {}, bad header {}), {} shown, {} not shown (busy), {} packets, {} empty, {} bad headers; mark {}",
        a.frames, a.dropped(), a.drop_err, a.drop_short, a.drop_long, a.drop_header,
        SHOWN.load(Ordering::Relaxed), SKIPPED.load(Ordering::Relaxed), a.packets, a.empty, a.bad_headers,
        if mark_on() { "on" } else { "off" }
    );
    crate::xhci::cam_ring_stats();
    if let Some((n, h)) = sum {
        crate::kprintln!("camera stats: test pattern frame {} fnv64 {:016x}", n, h);
    }
}

/// Where an isochronous event's ring slot `k` stands against `next`, the slot whose completion comes next, in a
/// ring of `n` packets that are all queued (ARCHITECT_REVIEWS 13 Nov night, fix 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `k` is `next`: in order.
    Next,
    /// `k` is this many slots ahead: the controller passed the ones in between without an event (lost packets).
    Ahead(usize),
    /// `k` was already given back: a real fault.
    Behind,
}

/// Every slot is queued at once, so a slot far ahead and one just given back sit at the same place in the ring.
/// A controller skips a few service intervals, not most of the ring (a ring that runs dry says so with an
/// underrun event), so up to half the ring ahead is taken as skipped packets and the rest as behind.
pub fn slot(next: usize, k: usize, n: usize) -> Slot {
    if n == 0 || next >= n || k >= n {
        return Slot::Behind;
    }
    let gap = (k + n - next) % n;
    if gap == 0 {
        Slot::Next
    } else if gap <= n / 2 {
        Slot::Ahead(gap)
    } else {
        Slot::Behind
    }
}

// --- the selftest (`camera test`) ---

static mut ST: [u8; 4096] = [0; 4096];
static mut STF: [u8; 64] = [0; 64];

/// A hand-made configuration: VC interface 0 (header bcdUVC), VS interface 1 alt 0 with the formats in `fmts`,
/// then alternate settings with isochronous endpoints of the given wMaxPacketSize values. Returns its length.
fn cfg(out: &mut [u8], uvc: u16, fmts: &[(&[u8; 16], &[(u16, u16)])], mjpeg: bool, eps: &[u16]) -> usize {
    let mut n = 0;
    let mut put = |b: &[u8]| {
        out[n..n + b.len()].copy_from_slice(b);
        n += b.len();
    };
    put(&[9, 2, 0, 0, 2, 1, 0, 0x80, 250]); // configuration (total length not used by the walk)
    put(&[8, 0x0B, 0, 2, 0x0E, 3, 0, 0]); // interface association
    put(&[9, 4, 0, 0, 1, 0x0E, 1, 0, 0]);
    let u = uvc.to_le_bytes();
    put(&[13, 0x24, 1, u[0], u[1], 0, 0, 0, 0, 0, 0, 1, 1]);
    put(&[7, 5, 0x83, 3, 16, 0, 6]); // the VC interrupt endpoint
    put(&[9, 4, 1, 0, 0, 0x0E, 2, 0, 0]);
    put(&[14, 0x24, 1, fmts.len() as u8 + mjpeg as u8, 0, 0, 0x81, 0, 0, 0, 0, 0, 1, 0]);
    for (k, (guid, sizes)) in fmts.iter().enumerate() {
        let mut f = [0u8; 27];
        f[..5].copy_from_slice(&[27, 0x24, 4, k as u8 + 1, sizes.len() as u8]);
        f[5..21].copy_from_slice(&guid[..]);
        f[21] = 16;
        put(&f);
        for (j, &(w, h)) in sizes.iter().enumerate() {
            let mut d = [0u8; 34];
            d[..5].copy_from_slice(&[34, 0x24, 5, j as u8 + 1, 0]);
            d[5..7].copy_from_slice(&w.to_le_bytes());
            d[7..9].copy_from_slice(&h.to_le_bytes());
            d[25] = 2;
            d[26..30].copy_from_slice(&333_333u32.to_le_bytes());
            d[30..34].copy_from_slice(&666_666u32.to_le_bytes());
            put(&d);
        }
    }
    if mjpeg {
        put(&[11, 0x24, 6, fmts.len() as u8 + 1, 1, 0, 1, 0, 0, 0, 0]);
        let mut d = [0u8; 30];
        d[..5].copy_from_slice(&[30, 0x24, 7, 1, 0]);
        d[5..7].copy_from_slice(&1280u16.to_le_bytes());
        d[7..9].copy_from_slice(&720u16.to_le_bytes());
        d[25] = 1;
        d[26..30].copy_from_slice(&333_333u32.to_le_bytes());
        put(&d);
    }
    for (k, &mps) in eps.iter().enumerate() {
        put(&[9, 4, 1, k as u8 + 1, 1, 0x0E, 2, 0, 0]);
        let m = mps.to_le_bytes();
        put(&[7, 5, 0x81, 5, m[0], m[1], 1]);
    }
    n
}

/// One packet: a header of `hl` bytes (info byte `info`), then `data` bytes of value `v`.
fn pkt(out: &mut [u8], hl: u8, info: u8, data: usize, v: u8) -> usize {
    out[0] = hl;
    if hl >= 2 {
        out[1] = info | 0x80;
    }
    for b in out[2..(hl as usize).max(2)].iter_mut() {
        *b = 0;
    }
    let at = (hl as usize).max(2);
    for b in out[at..at + data].iter_mut() {
        *b = v;
    }
    at + data
}

/// `camera test`: the parsers on hand-made descriptors and packets, YUY2 colours, and the origin rule.
pub fn selftest() {
    // SAFETY: the shell runs one command at a time; ST and STF are used only here.
    let (b, p) = unsafe { (&mut *addr_of_mut!(ST), &mut *addr_of_mut!(STF)) };
    let (mut cases, mut fails) = (0usize, 0usize);
    let mut case = |name: &str, ok: bool, got: &dyn core::fmt::Debug| {
        cases += 1;
        if !ok {
            fails += 1;
        }
        crate::kprintln!("camera selftest: {}: {:?} ({})", name, got, if ok { "as expected" } else { "NOT as expected" });
    };
    let sizes: &[(u16, u16)] = &[(320, 240), (640, 480), (1280, 720)];

    // descriptors
    let n = cfg(b, 0x0110, &[(&YUY2, sizes)], true, &[0x0080, 0x0400, 0x1400]);
    let r = descriptors(&b[..n]);
    let ok = matches!(r, Ok(c) if c.nsizes == 3 && c.mjpeg && c.nalts == 3 && c.uvc == 0x0110 && c.vs_if == 1
        && pick(&c).is_some_and(|s| (s.w, s.h, s.interval, s.bytes) == (640, 480, 666_666, 614_400))
        && alt_for(&c, 2000).is_some_and(|a| (a.alt, a.mps, a.mult) == (3, 1024, 3)));
    case("a real-looking camera: 640x480 chosen at 15 a second, alt 3 for 2000 bytes", ok, &r.map(|c| pick(&c)));
    let n = cfg(b, 0x0100, &[], true, &[0x0400]);
    let r = descriptors(&b[..n]);
    case("MJPEG only: nothing to show in slice 1", matches!(r, Ok(c) if c.mjpeg && pick(&c).is_none()), &r.map(|c| c.nsizes));
    let n = cfg(b, 0x0110, &[(&YUY2, sizes)], false, &[0x0400]);
    b[n - 7] = 0; // the last endpoint's length
    case("a descriptor of length 0", descriptors(&b[..n]).err() == Some(Bad::ZeroLength), &descriptors(&b[..n]).err());
    let n = cfg(b, 0x0110, &[(&YUY2, &[(320, 240)])], false, &[0x0400]);
    let fd = 9 + 8 + 9 + 13 + 7 + 9 + 14 + 27; // where the frame descriptor starts
    let r = descriptors(&b[..fd + 20]);
    case("a frame descriptor past the end", r.err() == Some(Bad::PastEnd), &r.err());
    let _ = n;
    let many = [(320u16, 240u16); 100];
    let n = cfg(b, 0x0110, &[(&YUY2, &many[..])], false, &[0x0400]);
    let r = descriptors(&b[..n]);
    case("100 frame descriptors: 64 kept", matches!(r, Ok(c) if c.nsizes == 64 && c.more == 36), &r.map(|c| (c.nsizes, c.more)));
    let n = cfg(b, 0x0110, &[(&YUY2, sizes)], false, &[0x0400, 0x1C00]);
    let r = descriptors(&b[..n]);
    case("an endpoint with the reserved transaction count", r.err() == Some(Bad::ReservedMult), &r.err());
    let huge: &[(u16, u16)] = &[(65535, 65535), (640, 480)];
    let n = cfg(b, 0x0110, &[(&YUY2, huge)], false, &[0x1400]);
    let r = descriptors(&b[..n]);
    case(
        "65535x65535 does not overflow and is never chosen",
        matches!(r, Ok(c) if c.sizes[0].bytes == 0 && pick(&c).is_some_and(|s| s.w == 640)),
        &r.map(|c| c.sizes[0].bytes),
    );
    let n = cfg(b, 0x0110, &[(&[0x4E, 0x56, 0x31, 0x32, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71], sizes)], false, &[0x400]);
    let r = descriptors(&b[..n]);
    case("NV12 only: another format, nothing chosen", matches!(r, Ok(c) if c.other && pick(&c).is_none()), &r.map(|c| c.nsizes));

    // payloads: a 4x2 frame (16 bytes)
    let mut f = [0u8; 32];
    let run = |a: &mut Asm, f: &mut [u8], pk: &[u8]| -> u32 {
        let mut whole = 0;
        loop {
            if payload(a, pk, f) == Step::Whole {
                whole += 1;
                if a.again {
                    continue;
                }
            }
            return whole;
        }
    };
    let feed = |a: &mut Asm, f: &mut [u8], p: &mut [u8], list: &[(u8, u8, usize, u8)]| -> u32 {
        let mut w = 0;
        for &(hl, info, data, v) in list {
            let n = pkt(p, hl, info, data, v);
            w += run(a, f, &p[..n]);
        }
        w
    };
    // first the end of a frame we joined late (fid 1, EOF), then a clean frame (fid 0) in two packets
    let mut a = Asm::new(16);
    let w = feed(&mut a, &mut f, p, &[(12, 1, 6, 9), (12, 1 | 2, 0, 0), (12, 0, 8, 7), (12, 2, 8, 7)]);
    case("a clean frame after joining late", w == 1 && a.frames == 1 && a.dropped() == 0 && f[..16] == [7; 16], &(w, a.frames, a.dropped()));
    // the toggle ends a frame without EOF, the packet that toggled starts the next
    let mut a = Asm::new(16);
    let w = feed(&mut a, &mut f, p, &[(2, 1 | 2, 0, 0), (2, 0, 16, 5), (2, 1, 8, 6)]);
    case("a frame ended by the toggle alone", w == 1 && a.frames == 1 && a.fill == 8, &(w, a.frames, a.fill));
    let mut a = Asm::new(16);
    let w = feed(&mut a, &mut f, p, &[(2, 1 | 2, 0, 0), (2, 0, 8, 5), (2, 1, 8, 6), (2, 1 | 2, 8, 6)]);
    case("the toggle in the middle of a frame: dropped as short", w == 1 && a.drop_short == 1 && a.frames == 1, &(w, a.drop_short, a.frames));
    let mut a = Asm::new(16);
    let w = feed(&mut a, &mut f, p, &[(2, 1 | 2, 0, 0), (2, 0x40, 8, 5), (2, 2, 8, 5)]);
    case("the error bit: dropped", w == 0 && a.drop_err == 1, &(w, a.drop_err));
    let mut a = Asm::new(16);
    let w = feed(&mut a, &mut f, p, &[(2, 1 | 2, 0, 0), (2, 0, 8, 5), (2, 2, 9, 5)]);
    case("a frame one byte too long: dropped", w == 0 && a.drop_long == 1, &(w, a.drop_long));
    for hl in [0u8, 1, 13] {
        let mut a = Asm::new(16);
        let mut w = feed(&mut a, &mut f, p, &[(2, 1 | 2, 0, 0), (2, 0, 8, 5)]);
        p[0] = hl;
        p[1] = 0x80;
        w += run(&mut a, &mut f, &p[..if hl == 13 { 12 } else { 8 }]);
        w += feed(&mut a, &mut f, p, &[(2, 2, 8, 5)]);
        case(
            if hl == 0 { "a header length of 0" } else if hl == 1 { "a header length of 1" } else { "a header length of 13 in a 12-byte packet" },
            w == 0 && a.bad_headers == 1 && a.drop_header == 1,
            &(w, a.bad_headers, a.drop_header),
        );
    }

    // colours
    case("YUY2 white", rgb(235, 128, 128) == 0xFF_FFFF, &rgb(235, 128, 128));
    case("YUY2 black", rgb(16, 128, 128) == 0, &rgb(16, 128, 128));
    let red = rgb(81, 90, 240);
    case("YUY2 red", red >> 16 >= 0xF0 && (red >> 8) & 0xFF <= 0x10 && red & 0xFF <= 0x10, &red);

    // the ring slot of an isochronous event (39 packets)
    case("a packet's event in order", slot(5, 5, 39) == Slot::Next, &slot(5, 5, 39));
    case("one packet skipped without an event", slot(5, 6, 39) == Slot::Ahead(1), &slot(5, 6, 39));
    case("packets skipped across the end of the ring", slot(37, 1, 39) == Slot::Ahead(3), &slot(37, 1, 39));
    case("a packet already given back", slot(5, 4, 39) == Slot::Behind, &slot(5, 4, 39));

    // where commands come from
    case("camera on from the network is refused", start(Origin::Network).is_err(), &start(Origin::Network).err());

    crate::kprintln!("camera selftest: {} of {} cases as expected", cases - fails, cases);
}
