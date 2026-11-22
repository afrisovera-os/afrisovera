//! The Intel 7000-series Wi-Fi chip's words, read and written as plain bytes (Wi-Fi W5). Pure code over slices: no
//! registers, no memory map, so every function runs in the host tests and the fuzz target `iwm_rx`. Everything the
//! chip writes is untrusted input (the architect's firmware rule 4): every length is checked against the slice, and
//! a malformed record is refused whole.
//!
//! Facts from the published OpenBSD iwm driver (if_iwmreg.h, if_iwm.c; ISC), read for facts only and written down in
//! the notes folder's WIFI_W5_REGISTERS.md. Little endian throughout.
//! - An RX buffer holds packets one after another, each starting on a 64-byte boundary: a u32 `len_n_flags` (the
//!   frame length in bits 13:0, counting the 4-byte header and the data), the header (code, flags, idx, qid), then
//!   the data. 0x55550000 marks the end.
//! - ALIVE (code 0x1) comes in three layouts, chosen by its length: 44, 64 or 68 bytes.
//! - NVM_ACCESS (code 0x88) answers with an 8-byte header (offset, length, type, status) and the bytes read.
#![allow(dead_code)]

pub const ALIVE: u8 = 0x01;
pub const REPLY_ERROR: u8 = 0x02;
pub const INIT_COMPLETE_NOTIF: u8 = 0x04;
pub const PHY_CONFIGURATION_CMD: u8 = 0x6A;
pub const CALIB_RES_NOTIF_PHY_DB: u8 = 0x6B;
pub const NVM_ACCESS_CMD: u8 = 0x88;
pub const TX_ANT_CONFIGURATION_CMD: u8 = 0x98;
pub const BT_CONFIG: u8 = 0x9B;
pub const CARD_STATE_NOTIFICATION: u8 = 0xA1;
pub const REPLY_SF_CFG_CMD: u8 = 0xD1;

pub const ALIVE_STATUS_OK: u16 = 0xCAFE;
/// A reply's flags: the command failed.
pub const CMD_FAILED: u8 = 0x40;
/// The end of the packets in an RX buffer.
pub const FRAME_INVALID: u32 = 0x5555_0000;
pub const FRAME_SIZE_MASK: u32 = 0x3FFF;
pub const FRAME_ALIGN: usize = 64;

/// NVM sections of the 7000 family, and where their fields are (in 16-bit words from the section's start).
pub const NVM_HW: u16 = 0;
pub const NVM_SW: u16 = 1;
pub const NVM_CALIBRATION: u16 = 4;
pub const NVM_PRODUCTION: u16 = 5;
const HW_ADDR_WORD: usize = 0x15;
const SW_VERSION_WORD: usize = 0;
const SW_RADIO_CFG_WORD: usize = 1;
const SW_SKU_WORD: usize = 2;
const SW_N_HW_ADDRS_WORD: usize = 3;
const SW_CHANNELS_WORD: usize = 0x20;
/// The 7000 family's channel list, in the order the NVM gives their flags.
pub const CHANNELS: [u8; 39] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124,
    128, 132, 136, 140, 144, 149, 153, 157, 161, 165,
];
pub const CH_VALID: u16 = 1 << 0;
pub const CH_ACTIVE: u16 = 1 << 3;
pub const CH_RADAR: u16 = 1 << 4;
pub const SKU_24GHZ: u16 = 1 << 0;
pub const SKU_52GHZ: u16 = 1 << 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    Short,
    BadLength,
    BadStatus(u16),
    Unknown,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// One packet from the chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub code: u8,
    pub flags: u8,
    pub idx: u8,
    pub qid: u8,
    pub data: &'a [u8],
}

impl Packet<'_> {
    /// A reply to one of our commands (qid bit 7 clear), not a notification the chip sent on its own.
    pub fn is_reply(&self) -> bool {
        self.qid & 0x80 == 0
    }
}

/// Every packet in one RX buffer, in order, handed to `each`. Stops at the end marker, at a zero length, at a
/// packet the buffer cannot hold, or at the empty header the chip leaves after its last packet. Returns how many.
pub fn walk<'a>(buf: &'a [u8], mut each: impl FnMut(Packet<'a>)) -> usize {
    let mut at = 0usize;
    let mut n = 0;
    while buf.len().saturating_sub(at) >= 8 {
        let Some(lnf) = le32(buf, at) else { break };
        if lnf == FRAME_INVALID {
            break;
        }
        let len = (lnf & FRAME_SIZE_MASK) as usize; // the header and the data
        if len < 4 || len > buf.len() - at - 4 {
            break;
        }
        let (code, flags, idx, qid) = (buf[at + 4], buf[at + 5], buf[at + 6], buf[at + 7]);
        if code == 0 && idx == 0 && qid & 0x7F == 0 {
            break; // nothing written here
        }
        each(Packet { code, flags, idx, qid, data: &buf[at + 8..at + 4 + len] });
        n += 1;
        let next = (at + 4 + len + FRAME_ALIGN - 1) / FRAME_ALIGN * FRAME_ALIGN;
        if next <= at {
            break;
        }
        at = next;
    }
    n
}

/// The ALIVE notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alive {
    /// 1, 2 or 3: the layout, by length (44, 64, 68 bytes)
    pub layout: u8,
    pub status: u16,
    pub ucode_major: u32,
    pub ucode_minor: u32,
    pub error_table: u32,
    pub log_table: u32,
    /// the scheduler's SRAM base, where queue contexts live
    pub scd_base: u32,
    /// the UMAC's error table (layouts 2 and 3)
    pub umac_error: Option<u32>,
}

impl Alive {
    pub fn ok(&self) -> bool {
        self.status == ALIVE_STATUS_OK
    }
}

pub fn parse_alive(d: &[u8]) -> Result<Alive, Why> {
    let layout = match d.len() {
        44 => 1,
        64 => 2,
        68 => 3,
        _ => return Err(Why::BadLength),
    };
    let w = |at| le32(d, at).ok_or(Why::Short);
    let (major, minor) = if layout == 3 { (w(8)?, w(4)?) } else { (d[5] as u32, d[4] as u32) };
    Ok(Alive {
        layout,
        status: le16(d, 0).ok_or(Why::Short)?,
        ucode_major: major,
        ucode_minor: minor,
        error_table: w(20)?,
        log_table: w(24)?,
        scd_base: w(40)?,
        umac_error: match layout {
            2 => Some(w(56)?),
            3 => Some(w(60)?),
            _ => None,
        },
    })
}

/// One NVM_ACCESS answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NvmChunk<'a> {
    pub offset: u16,
    pub section: u16,
    pub bytes: &'a [u8],
}

/// An NVM_ACCESS answer: refused unless its status is 0 and its stated length fits the packet.
pub fn parse_nvm(d: &[u8]) -> Result<NvmChunk<'_>, Why> {
    if d.len() < 8 {
        return Err(Why::Short);
    }
    let (offset, length, section, status) = (le16(d, 0).unwrap(), le16(d, 2).unwrap(), le16(d, 4).unwrap(), le16(d, 6).unwrap());
    if status != 0 {
        return Err(Why::BadStatus(status));
    }
    let bytes = d.get(8..8 + length as usize).ok_or(Why::BadLength)?;
    Ok(NvmChunk { offset, section, bytes })
}

/// What the NVM says about this card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NvmInfo {
    pub mac: [u8; 6],
    pub version: u16,
    pub radio_cfg: u16,
    pub sku: u16,
    pub n_hw_addrs: u16,
    /// each channel of CHANNELS with its flags
    pub flags: [u16; 39],
}

impl NvmInfo {
    /// Channels the NVM marks valid: (2.4 GHz, 5 GHz). 5 GHz ones count only when the SKU has the band.
    pub fn valid(&self) -> (usize, usize) {
        let mut n = (0, 0);
        for (i, f) in self.flags.iter().enumerate() {
            if f & CH_VALID != 0 {
                if i < 14 {
                    n.0 += 1;
                } else if self.sku & SKU_52GHZ != 0 {
                    n.1 += 1;
                }
            }
        }
        n
    }
    /// TX antennas (radio config bits 11:8) and RX antennas (bits 15:12).
    pub fn antennas(&self) -> (u8, u8) {
        (((self.radio_cfg >> 8) & 0xF) as u8, ((self.radio_cfg >> 12) & 0xF) as u8)
    }
}

/// The hardware and software sections (as read from their start) into what we use. The 7000 family stores the
/// MAC address as 16-bit words: bytes come in the order 1,0,3,2,5,4.
pub fn parse_nvm_info(hw: &[u8], sw: &[u8]) -> Result<NvmInfo, Why> {
    let m = hw.get(HW_ADDR_WORD * 2..HW_ADDR_WORD * 2 + 6).ok_or(Why::Short)?;
    let mac = [m[1], m[0], m[3], m[2], m[5], m[4]];
    let w = |word: usize| le16(sw, word * 2).ok_or(Why::Short);
    let mut flags = [0u16; 39];
    for (i, f) in flags.iter_mut().enumerate() {
        *f = w(SW_CHANNELS_WORD + i)?;
    }
    Ok(NvmInfo {
        mac,
        version: w(SW_VERSION_WORD)?,
        radio_cfg: w(SW_RADIO_CFG_WORD)?,
        sku: w(SW_SKU_WORD)?,
        n_hw_addrs: w(SW_N_HW_ADDRS_WORD)?,
        flags,
    })
}

/// A calibration result (CALIB_RES_NOTIF_PHY_DB): its type and its length as stated, checked against the packet.
pub fn parse_phy_db(d: &[u8]) -> Result<(u16, &[u8]), Why> {
    let ty = le16(d, 0).ok_or(Why::Short)?;
    let len = le16(d, 2).ok_or(Why::Short)? as usize;
    Ok((ty, d.get(4..4 + len).ok_or(Why::BadLength)?))
}

// ---- what we write -------------------------------------------------------------------------------------------------

/// One transfer buffer of a TFD: the DMA address's low 32 bits, then a u16 holding address bits 35:32 and the length
/// (12 bits) shifted by 4. None if the address is above 36 bits or the length does not fit.
pub fn tb(addr: u64, len: usize) -> Option<[u8; 6]> {
    if addr >> 36 != 0 || len > 0xFFF {
        return None;
    }
    let lo = (addr as u32).to_le_bytes();
    let hi = (((addr >> 32) as u16) & 0xF) | ((len as u16) << 4);
    let h = hi.to_le_bytes();
    Some([lo[0], lo[1], lo[2], lo[3], h[0], h[1]])
}

/// A whole TFD (128 bytes) with these transfer buffers (1 to 20).
pub fn tfd(tbs: &[(u64, usize)]) -> Option<[u8; 128]> {
    if tbs.is_empty() || tbs.len() > 20 {
        return None;
    }
    let mut t = [0u8; 128];
    t[3] = tbs.len() as u8;
    for (i, &(a, l)) in tbs.iter().enumerate() {
        t[4 + 6 * i..10 + 6 * i].copy_from_slice(&tb(a, l)?);
    }
    Some(t)
}

/// A command for the chip: the 4-byte header (code, flags 0, idx, qid), then the payload. Returns its length.
pub fn command(code: u8, idx: u8, qid: u8, payload: &[u8], out: &mut [u8]) -> Option<usize> {
    let n = 4 + payload.len();
    let o = out.get_mut(..n)?;
    o[..4].copy_from_slice(&[code, 0, idx, qid]);
    o[4..].copy_from_slice(payload);
    Some(n)
}

/// The scheduler's byte-count entry for a command of `len` bytes (header included): length plus 8 (CRC and
/// delimiter), station 0.
pub fn byte_count(len: usize) -> u16 {
    ((len + 8) & 0xFFF) as u16
}

/// NVM_ACCESS read of `length` bytes of `section` from `offset`.
pub fn nvm_read_payload(section: u16, offset: u16, length: u16) -> [u8; 8] {
    let (s, o, l) = (section.to_le_bytes(), offset.to_le_bytes(), length.to_le_bytes());
    [0, 0, s[0], s[1], o[0], o[1], l[0], l[1]]
}

/// The firmware file for this chip: every 3165 and 7265D silicon (CSR_HW_REV type 0x210) take 7265D-29, an early 7265
/// takes 7265-17. None for any other device.
pub fn firmware_for(device: u16, hw_rev: u32) -> Option<&'static str> {
    match device {
        0x3165 | 0x3166 => Some("iwlwifi-7265D-29.ucode"),
        0x095A | 0x095B if hw_rev & 0xFFF0 == 0x210 => Some("iwlwifi-7265D-29.ucode"),
        0x095A | 0x095B => Some("iwlwifi-7265-17.ucode"),
        _ => None,
    }
}

/// One DMA chunk of a firmware upload: where in the chip's memory, where in the section, how long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub dst: u32,
    pub from: usize,
    pub len: usize,
}

/// The chunks of one section of `len` bytes going to `dst`, each at most `max` bytes, in order.
pub fn chunks(dst: u32, len: usize, max: usize) -> impl Iterator<Item = Chunk> {
    let max = max.max(1);
    (0..len.div_ceil(max)).map(move |i| {
        let from = i * max;
        Chunk { dst: dst.wrapping_add(from as u32), from, len: (len - from).min(max) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec;
    use std::vec::Vec;

    fn packet(code: u8, idx: u8, qid: u8, data: &[u8]) -> Vec<u8> {
        let mut p = ((4 + data.len()) as u32).to_le_bytes().to_vec();
        p.extend_from_slice(&[code, 0, idx, qid]);
        p.extend_from_slice(data);
        while p.len() % FRAME_ALIGN != 0 {
            p.push(0);
        }
        p
    }

    #[test]
    fn walk_reads_packets_and_stops_at_the_end() {
        let mut b = packet(ALIVE, 0, 0x80, &[1; 44]);
        b.extend(packet(NVM_ACCESS_CMD, 3, 9, &[2; 100]));
        b.extend(FRAME_INVALID.to_le_bytes());
        b.resize(4096, 0);
        let mut got = Vec::new();
        assert_eq!(walk(&b, |p| got.push((p.code, p.idx, p.qid, p.data.len(), p.is_reply()))), 2);
        assert_eq!(got, vec![(ALIVE, 0, 0x80, 44, false), (NVM_ACCESS_CMD, 3, 9, 100, true)]);
    }

    #[test]
    fn walk_never_reads_past_the_buffer() {
        // a stated length past the end, a zero length, an empty header: each stops the walk
        let mut b = packet(ALIVE, 0, 0x80, &[1; 44]);
        let at = b.len();
        b.extend(5000u32.to_le_bytes());
        b.extend([7, 0, 1, 9]);
        b.resize(4096, 0);
        assert_eq!(walk(&b, |_| {}), 1);
        b[at..at + 4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(walk(&b, |_| {}), 1);
        b[at..at + 4].copy_from_slice(&8u32.to_le_bytes());
        b[at + 4..at + 8].copy_from_slice(&[0, 0, 0, 0x80]);
        assert_eq!(walk(&b, |_| {}), 1, "an empty header (qid bits aside) ends the packets");
        assert_eq!(walk(&[], |_| {}), 0);
        assert_eq!(walk(&[1, 2, 3], |_| {}), 0);
        // every cut of a good buffer: never a panic, never a packet past the cut
        let full = packet(NVM_ACCESS_CMD, 1, 9, &[3; 200]);
        for cut in 0..full.len() {
            walk(&full[..cut], |p| assert!(p.data.len() <= cut));
        }
    }

    #[test]
    fn alive_in_its_three_layouts() {
        let mut v1 = vec![0u8; 44];
        v1[..2].copy_from_slice(&0xCAFEu16.to_le_bytes());
        v1[4] = 2;
        v1[5] = 17;
        v1[20..24].copy_from_slice(&0x0080_0000u32.to_le_bytes());
        v1[40..44].copy_from_slice(&0x0000_A000u32.to_le_bytes());
        let a = parse_alive(&v1).unwrap();
        assert_eq!((a.layout, a.ok(), a.ucode_major, a.ucode_minor, a.error_table, a.scd_base, a.umac_error), (1, true, 17, 2, 0x80_0000, 0xA000, None));
        let mut v2 = v1.clone();
        v2.resize(64, 0);
        v2[56..60].copy_from_slice(&0x1234u32.to_le_bytes());
        assert_eq!(parse_alive(&v2).unwrap().umac_error, Some(0x1234));
        let mut v3 = vec![0u8; 68];
        v3[..2].copy_from_slice(&0xDEADu16.to_le_bytes());
        v3[4..8].copy_from_slice(&5u32.to_le_bytes());
        v3[8..12].copy_from_slice(&29u32.to_le_bytes());
        v3[40..44].copy_from_slice(&0xB000u32.to_le_bytes());
        v3[60..64].copy_from_slice(&0x99u32.to_le_bytes());
        let a = parse_alive(&v3).unwrap();
        assert_eq!((a.layout, a.ok(), a.ucode_major, a.ucode_minor, a.scd_base, a.umac_error), (3, false, 29, 5, 0xB000, Some(0x99)));
        for n in [0, 43, 45, 63, 65, 67, 69] {
            assert_eq!(parse_alive(&vec![0; n]), Err(Why::BadLength));
        }
    }

    #[test]
    fn nvm_answers_and_the_card_info() {
        let mut ans = vec![0u8; 8];
        ans[2..4].copy_from_slice(&6u16.to_le_bytes());
        ans.extend([1, 2, 3, 4, 5, 6]);
        assert_eq!(parse_nvm(&ans).unwrap().bytes, &[1, 2, 3, 4, 5, 6]);
        let mut bad = ans.clone();
        bad[6] = 1;
        assert_eq!(parse_nvm(&bad), Err(Why::BadStatus(1)));
        let mut long = ans.clone();
        long[2] = 7;
        assert_eq!(parse_nvm(&long), Err(Why::BadLength));
        assert_eq!(parse_nvm(&ans[..7]), Err(Why::Short));
        // the hardware section: the MAC as words at word 0x15
        let mut hw = vec![0u8; 0x2A + 6];
        hw[0x2A..].copy_from_slice(&[0x16, 0x00, 0x2E, 0xEA, 0x56, 0x34]);
        // the software section: version, radio config, SKU, addresses, then the channel flags at word 0x20
        let mut sw = vec![0u8; (0x20 + 39) * 2];
        sw[0..2].copy_from_slice(&0x0B20u16.to_le_bytes());
        sw[2..4].copy_from_slice(&0x3300u16.to_le_bytes());
        sw[4..6].copy_from_slice(&(SKU_24GHZ | SKU_52GHZ).to_le_bytes());
        for i in 0..39 {
            let f: u16 = if i < 13 || (14..20).contains(&i) { CH_VALID | CH_ACTIVE } else { 0 };
            sw[(0x20 + i) * 2..(0x20 + i) * 2 + 2].copy_from_slice(&f.to_le_bytes());
        }
        let info = parse_nvm_info(&hw, &sw).unwrap();
        assert_eq!(info.mac, [0x00, 0x16, 0xEA, 0x2E, 0x34, 0x56]);
        assert_eq!((info.version, info.sku), (0x0B20, 3));
        assert_eq!(info.antennas(), (3, 3));
        assert_eq!(info.valid(), (13, 6));
        let mut no5 = info;
        no5.sku = SKU_24GHZ;
        assert_eq!(no5.valid(), (13, 0), "5 GHz channels need the SKU's band");
        assert_eq!(parse_nvm_info(&hw[..0x2A + 5], &sw), Err(Why::Short));
        assert_eq!(parse_nvm_info(&hw, &sw[..sw.len() - 1]), Err(Why::Short));
        assert_eq!(parse_phy_db(&[1, 0, 2, 0, 9, 9]).unwrap(), (1, &[9u8, 9][..]));
        assert_eq!(parse_phy_db(&[1, 0, 3, 0, 9, 9]), Err(Why::BadLength));
    }

    #[test]
    fn tfds_commands_and_byte_counts() {
        assert_eq!(tb(0x9_1234_5678, 100), Some([0x78, 0x56, 0x34, 0x12, 0x49, 0x06]));
        assert_eq!(tb(1 << 36, 4), None, "above the chip's 36-bit DMA reach");
        assert_eq!(tb(0x1000, 0x1000), None);
        let t = tfd(&[(0x1000, 40)]).unwrap();
        assert_eq!((t[3], &t[4..10]), (1, &[0x00, 0x10, 0, 0, 0x80, 0x02][..]));
        assert!(tfd(&[]).is_none() && tfd(&[(0, 1); 21]).is_none());
        let mut c = [0u8; 16];
        assert_eq!(command(BT_CONFIG, 7, 9, &[3, 0, 0, 0], &mut c), Some(8));
        assert_eq!(&c[..8], &[BT_CONFIG, 0, 7, 9, 3, 0, 0, 0]);
        assert_eq!(command(BT_CONFIG, 0, 9, &[0; 13], &mut c), None);
        assert_eq!(byte_count(12), 20);
        assert_eq!(nvm_read_payload(1, 0x800, 2048), [0, 0, 1, 0, 0, 8, 0, 8]);
    }

    #[test]
    fn the_firmware_file_by_device_and_revision() {
        assert_eq!(firmware_for(0x095A, 0x210), Some("iwlwifi-7265D-29.ucode"));
        assert_eq!(firmware_for(0x095B, 0x21C), Some("iwlwifi-7265D-29.ucode"), "step and dash bits do not matter");
        assert_eq!(firmware_for(0x095A, 0x144), Some("iwlwifi-7265-17.ucode"));
        assert_eq!(firmware_for(0x3165, 0), Some("iwlwifi-7265D-29.ucode"));
        assert_eq!(firmware_for(0x08B1, 0x210), None, "a 7260 is not driven");
    }

    #[test]
    fn chunks_cover_a_section_exactly_and_never_exceed_the_limit() {
        for (len, max) in [(196_608usize, 0x20000usize), (69_268, 0x20000), (0x20000, 0x20000), (0x20001, 0x20000), (5, 2), (0, 8)] {
            let c: Vec<Chunk> = chunks(0x4_0000, len, max).collect();
            assert!(c.iter().all(|k| k.len <= max && k.len > 0), "a chunk over the limit");
            assert_eq!(c.iter().map(|k| k.len).sum::<usize>(), len, "the chunks do not cover the section");
            for w in c.windows(2) {
                assert_eq!(w[0].from + w[0].len, w[1].from, "a gap or an overlap");
            }
            for k in &c {
                assert_eq!(k.dst, 0x4_0000 + k.from as u32);
            }
        }
    }
}
