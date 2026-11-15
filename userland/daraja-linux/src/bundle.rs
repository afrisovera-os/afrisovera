//! The bundle's header (AFR-0001 7.7 as amended 15 Nov 2026, the architect's 48-byte layout), read and checked
//! before a byte of the archive is touched.
//!
//! | bytes  | holds                                                                          |
//! |--------|--------------------------------------------------------------------------------|
//! | 0..8   | magic `DARAJA02`                                                               |
//! | 8..16  | the archive's length in bytes, little-endian (the kernel, from what it copied) |
//! | 16..32 | 16 secret random bytes for AT_RANDOM                                           |
//! | 32..40 | the mapped page count, little-endian (the kernel, from the frames it mapped)   |
//! | 40..48 | reserved, 0                                                                    |
//!
//! The bridge copies the header once (here: the caller passes the first 48 bytes, already on its stack), then
//! refuses unless the magic is right, the reserved bytes are 0, and 48 + length fits in the mapped pages. Only then
//! does it make the archive slice, and every later read goes through `cpio`, bounded by that slice.

pub const HEADER: usize = 48;
pub const MAGIC: &[u8; 8] = b"DARAJA02";

/// Why a bundle was refused (each is printed by name).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// not DARAJA02: a kernel and a bridge built for different layouts
    BadMagic,
    /// bytes 40..48 are not zero
    ReservedNotZero,
    /// the page count is 0, or 48 + length does not fit in it (or overflows)
    LengthPastMapping,
}

/// What a checked header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub archive_len: usize,
    pub random: [u8; 16],
    pub pages: u64,
}

fn u64_at(h: &[u8; HEADER], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&h[at..at + 8]);
    u64::from_le_bytes(b)
}

/// Check the header copy `h`.
pub fn check(h: &[u8; HEADER]) -> Result<Header, Refused> {
    if &h[0..8] != MAGIC {
        return Err(Refused::BadMagic);
    }
    if h[40..48].iter().any(|&b| b != 0) {
        return Err(Refused::ReservedNotZero);
    }
    let len = u64_at(h, 8);
    let pages = u64_at(h, 32);
    let mapped = pages.checked_mul(4096).ok_or(Refused::LengthPastMapping)?;
    let need = len.checked_add(HEADER as u64).ok_or(Refused::LengthPastMapping)?;
    if pages == 0 || need > mapped {
        return Err(Refused::LengthPastMapping);
    }
    let archive_len = usize::try_from(len).map_err(|_| Refused::LengthPastMapping)?;
    let mut random = [0u8; 16];
    random.copy_from_slice(&h[16..32]);
    Ok(Header { archive_len, random, pages })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(len: u64, pages: u64) -> [u8; HEADER] {
        let mut h = [0u8; HEADER];
        h[0..8].copy_from_slice(MAGIC);
        h[8..16].copy_from_slice(&len.to_le_bytes());
        h[16..32].copy_from_slice(&[7u8; 16]);
        h[32..40].copy_from_slice(&pages.to_le_bytes());
        h
    }

    #[test]
    fn accepts_a_fitting_bundle() {
        let h = check(&hdr(4096 - 48, 1)).unwrap();
        assert_eq!((h.archive_len, h.pages, h.random), (4048, 1, [7u8; 16]));
        assert!(check(&hdr(0, 1)).is_ok());
    }

    #[test]
    fn refuses_by_name() {
        let mut h = hdr(10, 1);
        h[7] = b'1'; // DARAJA01, the old layout
        assert_eq!(check(&h), Err(Refused::BadMagic));
        let mut h = hdr(10, 1);
        h[45] = 1;
        assert_eq!(check(&h), Err(Refused::ReservedNotZero));
        assert_eq!(check(&hdr(4096 - 47, 1)), Err(Refused::LengthPastMapping)); // one byte past
        assert_eq!(check(&hdr(10, 0)), Err(Refused::LengthPastMapping));
        assert_eq!(check(&hdr(u64::MAX - 10, 1)), Err(Refused::LengthPastMapping)); // overflow
        assert_eq!(check(&hdr(10, u64::MAX)), Err(Refused::LengthPastMapping)); // pages * 4096 overflows
    }
}
