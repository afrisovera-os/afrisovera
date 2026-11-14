//! USB CDC-NCM 1.0 transfer blocks (NTB16), as pure functions on byte slices (13 Nov 2026, CDC_NCM_PLAN.md).
//!
//! NCM moves Ethernet frames inside NTBs: a 12-byte NTH16 header ("NCMH"), then one or more NDP16 tables
//! ("NCM0") that list (offset, length) for each frame; one USB transfer can carry many frames. Written from the
//! USB-IF "Universal Serial Bus Communications Class Subclass Specification for Network Control Model Devices"
//! 1.0, sections 3.2 to 3.3 and 6.2.1. No USB in this file: xhci.rs hands it the bytes a transfer brought and
//! sends what `build` made.
//!
//! The device is not trusted (architect's conditions, ARCHITECT_REVIEWS 13 Nov): the block length is the
//! smaller of what the header says and what arrived; every index and length is checked against it; an NDP's
//! entries stop at its own length or at the first zero entry; at most 8 NDPs and 64 frames per block (a chain
//! pointing back at itself ends there); a frame shorter than 14 or longer than 1518 bytes is skipped and counted.
//! CRC tables (NCM1) and 32-bit blocks are refused: the driver never asks for them.

/// "NCMH", the NTH16 signature, as a little-endian u32.
pub const NTH16_SIG: u32 = 0x484D_434E;
/// "NCM0": an NDP16 without CRCs.
pub const NDP16_SIG: u32 = 0x304D_434E;
/// "NCM1": an NDP16 with CRCs (never asked for).
pub const NDP16_CRC_SIG: u32 = 0x314D_434E;
pub const NTH16_LEN: usize = 12;
pub const MAX_NDPS: usize = 8;
pub const MAX_FRAMES: usize = 64;
pub const MIN_FRAME: usize = 14;
pub const MAX_FRAME: usize = 1514 + 4;

/// Why a whole block was dropped (frames taken before it stay taken).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bad {
    /// fewer than 12 bytes, or a block length under 12
    Short,
    /// not "NCMH", or a header length other than 12
    Header,
    /// an NDP index of 0, not a multiple of 4, inside the header, or past the block
    NdpIndex,
    /// not "NCM0" ("NCM1" included: CRCs were never asked for)
    NdpSignature,
    /// an NDP length under 16, not a multiple of 4, or past the block
    NdpLength,
    /// more than 8 NDPs chained (a chain pointing back on itself ends here)
    TooManyNdps,
}

impl Bad {
    pub fn name(self) -> &'static str {
        match self {
            Bad::Short => "too short",
            Bad::Header => "bad header",
            Bad::NdpIndex => "bad table index",
            Bad::NdpSignature => "bad table signature",
            Bad::NdpLength => "bad table length",
            Bad::TooManyNdps => "more than 8 tables",
        }
    }
}

/// What one block gave.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Got {
    /// frames handed on
    pub frames: usize,
    /// frames refused by the taker (its queue was full)
    pub dropped: usize,
    /// entries skipped: too short, too long, past the block, or past the 64-frame cap
    pub skipped: usize,
    /// NDPs read
    pub ndps: usize,
    pub bad: Option<Bad>,
}

fn le16(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]) as usize)
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Walk one received block; `put` takes each frame and says false when it has no room (the frame is then
/// counted as dropped, never written anywhere).
pub fn parse(rx: &[u8], mut put: impl FnMut(&[u8]) -> bool) -> Got {
    let mut g = Got::default();
    if rx.len() < NTH16_LEN {
        g.bad = Some(Bad::Short);
        return g;
    }
    if le32(rx, 0) != Some(NTH16_SIG) || le16(rx, 4) != Some(NTH16_LEN) {
        g.bad = Some(Bad::Header);
        return g;
    }
    // wBlockLength 0: the block ends where the transfer did (NCM 1.0 table 3-1)
    let said = le16(rx, 8).unwrap_or(0);
    let blen = if said == 0 { rx.len() } else { said.min(rx.len()) };
    if blen < NTH16_LEN {
        g.bad = Some(Bad::Short);
        return g;
    }
    let b = &rx[..blen];
    let mut ndp = le16(b, 10).unwrap_or(0);
    let mut seen = 0usize; // entries looked at, toward the 64 cap
    loop {
        if ndp < NTH16_LEN || ndp % 4 != 0 || ndp.checked_add(8).map_or(true, |e| e > blen) {
            g.bad = Some(Bad::NdpIndex);
            return g;
        }
        if g.ndps == MAX_NDPS {
            g.bad = Some(Bad::TooManyNdps);
            return g;
        }
        g.ndps += 1;
        if le32(b, ndp) != Some(NDP16_SIG) {
            g.bad = Some(Bad::NdpSignature);
            return g;
        }
        let nlen = le16(b, ndp + 4).unwrap_or(0);
        let nend = match ndp.checked_add(nlen) {
            Some(e) if nlen >= 16 && nlen % 4 == 0 && e <= blen => e,
            _ => {
                g.bad = Some(Bad::NdpLength);
                return g;
            }
        };
        let next = le16(b, ndp + 6).unwrap_or(0);
        let mut e = ndp + 8;
        while let Some(stop) = e.checked_add(4) {
            if stop > nend {
                break;
            }
            let (di, dl) = (le16(b, e).unwrap_or(0), le16(b, e + 2).unwrap_or(0));
            e = stop;
            if di == 0 || dl == 0 {
                break; // the table's end
            }
            seen += 1;
            if seen > MAX_FRAMES {
                g.skipped += 1;
                continue;
            }
            let fits = di >= NTH16_LEN && di.checked_add(dl).map_or(false, |end| end <= blen);
            if !fits || !(MIN_FRAME..=MAX_FRAME).contains(&dl) {
                g.skipped += 1;
                continue;
            }
            if put(&b[di..di + dl]) {
                g.frames += 1;
            } else {
                g.dropped += 1;
            }
        }
        if next == 0 {
            return g;
        }
        ndp = next;
    }
}

/// The device's NTB parameters (GET_NTB_PARAMETERS, NCM 1.0 6.2.1), checked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Params {
    pub formats: u16,
    pub in_max: usize,
    pub out_max: usize,
    pub out_div: usize,
    pub out_rem: usize,
    pub out_align: usize,
    pub out_datagrams: usize,
}

/// Read and check the 28-byte reply. `tx_room` is the size of one transmit buffer: the largest frame must fit
/// in one block there, with the device's alignment, or the device is refused (the caller falls back to ECM).
pub fn params(r: &[u8], tx_room: usize) -> Result<Params, &'static str> {
    if r.len() < 28 || le16(r, 0).unwrap_or(0) < 28 {
        return Err("the NTB parameters are shorter than 28 bytes");
    }
    let formats = le16(r, 2).unwrap_or(0) as u16;
    if formats & 1 == 0 {
        return Err("the device does not offer 16-bit blocks");
    }
    let in_max = le32(r, 4).unwrap_or(0) as usize;
    let out_max = le32(r, 16).unwrap_or(0) as usize;
    if in_max < 2048 || out_max < 2048 {
        return Err("a block size under 2048 bytes");
    }
    let out_div = le16(r, 20).unwrap_or(0);
    let out_rem = le16(r, 22).unwrap_or(0);
    let raw_align = le16(r, 24).unwrap_or(0);
    if out_div == 0 || !out_div.is_power_of_two() {
        return Err("the out divisor is not a power of two");
    }
    if out_rem >= out_div {
        return Err("the out payload remainder is not smaller than the divisor");
    }
    if raw_align != 0 && !raw_align.is_power_of_two() {
        return Err("the out table alignment is not a power of two");
    }
    let p = Params {
        formats,
        in_max,
        out_max,
        out_div,
        out_rem,
        out_align: raw_align.max(4),
        out_datagrams: le16(r, 26).unwrap_or(0),
    };
    let (_, end) = layout(&p, 1514);
    if end + 1 > tx_room.min(out_max) {
        return Err("the device's alignment does not leave room for a full frame in one block");
    }
    Ok(p)
}

fn up(x: usize, a: usize) -> usize {
    (x + a - 1) & !(a - 1)
}

/// Where the table and the frame go in an outgoing block: (frame offset, end of the frame).
fn layout(p: &Params, n: usize) -> (usize, usize) {
    let ndp = up(NTH16_LEN, p.out_align);
    let mut d = ndp + 16; // one entry and the zero entry that ends the table
    d += (p.out_rem + p.out_div - d % p.out_div) % p.out_div;
    (d, d + n)
}

/// One frame into one block in `out`; the block's length, or None when it does not fit. `mps` is the bulk-OUT
/// packet size: a block that would end exactly on a packet boundary gets one zero byte more inside it, so no
/// zero-length packet is needed to end the transfer (as the RNDIS path does).
pub fn build(frame: &[u8], seq: u16, p: &Params, mps: usize, out: &mut [u8]) -> Option<usize> {
    let ndp = up(NTH16_LEN, p.out_align);
    let (d, mut end) = layout(p, frame.len());
    if mps > 0 && end % mps == 0 {
        end += 1;
    }
    if frame.is_empty() || end > out.len() || end > p.out_max || end > 0xFFFF {
        return None;
    }
    out[..end].fill(0);
    let w16 = |o: &mut [u8], at: usize, v: usize| o[at..at + 2].copy_from_slice(&(v as u16).to_le_bytes());
    out[0..4].copy_from_slice(&NTH16_SIG.to_le_bytes());
    w16(out, 4, NTH16_LEN);
    w16(out, 6, seq as usize);
    w16(out, 8, end);
    w16(out, 10, ndp);
    out[ndp..ndp + 4].copy_from_slice(&NDP16_SIG.to_le_bytes());
    w16(out, ndp + 4, 16);
    w16(out, ndp + 6, 0);
    w16(out, ndp + 8, d);
    w16(out, ndp + 10, frame.len());
    out[d..d + frame.len()].copy_from_slice(frame);
    Some(end)
}

// --- the selftest: hand-made blocks, the hostile ones included, and build-then-parse round trips ---

static mut ST: [u8; 4096] = [0; 4096];

/// A block made by hand: header, then tables at the given offsets, each with its entries and next index, then
/// frames of `fill` bytes wherever the entries point. Returns its length (`blen` as written, or the end).
fn hand(buf: &mut [u8], blen: Option<usize>, first: usize, tables: &[(usize, usize, &[(usize, usize)], usize)], end: usize) -> usize {
    buf[..end].fill(0);
    buf[0..4].copy_from_slice(&NTH16_SIG.to_le_bytes());
    buf[4..6].copy_from_slice(&12u16.to_le_bytes());
    buf[8..10].copy_from_slice(&(blen.unwrap_or(end) as u16).to_le_bytes());
    buf[10..12].copy_from_slice(&(first as u16).to_le_bytes());
    for &(at, len, ents, next) in tables {
        buf[at..at + 4].copy_from_slice(&NDP16_SIG.to_le_bytes());
        buf[at + 4..at + 6].copy_from_slice(&(len as u16).to_le_bytes());
        buf[at + 6..at + 8].copy_from_slice(&(next as u16).to_le_bytes());
        for (k, &(di, dl)) in ents.iter().enumerate() {
            let e = at + 8 + 4 * k;
            buf[e..e + 2].copy_from_slice(&(di as u16).to_le_bytes());
            buf[e + 2..e + 4].copy_from_slice(&(dl as u16).to_le_bytes());
            if di + dl <= end && di >= 12 {
                for (j, x) in buf[di..di + dl].iter_mut().enumerate() {
                    *x = (k as u8).wrapping_mul(17).wrapping_add(j as u8);
                }
            }
        }
    }
    end
}

/// `usbnettest ncm`: every case prints one line; the last line says how many came out as expected.
pub fn selftest() {
    // SAFETY: the shell runs one command at a time; ST is used only here.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(ST) };
    let (mut cases, mut fails) = (0usize, 0usize);
    let mut case = |name: &str, ok: bool, got: &dyn core::fmt::Debug| {
        cases += 1;
        if !ok {
            fails += 1;
        }
        crate::kprintln!("ncm selftest: {}: {:?} ({})", name, got, if ok { "as expected" } else { "NOT as expected" });
    };
    let count = |b: &[u8], room: usize| {
        let mut n = 0usize;
        parse(b, |_| {
            n += 1;
            n <= room
        })
    };
    let want = |frames, skipped, dropped, ndps, bad| Got { frames, skipped, dropped, ndps, bad };

    // three frames in one table
    let n = hand(buf, None, 12, &[(12, 24, &[(40, 60), (100, 64), (164, 1514)], 0)], 1678);
    let g = count(&buf[..n], 99);
    case("three frames, one table", g == want(3, 0, 0, 1, None), &g);
    // two tables chained: two frames, then one
    let n = hand(buf, None, 12, &[(12, 20, &[(64, 60), (124, 60)], 40), (40, 16, &[(184, 100)], 0)], 284);
    let g = count(&buf[..n], 99);
    case("two tables chained", g == want(3, 0, 0, 2, None), &g);
    // a table whose next index is itself: stopped at 8 tables
    let n = hand(buf, None, 12, &[(12, 16, &[(40, 60)], 12)], 100);
    let g = count(&buf[..n], 99);
    case("table pointing at itself", g.bad == Some(Bad::TooManyNdps) && g.ndps == MAX_NDPS, &g);
    // two tables pointing back at each other
    let n = hand(buf, None, 40, &[(12, 16, &[(80, 60)], 40), (40, 16, &[(140, 60)], 12)], 200);
    let g = count(&buf[..n], 99);
    case("tables pointing backwards", g.bad == Some(Bad::TooManyNdps) && g.ndps == MAX_NDPS, &g);
    // the header says more bytes than arrived: the block ends where the transfer did, the frame past it skipped
    let n = hand(buf, Some(4000), 12, &[(12, 20, &[(40, 60), (100, 600)], 0)], 400);
    let g = count(&buf[..n], 99);
    case("block length past the transfer", g == want(1, 1, 0, 1, None), &g);
    // wBlockLength 0: the transfer's length
    let n = hand(buf, Some(0), 12, &[(12, 16, &[(40, 60)], 0)], 100);
    let g = count(&buf[..n], 99);
    case("block length 0", g == want(1, 0, 0, 1, None), &g);
    // table index not a multiple of 4, inside the header, past the end
    let n = hand(buf, None, 14, &[], 100);
    let g = count(&buf[..n], 99);
    case("table index unaligned", g.bad == Some(Bad::NdpIndex) && g.frames == 0, &g);
    let n = hand(buf, None, 8, &[], 100);
    let g = count(&buf[..n], 99);
    case("table index inside the header", g.bad == Some(Bad::NdpIndex), &g);
    let n = hand(buf, None, 96, &[], 100);
    let g = count(&buf[..n], 99);
    case("table index past the block", g.bad == Some(Bad::NdpIndex), &g);
    // NCM1 (CRCs) refused
    let n = hand(buf, None, 12, &[(12, 16, &[(40, 60)], 0)], 100);
    buf[15] = b'1';
    let g = count(&buf[..n], 99);
    case("CRC table refused", g.bad == Some(Bad::NdpSignature) && g.frames == 0, &g);
    // wrong header signature, short block
    let n = hand(buf, None, 12, &[(12, 16, &[(40, 60)], 0)], 100);
    buf[3] = b'X';
    let g = count(&buf[..n], 99);
    case("wrong header", g.bad == Some(Bad::Header), &g);
    let g = count(&buf[..8], 99);
    case("8 bytes only", g.bad == Some(Bad::Short), &g);
    // a table longer than the block, and one shorter than 16
    let n = hand(buf, None, 12, &[(12, 200, &[(40, 60)], 0)], 120);
    let g = count(&buf[..n], 99);
    case("table past the block", g.bad == Some(Bad::NdpLength), &g);
    let n = hand(buf, None, 12, &[(12, 12, &[], 0)], 120);
    let g = count(&buf[..n], 99);
    case("table shorter than 16", g.bad == Some(Bad::NdpLength), &g);
    // frames too short, too long, past the end, inside the header: skipped and counted, the good one taken
    let n = hand(buf, None, 12, &[(12, 32, &[(60, 10), (80, 2000), (90, 1000), (4, 20), (120, 64)], 0)], 400);
    let g = count(&buf[..n], 99);
    case("bad frames skipped", g == want(1, 4, 0, 1, None), &g);
    // 64 tiny frames into a queue with room for 16: the rest dropped and counted, nothing written past it
    let mut ents = [(0usize, 0usize); 65];
    for (k, e) in ents.iter_mut().enumerate() {
        *e = (300 + 14 * k, 14);
    }
    let n = hand(buf, None, 12, &[(12, 8 + 4 * 65, &ents[..64], 0)], 300 + 14 * 65);
    let g = count(&buf[..n], 16);
    case("64 tiny frames, room for 16", g == want(16, 0, 48, 1, None), &g);
    // 65 entries: the 65th is past the cap
    let n = hand(buf, None, 12, &[(12, 8 + 4 * 66, &ents[..65], 0)], 300 + 14 * 65);
    let g = count(&buf[..n], 99);
    case("65 frames, cap 64", g == want(64, 1, 0, 1, None), &g);

    // build then parse, with the alignments and remainders devices ask for
    let mut ok_all = true;
    let mut frame = [0u8; 1514];
    for (j, x) in frame.iter_mut().enumerate() {
        *x = (j * 7) as u8;
    }
    for &(align, div, rem, len, mps) in &[
        (4usize, 4usize, 0usize, 1514usize, 512usize),
        (64, 4, 0, 60, 64),
        (512, 4, 0, 1514, 512),
        (4, 8, 2, 333, 512),
        (4, 4, 0, 512 - 28, 512), // ends exactly on 512: one pad byte
    ] {
        let p = Params { formats: 1, in_max: 16384, out_max: 16384, out_div: div, out_rem: rem, out_align: align, out_datagrams: 0 };
        let (d, _) = layout(&p, len);
        let mut out = [0u8; 2048];
        let ok = match build(&frame[..len], 7, &p, mps, &mut out) {
            Some(n) => {
                let mut same = false;
                let g = parse(&out[..n], |f| {
                    same = f == &frame[..len];
                    true
                });
                g == want(1, 0, 0, 1, None) && same && n % mps != 0 && d % div == rem && le16(&out, 10).map_or(false, |x| x % align == 0)
            }
            None => false,
        };
        if !ok {
            ok_all = false;
            crate::kprintln!("ncm selftest: round trip align {} div {} rem {} len {}: FAILED", align, div, rem, len);
        }
    }
    case("build then parse (5 layouts)", ok_all, &ok_all);

    // the device's parameters: one good set, and the ones that must be refused
    let mk = |fmt: u16, inmax: u32, outmax: u32, div: u16, rem: u16, align: u16| {
        let mut r = [0u8; 28];
        r[0..2].copy_from_slice(&28u16.to_le_bytes());
        r[2..4].copy_from_slice(&fmt.to_le_bytes());
        r[4..8].copy_from_slice(&inmax.to_le_bytes());
        r[16..20].copy_from_slice(&outmax.to_le_bytes());
        r[20..22].copy_from_slice(&div.to_le_bytes());
        r[22..24].copy_from_slice(&rem.to_le_bytes());
        r[24..26].copy_from_slice(&align.to_le_bytes());
        r
    };
    let good = params(&mk(1, 16384, 16384, 4, 0, 4), 2048);
    case("parameters: a usual device", good.is_ok(), &good.map(|p| p.out_align));
    let mut refused = 0;
    for r in [
        mk(2, 16384, 16384, 4, 0, 4),  // 32-bit blocks only
        mk(1, 1024, 16384, 4, 0, 4),   // IN blocks too small
        mk(1, 16384, 16384, 0, 0, 4),  // divisor 0
        mk(1, 16384, 16384, 3, 0, 4),  // divisor not a power of two
        mk(1, 16384, 16384, 4, 4, 4),  // remainder not below the divisor
        mk(1, 16384, 16384, 4, 0, 6),  // alignment not a power of two
        mk(1, 16384, 16384, 4, 0, 1024), // alignment leaves no room in a 2 KiB buffer
    ] {
        if params(&r, 2048).is_err() {
            refused += 1;
        }
    }
    case("parameters: 7 bad sets refused", refused == 7, &refused);

    crate::kprintln!("ncm selftest: {} of {} cases as expected", cases - fails, cases);
}
