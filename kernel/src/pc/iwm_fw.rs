//! The Intel 7000-series firmware file (`.ucode`, TLV format) read with every length checked (Wi-Fi W5). Pure code
//! over a byte slice: nothing is copied, sections are kept as places in the file. The file is read only AFTER the
//! firmware store has matched its sha256 against the MANIFEST (fwdb.rs, the driver engineer's rule 4), so this is
//! defence in depth: a damaged or hostile file is refused whole, never half used.
//!
//! The layout is from the published OpenBSD iwm driver's header (if_iwmreg.h, if_iwmvar.h; ISC), read for facts:
//! - an 88-byte header: u32 zero, u32 magic 0x0a4c5749, 64 bytes of name, u32 version, u32 build, 8 bytes unused;
//! - then records: u32 type, u32 length, the payload, padded to a multiple of 4 (the last record may lack its
//!   padding). Little endian throughout.
//! Firmware sections (types 19, 20, 21, 34) are a u32 device address and the bytes to load there, at most 16 per
//! image. Unknown record types are refused, like OpenBSD's driver does; the ones known to carry nothing we use are
//! skipped by name.
#![allow(dead_code)]

pub const MAGIC: u32 = 0x0a4c_5749;
pub const HEADER: usize = 88;
/// Sections per image (if_iwmvar.h IWM_UCODE_SECT_MAX).
pub const SECT_MAX: usize = 16;
/// The largest section the 7000 family loads in one go (if_iwmvar.h IWM_FWDMASEGSZ, 192 KiB).
pub const SECT_LEN_MAX: usize = 192 * 1024;
/// The largest file (if_iwmvar.h IWM_FWMAXSIZE, 2 MiB); the firmware store's buffer is 4 MiB.
pub const FILE_MAX: usize = 2 * 1024 * 1024;
/// Section addresses that are markers, not places: between CPU1's and CPU2's sections, and before paging.
pub const SEP_CPU1_CPU2: u32 = 0xFFFF_CCCC;
pub const SEP_PAGING: u32 = 0xAAAA_BBBB;
/// Paging (if_iwmreg.h): 4 KiB pages, at most 1 MiB.
pub const PAGE: u32 = 4096;
pub const PAGING_MAX: u32 = 1024 * 1024;
pub const MAX_SCAN_CHANNELS: u32 = 52;
pub const DEFAULT_SCAN_CHANNELS: u32 = 40;
pub const PROBE_REQ_MAX: u32 = 512;

// record types (if_iwmreg.h IWM_UCODE_TLV_*)
pub const T_PROBE_MAX_LEN: u32 = 6;
pub const T_PAN: u32 = 7;
pub const T_FLAGS: u32 = 18;
pub const T_SEC_RT: u32 = 19;
pub const T_SEC_INIT: u32 = 20;
pub const T_SEC_WOWLAN: u32 = 21;
pub const T_DEF_CALIB: u32 = 22;
pub const T_PHY_SKU: u32 = 23;
pub const T_NUM_OF_CPU: u32 = 27;
pub const T_CSCHEME: u32 = 28;
pub const T_API_CHANGES_SET: u32 = 29;
pub const T_ENABLED_CAPABILITIES: u32 = 30;
pub const T_N_SCAN_CHANNELS: u32 = 31;
pub const T_PAGING: u32 = 32;
pub const T_SEC_RT_USNIFFER: u32 = 34;
pub const T_FW_VERSION: u32 = 36;
pub const T_CMD_VERSIONS: u32 = 48;
/// Records that carry nothing this driver uses (SDIO address, debug destinations, configurations and triggers, scan
/// capabilities, memory segments, debug addresses, hardware type, the security table address (66) and the sleep-mode
/// key address (67) that iwlwifi-7265D-29 carries, the debug set): read past, never used. 66 and 67 are named in
/// Intel's dual-licensed iwlwifi fw/file.h (SEC_TABLE_ADDR, D3_KEK_KCK_ADDR); OpenBSD's iwx driver skips both.
const SKIPPED: [u32; 12] = [35, 38, 39, 50, 51, 54, 55, 58, 66, 67, 0x100_0003, 0x100_0004];
const DEBUG_BASE: u32 = 0x100_0005;

/// The four images a file can hold (if_iwmvar.h enum iwm_ucode_type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Image {
    Regular = 0,
    Init = 1,
    Wow = 2,
    RegularUsniffer = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// shorter than the header
    Short,
    /// the first word is not 0 or the magic is wrong: not a TLV firmware file
    NotTlv,
    TooBig,
    /// a record's length runs past the end of the file (at: the record's offset)
    Truncated { at: usize },
    /// a record of the wrong length or with a value out of range
    Bad { tlv: u32, at: usize },
    /// a record type this driver does not know
    Unknown { tlv: u32, at: usize },
    /// more than 16 sections in one image, or a command-versions record twice
    TooMany { at: usize },
    /// a section larger than the chip loads in one go
    SectionTooBig { at: usize },
    /// two sections of one image write the same device memory, or one wraps past 4 GiB
    Overlap,
    /// no sections for the regular or the init image
    MissingImage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Section {
    /// where it goes in the chip's memory, or a separator value
    pub devoff: u32,
    /// where its bytes are in the file
    pub at: usize,
    pub len: usize,
}

impl Section {
    pub fn is_separator(&self) -> bool {
        self.devoff == SEP_CPU1_CPU2 || self.devoff == SEP_PAGING
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sections {
    pub list: [Section; SECT_MAX],
    pub n: usize,
    /// bytes of the image the chip pages in from host memory (0: none)
    pub paging: u32,
    /// the init flow and event triggers (DEF_CALIB), if given
    pub calib: Option<(u32, u32)>,
}

impl Sections {
    pub fn iter(&self) -> impl Iterator<Item = &Section> {
        self.list[..self.n].iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fw {
    pub name: [u8; 64],
    pub ver: u32,
    pub build: u32,
    pub images: [Sections; 4],
    pub flags: u32,
    pub phy_sku: Option<u32>,
    pub api: [u32; 4],
    pub capa: [u32; 4],
    pub n_scan_channels: u32,
    pub num_cpu: Option<u32>,
    pub probe_max_len: Option<u32>,
    pub fw_version: Option<[u32; 3]>,
    /// every record seen, by count (for `wifi diag` and the evidence)
    pub records: u32,
}

fn le32(f: &[u8], at: usize) -> Option<u32> {
    let b = f.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl Fw {
    /// The name the file gives itself, up to its first zero byte.
    pub fn name(&self) -> &[u8] {
        let n = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        &self.name[..n]
    }
    pub fn image(&self, i: Image) -> &Sections {
        &self.images[i as usize]
    }
    /// A section's bytes in `file` (the same file `parse` read).
    pub fn bytes<'a>(&self, file: &'a [u8], s: &Section) -> &'a [u8] {
        file.get(s.at..s.at + s.len).unwrap_or(&[])
    }
    pub fn has_api(&self, bit: u32) -> bool {
        bit < 128 && self.api[(bit / 32) as usize] & (1 << (bit % 32)) != 0
    }
    pub fn has_capa(&self, bit: u32) -> bool {
        bit < 128 && self.capa[(bit / 32) as usize] & (1 << (bit % 32)) != 0
    }
}

/// Read a firmware file. Everything is checked before anything is returned.
pub fn parse(file: &[u8]) -> Result<Fw, Why> {
    if file.len() < HEADER {
        return Err(Why::Short);
    }
    if file.len() > FILE_MAX {
        return Err(Why::TooBig);
    }
    if le32(file, 0) != Some(0) || le32(file, 4) != Some(MAGIC) {
        return Err(Why::NotTlv);
    }
    let mut name = [0u8; 64];
    name.copy_from_slice(&file[8..72]);
    let mut fw = Fw {
        name,
        ver: le32(file, 72).ok_or(Why::Short)?,
        build: le32(file, 76).ok_or(Why::Short)?,
        images: [Sections::default(); 4],
        flags: 0,
        phy_sku: None,
        api: [0; 4],
        capa: [0; 4],
        n_scan_channels: DEFAULT_SCAN_CHANNELS,
        num_cpu: None,
        probe_max_len: None,
        fw_version: None,
        records: 0,
    };
    let mut cmd_versions = false;
    let mut at = HEADER;
    while file.len() - at >= 8 {
        let tlv = le32(file, at).ok_or(Why::Truncated { at })?;
        let len = le32(file, at + 4).ok_or(Why::Truncated { at })? as usize;
        let body = at + 8;
        if len > file.len() - body {
            return Err(Why::Truncated { at });
        }
        let d = &file[body..body + len];
        let bad = Why::Bad { tlv, at };
        let word = |i: usize| le32(d, i * 4).ok_or(bad);
        match tlv {
            T_PROBE_MAX_LEN => {
                if len < 4 || word(0)? > PROBE_REQ_MAX {
                    return Err(bad);
                }
                fw.probe_max_len = Some(word(0)?);
            }
            T_PAN => {
                if len != 0 {
                    return Err(bad);
                }
            }
            T_FLAGS => {
                if len < 4 {
                    return Err(bad);
                }
                fw.flags = word(0)?;
            }
            T_CSCHEME => {
                // cipher schemes the chip could do in hardware: unused (encryption is ours, in software); the
                // list must at least hold its count byte
                if len < 1 {
                    return Err(bad);
                }
            }
            T_NUM_OF_CPU => {
                let n = if len == 4 { word(0)? } else { return Err(bad) };
                if n != 1 && n != 2 {
                    return Err(bad);
                }
                fw.num_cpu = Some(n);
            }
            T_SEC_RT | T_SEC_INIT | T_SEC_WOWLAN | T_SEC_RT_USNIFFER => {
                if len < 4 {
                    return Err(bad);
                }
                let img = match tlv {
                    T_SEC_RT => Image::Regular,
                    T_SEC_INIT => Image::Init,
                    T_SEC_WOWLAN => Image::Wow,
                    _ => Image::RegularUsniffer,
                };
                let s = &mut fw.images[img as usize];
                if s.n == SECT_MAX {
                    return Err(Why::TooMany { at });
                }
                let sec = Section { devoff: word(0)?, at: body + 4, len: len - 4 };
                if sec.len > SECT_LEN_MAX {
                    return Err(Why::SectionTooBig { at });
                }
                s.list[s.n] = sec;
                s.n += 1;
            }
            T_DEF_CALIB => {
                if len != 12 {
                    return Err(bad);
                }
                let ty = word(0)? as usize;
                if ty >= 4 {
                    return Err(bad);
                }
                fw.images[ty].calib = Some((word(1)?, word(2)?));
            }
            T_PHY_SKU => {
                if len != 4 {
                    return Err(bad);
                }
                fw.phy_sku = Some(word(0)?);
            }
            T_API_CHANGES_SET | T_ENABLED_CAPABILITIES => {
                if len != 8 {
                    return Err(bad);
                }
                let i = word(0)? as usize;
                if i >= 4 {
                    return Err(bad); // OpenBSD stops reading here and reports success; we refuse the file
                }
                if tlv == T_API_CHANGES_SET {
                    fw.api[i] |= word(1)?;
                } else {
                    fw.capa[i] |= word(1)?;
                }
            }
            T_N_SCAN_CHANNELS => {
                let n = if len == 4 { word(0)? } else { return Err(bad) };
                if n > MAX_SCAN_CHANNELS {
                    return Err(bad);
                }
                fw.n_scan_channels = n;
            }
            T_PAGING => {
                let n = if len == 4 { word(0)? } else { return Err(bad) };
                if n > PAGING_MAX || n % PAGE != 0 {
                    return Err(bad);
                }
                fw.images[Image::Regular as usize].paging = n;
                fw.images[Image::RegularUsniffer as usize].paging = n;
            }
            T_FW_VERSION => {
                if len != 12 {
                    return Err(bad);
                }
                fw.fw_version = Some([word(0)?, word(1)?, word(2)?]);
            }
            T_CMD_VERSIONS => {
                if cmd_versions {
                    return Err(Why::TooMany { at });
                }
                cmd_versions = true;
            }
            t if SKIPPED.contains(&t) || (DEBUG_BASE..DEBUG_BASE + 5).contains(&t) => {}
            t => return Err(Why::Unknown { tlv: t, at }),
        }
        fw.records += 1;
        // the next record starts at the next multiple of 4; a last record without its padding ends the file
        let next = body + ((len + 3) & !3);
        if next > file.len() {
            break;
        }
        at = next;
    }
    if fw.images[Image::Regular as usize].n == 0 || fw.images[Image::Init as usize].n == 0 {
        return Err(Why::MissingImage);
    }
    for s in fw.images.iter() {
        check_overlap(s)?;
    }
    Ok(fw)
}

/// No two real sections of one image may write the same chip memory, and none may run past 4 GiB.
fn check_overlap(s: &Sections) -> Result<(), Why> {
    let real = |x: &Section| !x.is_separator();
    for (i, a) in s.list[..s.n].iter().enumerate().filter(|(_, x)| real(x)) {
        let a_end = (a.devoff as u64) + a.len as u64;
        if a_end > 1u64 << 32 {
            return Err(Why::Overlap);
        }
        for b in s.list[i + 1..s.n].iter().filter(|x| real(x)) {
            let b_end = (b.devoff as u64) + b.len as u64;
            if (a.devoff as u64) < b_end && (b.devoff as u64) < a_end {
                return Err(Why::Overlap);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    use std::vec::Vec;

    /// A file built here from the layout: the header, then the records given.
    fn file(records: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&MAGIC.to_le_bytes());
        let mut name = [0u8; 64];
        name[..12].copy_from_slice(b"test 17.1.0\0");
        f.extend_from_slice(&name);
        f.extend_from_slice(&0x11_00_11_00u32.to_le_bytes());
        f.extend_from_slice(&1234u32.to_le_bytes());
        f.extend_from_slice(&[0; 8]);
        for (t, d) in records {
            f.extend_from_slice(&t.to_le_bytes());
            f.extend_from_slice(&(d.len() as u32).to_le_bytes());
            f.extend_from_slice(d);
            while f.len() % 4 != 0 {
                f.push(0);
            }
        }
        f
    }
    fn sect(off: u32, n: usize, fill: u8) -> Vec<u8> {
        let mut d = off.to_le_bytes().to_vec();
        d.extend(std::iter::repeat(fill).take(n));
        d
    }
    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }
    fn minimal() -> Vec<(u32, Vec<u8>)> {
        std::vec![(T_SEC_RT, sect(0x0, 16, 0xAA)), (T_SEC_INIT, sect(0x40_0000, 8, 0xBB))]
    }

    #[test]
    fn a_whole_file_reads_back() {
        let mut r = minimal();
        r.push((T_SEC_RT, sect(SEP_CPU1_CPU2, 0, 0)));
        r.push((T_SEC_RT, sect(0x1000, 5, 0xCC))); // 5 bytes: padding follows
        r.push((T_DEF_CALIB, words(&[1, 0x11, 0x22])));
        r.push((T_PHY_SKU, words(&[0x0011_0000 | 0x3])));
        r.push((T_API_CHANGES_SET, words(&[1, 0x8000_0001])));
        r.push((T_ENABLED_CAPABILITIES, words(&[0, 0x4])));
        r.push((T_N_SCAN_CHANNELS, words(&[52])));
        r.push((T_NUM_OF_CPU, words(&[2])));
        r.push((T_FW_VERSION, words(&[17, 2, 0])));
        r.push((T_FLAGS, words(&[0x1234])));
        r.push((T_PAN, std::vec![]));
        r.push((38, std::vec![1, 2, 3])); // a debug destination: skipped
        r.push((66, words(&[0x80_0000, 0x100]))); // the security table address (in iwlwifi-7265D-29): skipped
        r.push((67, words(&[0x80_1000, 0x40])));
        r.push((DEBUG_BASE + 4, std::vec![9; 7]));
        let f = file(&r);
        let fw = parse(&f).expect("a good file");
        assert_eq!(fw.name(), b"test 17.1.0");
        assert_eq!((fw.ver, fw.build), (0x11_00_11_00, 1234));
        let rt = fw.image(Image::Regular);
        assert_eq!(rt.n, 3);
        assert_eq!(fw.bytes(&f, &rt.list[0]), &[0xAA; 16]);
        assert!(rt.list[1].is_separator());
        assert_eq!(fw.bytes(&f, &rt.list[2]), &[0xCC; 5]);
        assert_eq!(fw.image(Image::Init).list[0].devoff, 0x40_0000);
        assert_eq!(fw.image(Image::Init).calib, Some((0x11, 0x22)));
        assert_eq!(fw.phy_sku, Some(0x0011_0003));
        assert!(fw.has_api(32) && fw.has_api(63) && !fw.has_api(33));
        assert!(fw.has_capa(2) && !fw.has_capa(3));
        assert_eq!((fw.n_scan_channels, fw.num_cpu, fw.fw_version), (52, Some(2), Some([17, 2, 0])));
        assert_eq!(fw.flags, 0x1234);
        assert_eq!(fw.records, 17);
    }

    #[test]
    fn the_last_record_may_lack_its_padding() {
        let mut f = file(&minimal());
        f.extend_from_slice(&T_PHY_SKU.to_le_bytes());
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&7u32.to_le_bytes());
        // a record with an odd length and no padding at the very end
        f.extend_from_slice(&38u32.to_le_bytes());
        f.extend_from_slice(&3u32.to_le_bytes());
        f.extend_from_slice(&[1, 2, 3]);
        let fw = parse(&f).unwrap();
        assert_eq!(fw.phy_sku, Some(7));
    }

    #[test]
    fn not_a_firmware_file() {
        assert_eq!(parse(&[0; 40]), Err(Why::Short));
        let mut f = file(&minimal());
        f[0] = 1;
        assert_eq!(parse(&f), Err(Why::NotTlv));
        let mut f = file(&minimal());
        f[4] ^= 0x20;
        assert_eq!(parse(&f), Err(Why::NotTlv));
        let big = std::vec![0u8; FILE_MAX + 1];
        assert_eq!(parse(&big), Err(Why::TooBig));
    }

    #[test]
    fn every_truncation_is_refused() {
        let mut r = minimal();
        r.push((T_PHY_SKU, words(&[1])));
        let f = file(&r);
        assert!(parse(&f).is_ok());
        // cut anywhere inside the records: refused, or (cut exactly between records, or inside the last record's
        // padding) read as a shorter file, never a panic or a section past the end
        for cut in HEADER..f.len() {
            match parse(&f[..cut]) {
                Ok(fw) => {
                    for s in fw.images.iter().flat_map(|i| i.iter()) {
                        assert!(s.at + s.len <= cut);
                    }
                }
                Err(_) => {}
            }
        }
        // a length that runs past the end
        let mut f = file(&minimal());
        let at = f.len();
        f.extend_from_slice(&T_PHY_SKU.to_le_bytes());
        f.extend_from_slice(&100u32.to_le_bytes());
        f.extend_from_slice(&[0; 8]);
        assert_eq!(parse(&f), Err(Why::Truncated { at }));
        // a length near 4 GiB does not wrap
        let mut f = file(&minimal());
        f.extend_from_slice(&T_SEC_RT.to_le_bytes());
        f.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(parse(&f), Err(Why::Truncated { .. })));
    }

    #[test]
    fn wrong_lengths_and_values_are_refused() {
        let cases: [(u32, Vec<u8>); 13] = [
            (T_PHY_SKU, words(&[1, 2])),
            (T_DEF_CALIB, words(&[1, 2])),
            (T_DEF_CALIB, words(&[4, 0, 0])),
            (T_API_CHANGES_SET, words(&[4, 1])),
            (T_ENABLED_CAPABILITIES, words(&[9, 1])),
            (T_ENABLED_CAPABILITIES, words(&[0])),
            (T_N_SCAN_CHANNELS, words(&[53])),
            (T_NUM_OF_CPU, words(&[3])),
            (T_PAGING, words(&[4097])),
            (T_PAGING, words(&[PAGING_MAX + PAGE])),
            (T_PAN, std::vec![1]),
            (T_PROBE_MAX_LEN, words(&[513])),
            (T_SEC_RT, std::vec![1, 2]),
        ];
        for (t, d) in cases {
            let mut r = minimal();
            r.push((t, d.clone()));
            assert!(matches!(parse(&file(&r)), Err(Why::Bad { tlv, .. }) if tlv == t), "type {t} with {d:?} was taken");
        }
    }

    #[test]
    fn unknown_records_are_refused() {
        for t in [1u32, 5, 8, 17, 24, 26, 33, 40, 99, 0x100_000A] {
            let mut r = minimal();
            r.push((t, std::vec![0; 4]));
            assert!(matches!(parse(&file(&r)), Err(Why::Unknown { tlv, .. }) if tlv == t), "type {t} was taken");
        }
    }

    #[test]
    fn sections_are_bounded() {
        // seventeen sections in one image
        let mut r = minimal();
        for i in 0..16u32 {
            r.push((T_SEC_RT, sect(0x10_0000 + i * 0x100, 4, 1)));
        }
        assert!(matches!(parse(&file(&r)), Err(Why::TooMany { .. })));
        // one section larger than the chip loads in one go
        let mut r = minimal();
        r.push((T_SEC_RT, sect(0x80_0000, SECT_LEN_MAX + 1, 0)));
        assert!(matches!(parse(&file(&r)), Err(Why::SectionTooBig { .. })));
        // two sections of one image on the same chip memory
        let mut r = minimal();
        r.push((T_SEC_RT, sect(0x8, 16, 0)));
        assert_eq!(parse(&file(&r)), Err(Why::Overlap));
        // the same addresses in two different images are fine
        let mut r = minimal();
        r.push((T_SEC_INIT, sect(0x0, 16, 0)));
        assert!(parse(&file(&r)).is_ok());
        // a section that runs past 4 GiB
        let mut r = minimal();
        r.push((T_SEC_RT, sect(0xFFFF_FFF0, 32, 0)));
        assert_eq!(parse(&file(&r)), Err(Why::Overlap));
        // separators never count as overlapping
        let mut r = minimal();
        r.push((T_SEC_RT, sect(SEP_PAGING, 0, 0)));
        r.push((T_SEC_RT, sect(SEP_PAGING, 0, 0)));
        assert!(parse(&file(&r)).is_ok());
        // a file needs a regular and an init image
        assert_eq!(parse(&file(&[(T_SEC_RT, sect(0, 4, 0))])), Err(Why::MissingImage));
        assert_eq!(parse(&file(&[(T_SEC_INIT, sect(0, 4, 0))])), Err(Why::MissingImage));
        // command versions only once
        let mut r = minimal();
        r.push((T_CMD_VERSIONS, std::vec![0; 4]));
        r.push((T_CMD_VERSIONS, std::vec![0; 4]));
        assert!(matches!(parse(&file(&r)), Err(Why::TooMany { .. })));
    }
}
