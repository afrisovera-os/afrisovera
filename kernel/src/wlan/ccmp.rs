//! CCMP (W3 of the Wi-Fi plan): 802.11 frames encrypted and checked with AES-CCM, written from IEEE 802.11-2020
//! 12.5.3. Encryption is done here in software on every chip (plan section 3, architect's Q5).
//!
//! What this file decides, all from the standard:
//! - the AAD (12.5.3.3.3): frame control with the data subtype bits 4-6, Retry, Power Management and More Data
//!   masked to 0, Protected set, and Order masked in QoS data frames (where it means an HT Control field follows,
//!   which is not in the AAD); addresses 1-3; sequence control with the sequence number masked (fragment number
//!   kept); address 4 if present; the QoS control with bits 4-15 masked.
//! - the nonce (12.5.3.3.4): flags (the TID of a QoS frame, bit 4 for a management frame), address 2, the PN
//!   most significant byte first.
//! - the 8-byte CCMP header (12.5.3.2): PN0 PN1 0 (ExtIV | key id << 6) PN2 PN3 PN4 PN5.
//! - replay (12.5.3.4.4): a received PN must be strictly greater than the last one accepted for that TID; the
//!   counter moves only after the MIC checked.
//! Checked against the published CCMP MPDUs of P802.11i Annex I.7.4 (crypto_kat.rs CCMP_MPDU).
#![allow(dead_code)]

use super::crypto::{ccm8_decrypt, ccm8_encrypt};
use super::frame::{header_len, Why, TYPE_DATA, TYPE_MGMT};

pub const CCMP_HDR: usize = 8;
pub const MIC_LEN: usize = 8;
/// The largest PN (48 bits). A transmitter never wraps: at the end the keys must be renewed.
pub const PN_MAX: u64 = (1 << 48) - 1;
const EXT_IV: u8 = 0x20;

/// Build the AAD into `aad` and the nonce, from the MAC header `h` (exactly the header, `hdr_len` bytes).
fn aad_nonce(h: &[u8], pn: u64, aad: &mut [u8; 32]) -> Result<(usize, [u8; 13]), Why> {
    let fc = u16::from_le_bytes([*h.first().ok_or(Why::Short)?, *h.get(1).ok_or(Why::Short)?]);
    let hl = header_len(fc)?;
    if h.len() != hl {
        return Err(Why::BadLength);
    }
    let ftype = ((fc >> 2) & 3) as u8;
    let subtype = ((fc >> 4) & 0xF) as u8;
    let qos = ftype == TYPE_DATA && subtype & 0x8 != 0;
    let four = ftype == TYPE_DATA && fc & 0x0300 == 0x0300;
    let mut fc0 = h[0];
    let mut fc1 = h[1];
    if ftype == TYPE_DATA {
        fc0 &= !0x70; // subtype bits 4, 5, 6
    }
    fc1 &= !(0x08 | 0x10 | 0x20); // Retry, Power Management, More Data
    fc1 |= 0x40; // Protected
    if qos {
        fc1 &= !0x80; // Order: in a QoS data frame it means HT Control, which is not covered
    }
    aad[0] = fc0;
    aad[1] = fc1;
    aad[2..20].copy_from_slice(&h[4..22]); // A1, A2, A3
    aad[20] = h[22] & 0x0F; // fragment number kept, sequence number masked
    aad[21] = 0;
    let mut n = 22;
    if four {
        aad[22..28].copy_from_slice(&h[24..30]);
        n = 28;
    }
    let mut flags = 0u8;
    if qos {
        let at = if four { 30 } else { 24 };
        let tid = h[at] & 0x0F;
        aad[n] = tid;
        aad[n + 1] = 0;
        n += 2;
        flags = tid;
    }
    if ftype == TYPE_MGMT {
        flags |= 0x10;
    }
    let mut nonce = [0u8; 13];
    nonce[0] = flags;
    nonce[1..7].copy_from_slice(&h[10..16]); // A2
    let p = pn.to_be_bytes(); // 8 bytes, the top two are zero for a 48-bit PN
    nonce[7..13].copy_from_slice(&p[2..8]);
    Ok((n, nonce))
}

fn hdr_len_of(f: &[u8]) -> Result<usize, Why> {
    let fc = u16::from_le_bytes([*f.first().ok_or(Why::Short)?, *f.get(1).ok_or(Why::Short)?]);
    let hl = header_len(fc)?;
    if f.len() < hl {
        return Err(Why::Short);
    }
    Ok(hl)
}

/// The TID slot for replay counting: 0-15 for a QoS data frame's TID, 16 for every other frame.
pub fn replay_slot(f: &[u8]) -> Result<usize, Why> {
    let hl = hdr_len_of(f)?;
    let fc = u16::from_le_bytes([f[0], f[1]]);
    let ftype = ((fc >> 2) & 3) as u8;
    if ftype == TYPE_DATA && (fc >> 4) & 0x8 != 0 {
        let at = if fc & 0x0300 == 0x0300 { 30 } else { 24 };
        let _ = hl;
        return Ok((f[at] & 0x0F) as usize);
    }
    Ok(16)
}

/// Encrypt the plaintext MPDU `plain` (header and body, no FCS) with `tk`, key id `keyid` (0-3) and packet number
/// `pn`, into `out`: the header with Protected set, the CCMP header, the encrypted body, the 8-byte MIC.
pub fn encrypt(tk: &[u8; 16], keyid: u8, pn: u64, plain: &[u8], out: &mut [u8]) -> Result<usize, Why> {
    if keyid > 3 || pn > PN_MAX {
        return Err(Why::Unsupported);
    }
    let hl = hdr_len_of(plain)?;
    let body = &plain[hl..];
    let total = hl + CCMP_HDR + body.len() + MIC_LEN;
    let o = out.get_mut(..total).ok_or(Why::NoRoom)?;
    o[..hl].copy_from_slice(&plain[..hl]);
    o[1] |= 0x40;
    let p = pn.to_le_bytes();
    o[hl..hl + CCMP_HDR].copy_from_slice(&[p[0], p[1], 0, EXT_IV | (keyid << 6), p[2], p[3], p[4], p[5]]);
    let mut aad = [0u8; 32];
    let (alen, nonce) = aad_nonce(&o[..hl], pn, &mut aad)?;
    let (head, rest) = o.split_at_mut(hl + CCMP_HDR);
    let _ = head;
    let (data, mic) = rest.split_at_mut(body.len());
    data.copy_from_slice(body);
    let tag = ccm8_encrypt(tk, &nonce, &aad[..alen], data).map_err(|_| Why::BadLength)?;
    mic.copy_from_slice(&tag);
    Ok(total)
}

/// What a decrypted frame said in its CCMP header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rx {
    /// length of the plaintext MPDU written to `out` (header with Protected cleared, then the body)
    pub len: usize,
    pub keyid: u8,
    pub pn: u64,
    pub slot: usize,
}

/// Read the CCMP header of a protected frame without decrypting: (key id, PN). For choosing the key and the replay
/// check before the work of decryption.
pub fn peek(f: &[u8]) -> Result<(u8, u64), Why> {
    let hl = hdr_len_of(f)?;
    if f[1] & 0x40 == 0 {
        return Err(Why::Unsupported); // not protected
    }
    let c = f.get(hl..hl + CCMP_HDR).ok_or(Why::Short)?;
    if c[3] & EXT_IV == 0 || c[3] & 0x1F != 0 || c[2] != 0 {
        return Err(Why::BadLength); // CCMP always has ExtIV; reserved bits are zero
    }
    let pn = u64::from_le_bytes([c[0], c[1], c[4], c[5], c[6], c[7], 0, 0]);
    Ok((c[3] >> 6, pn))
}

/// Decrypt the protected MPDU `f` (no FCS) with `tk` into `out`. On a wrong MIC nothing is released: `out` is zeroed
/// and the frame refused. The caller checks the PN against its replay counter (see [`Replay`]) before or after,
/// and moves the counter only when this returned Ok.
pub fn decrypt(tk: &[u8; 16], f: &[u8], out: &mut [u8]) -> Result<Rx, Why> {
    let hl = hdr_len_of(f)?;
    let (keyid, pn) = peek(f)?;
    if f.len() < hl + CCMP_HDR + MIC_LEN {
        return Err(Why::Short);
    }
    let body_len = f.len() - hl - CCMP_HDR - MIC_LEN;
    let total = hl + body_len;
    let o = out.get_mut(..total).ok_or(Why::NoRoom)?;
    let mut aad = [0u8; 32];
    let (alen, nonce) = aad_nonce(&f[..hl], pn, &mut aad)?;
    o[..hl].copy_from_slice(&f[..hl]);
    o[1] &= !0x40;
    let data = &mut o[hl..];
    data.copy_from_slice(&f[hl + CCMP_HDR..hl + CCMP_HDR + body_len]);
    let mut tag = [0u8; 8];
    tag.copy_from_slice(&f[f.len() - MIC_LEN..]);
    if ccm8_decrypt(tk, &nonce, &aad[..alen], data, &tag).is_err() {
        o.iter_mut().for_each(|b| *b = 0);
        return Err(Why::BadLength);
    }
    Ok(Rx { len: total, keyid, pn, slot: replay_slot(f)? })
}

/// Receive replay counters, one per TID and one for non-QoS frames (12.5.3.4.4).
#[derive(Clone, Copy, Debug)]
pub struct Replay {
    last: [Option<u64>; 17],
}

impl Replay {
    pub const fn new() -> Self {
        Replay { last: [None; 17] }
    }
    /// Start from a known counter (the RSC of a group key from the handshake): the next PN must be greater.
    pub fn starting_at(slot_value: u64) -> Self {
        Replay { last: [Some(slot_value); 17] }
    }
    /// True if `pn` would be accepted in `slot` (strictly greater than the last accepted).
    pub fn fresh(&self, slot: usize, pn: u64) -> bool {
        match self.last.get(slot) {
            Some(Some(l)) => pn > *l,
            Some(None) => true,
            None => false,
        }
    }
    /// Record `pn` after its MIC checked. Returns false (and records nothing) if it was not fresh.
    pub fn accept(&mut self, slot: usize, pn: u64) -> bool {
        if !self.fresh(slot, pn) {
            return false;
        }
        self.last[slot] = Some(pn);
        true
    }
}

impl Default for Replay {
    fn default() -> Self {
        Self::new()
    }
}

/// The transmit PN: starts at 1, goes up by one per frame, never wraps (None at the end: renew the keys).
#[derive(Clone, Copy, Debug)]
pub struct TxPn {
    next: u64,
}

impl TxPn {
    pub const fn new() -> Self {
        TxPn { next: 1 }
    }
    pub fn take(&mut self) -> Option<u64> {
        if self.next > PN_MAX {
            return None;
        }
        let p = self.next;
        self.next += 1;
        Some(p)
    }
}

impl Default for TxPn {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_mpdus_both_ways() {
        // P802.11i Annex I.7.4, through FreeBSD's tests (crypto_kat.rs); the kernel KAT runner checks the same
        for &(tk, keyid, pn, plain, enc) in super::super::crypto_kat::CCMP_MPDU {
            let tk: &[u8; 16] = tk.try_into().unwrap();
            let mut out = [0u8; 128];
            let n = encrypt(tk, keyid, pn, plain, &mut out).unwrap();
            assert_eq!(&out[..n], enc);
            let mut back = [0u8; 128];
            let r = decrypt(tk, enc, &mut back).unwrap();
            assert_eq!((r.keyid, r.pn), (keyid, pn));
            // the plaintext MPDU comes back with Protected cleared
            let mut want = [0u8; 128];
            want[..plain.len()].copy_from_slice(plain);
            want[1] &= !0x40;
            assert_eq!(&back[..r.len], &want[..plain.len()]);
        }
    }

    #[test]
    fn any_changed_bit_is_refused() {
        let &(tk, _, _, _, enc) = &super::super::crypto_kat::CCMP_MPDU[0];
        let tk: &[u8; 16] = tk.try_into().unwrap();
        let mut out = [0u8; 128];
        for i in 0..enc.len() {
            for bit in [0x01u8, 0x80] {
                let mut f = [0u8; 128];
                f[..enc.len()].copy_from_slice(enc);
                f[i] ^= bit;
                // what the standard leaves outside the AAD: Duration (bytes 2-3), the sequence number (the top 12
                // bits of bytes 22-23), Retry/PwrMgt/MoreData (bits of byte 1, not flipped here) and the key id
                // (byte 27 bits 6-7, which only choose the key). Those still decrypt; every other change is
                // refused, and a refused frame leaves no plaintext behind
                let masked = i == 2 || i == 3 || (i == 22 && bit & 0xF0 != 0) || i == 23 || (i == 27 && bit == 0x80);
                out = [0xAA; 128];
                match decrypt(tk, &f[..enc.len()], &mut out) {
                    Ok(_) => assert!(masked, "a change at byte {i} bit {bit:#x} was accepted"),
                    Err(_) => {
                        assert!(!masked, "a change outside the AAD at byte {i} was refused");
                        assert!(out.iter().all(|&b| b == 0 || b == 0xAA), "plaintext left behind");
                    }
                }
            }
        }
    }

    #[test]
    fn replay_and_pn() {
        let mut r = Replay::new();
        assert!(r.accept(3, 10));
        assert!(!r.accept(3, 10), "the same PN twice");
        assert!(!r.accept(3, 9), "an older PN");
        assert!(r.accept(4, 1), "TIDs are counted apart");
        assert!(r.accept(3, 11));
        assert!(!r.fresh(17, 1), "no slot 17");
        let g = Replay::starting_at(500);
        assert!(!g.fresh(16, 500) && g.fresh(16, 501));
        let mut t = TxPn { next: PN_MAX };
        assert_eq!(t.take(), Some(PN_MAX));
        assert_eq!(t.take(), None, "the PN never wraps");
        let mut out = [0u8; 64];
        assert_eq!(encrypt(&[0; 16], 0, PN_MAX + 1, &[0x08, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &mut out), Err(Why::Unsupported));
    }

    #[test]
    fn header_checks() {
        let mut out = [0u8; 64];
        // unprotected frame, missing ExtIV, reserved bits, too short: refused before any decryption
        let mut f = [0u8; 48];
        f[0] = 0x08;
        assert_eq!(decrypt(&[0; 16], &f, &mut out), Err(Why::Unsupported));
        f[1] = 0x40;
        assert_eq!(decrypt(&[0; 16], &f, &mut out), Err(Why::BadLength)); // ExtIV clear
        f[27] = 0x20 | 0x01;
        assert_eq!(decrypt(&[0; 16], &f, &mut out), Err(Why::BadLength)); // reserved bit
        f[27] = 0x20;
        assert_eq!(decrypt(&[0; 16], &f[..39], &mut out), Err(Why::Short));
        assert_eq!(peek(&f), Ok((0, 0)));
    }
}
