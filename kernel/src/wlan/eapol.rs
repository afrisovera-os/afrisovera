//! EAPOL-Key frames (W3): read and write, from IEEE 802.11-2020 12.7.2 (and IEEE 802.1X-2010 for the EAPOL header).
//! Only the RSN key descriptor (type 2) with a 16-byte MIC (AKM 2, PSK) is read; anything else is refused. Every
//! length is checked against the slice, and the key data must lie inside the frame (architect's W3 condition).
//!
//! Layout after the 4-byte EAPOL header (version, type 3 = Key, body length big endian):
//!   descriptor type (1) | key information (2, BE) | key length (2, BE) | replay counter (8, BE) | nonce (32) |
//!   key IV (16) | RSC (8, little endian) | reserved (8) | MIC (16) | key data length (2, BE) | key data
#![allow(dead_code)]

use super::frame::Why;

pub const EAPOL_HDR: usize = 4;
pub const TYPE_KEY: u8 = 3;
pub const DESC_RSN: u8 = 2;
/// Offset of the MIC from the start of the EAPOL frame.
pub const MIC_AT: usize = EAPOL_HDR + 1 + 2 + 2 + 8 + 32 + 16 + 8 + 8;
pub const MIC_LEN: usize = 16;
/// The fixed part: EAPOL header to the end of the key data length.
pub const FIXED: usize = MIC_AT + MIC_LEN + 2;

// Key information bits (12.7.2, figure 12-33)
pub const KI_VERSION_MASK: u16 = 0x0007;
/// Key descriptor version 2: HMAC-SHA1-128 MIC, AES key wrap (AKM 2).
pub const KI_VERSION_2: u16 = 2;
pub const KI_PAIRWISE: u16 = 0x0008;
pub const KI_INSTALL: u16 = 0x0040;
pub const KI_ACK: u16 = 0x0080;
pub const KI_MIC: u16 = 0x0100;
pub const KI_SECURE: u16 = 0x0200;
pub const KI_ERROR: u16 = 0x0400;
pub const KI_REQUEST: u16 = 0x0800;
pub const KI_ENCRYPTED: u16 = 0x1000;

/// A parsed EAPOL-Key frame. `frame` is the frame cut to the length its header states (padding after it, which
/// some APs send, is not part of the MIC).
#[derive(Clone, Copy, Debug)]
pub struct Key<'a> {
    pub eapol_version: u8,
    pub info: u16,
    pub key_len: u16,
    pub replay: u64,
    pub nonce: [u8; 32],
    pub rsc: u64,
    pub mic: [u8; 16],
    pub data: &'a [u8],
    pub frame: &'a [u8],
}

impl<'a> Key<'a> {
    pub fn version(&self) -> u16 {
        self.info & KI_VERSION_MASK
    }
    pub fn has(&self, bit: u16) -> bool {
        self.info & bit != 0
    }
}

/// Read an EAPOL frame (from the EAPOL header on, the payload of a 0x888E data frame).
pub fn parse(f: &[u8]) -> Result<Key<'_>, Why> {
    if f.len() < FIXED {
        return Err(Why::Short);
    }
    if f[1] != TYPE_KEY {
        return Err(Why::Unsupported);
    }
    let body_len = u16::from_be_bytes([f[2], f[3]]) as usize;
    let frame = f.get(..EAPOL_HDR + body_len).ok_or(Why::BadLength)?;
    if frame.len() < FIXED {
        return Err(Why::Short);
    }
    if frame[4] != DESC_RSN {
        return Err(Why::Unsupported);
    }
    let info = u16::from_be_bytes([frame[5], frame[6]]);
    let key_len = u16::from_be_bytes([frame[7], frame[8]]);
    let mut r = [0u8; 8];
    r.copy_from_slice(&frame[9..17]);
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&frame[17..49]);
    let mut rsc = [0u8; 8];
    rsc.copy_from_slice(&frame[65..73]);
    let mut mic = [0u8; 16];
    mic.copy_from_slice(&frame[MIC_AT..MIC_AT + MIC_LEN]);
    let dlen = u16::from_be_bytes([frame[FIXED - 2], frame[FIXED - 1]]) as usize;
    let data = frame.get(FIXED..FIXED + dlen).ok_or(Why::BadLength)?;
    if FIXED + dlen != frame.len() {
        return Err(Why::BadLength); // bytes inside the stated body that belong to nothing
    }
    Ok(Key {
        eapol_version: frame[0],
        info,
        key_len,
        replay: u64::from_be_bytes(r),
        nonce,
        rsc: u64::from_le_bytes(rsc),
        mic,
        data,
        frame,
    })
}

/// Write an EAPOL-Key frame with a zero MIC into `out`; the caller computes the MIC over the returned bytes and
/// writes it at `MIC_AT`. Returns the frame's length.
pub fn build(eapol_version: u8, info: u16, key_len: u16, replay: u64, nonce: &[u8; 32], data: &[u8], out: &mut [u8]) -> Result<usize, Why> {
    if data.len() > 0xFFFF - (FIXED - EAPOL_HDR) {
        return Err(Why::BadLength);
    }
    let n = FIXED + data.len();
    let o = out.get_mut(..n).ok_or(Why::NoRoom)?;
    o.iter_mut().for_each(|b| *b = 0);
    o[0] = eapol_version;
    o[1] = TYPE_KEY;
    o[2..4].copy_from_slice(&((n - EAPOL_HDR) as u16).to_be_bytes());
    o[4] = DESC_RSN;
    o[5..7].copy_from_slice(&info.to_be_bytes());
    o[7..9].copy_from_slice(&key_len.to_be_bytes());
    o[9..17].copy_from_slice(&replay.to_be_bytes());
    o[17..49].copy_from_slice(nonce);
    o[FIXED - 2..FIXED].copy_from_slice(&(data.len() as u16).to_be_bytes());
    o[FIXED..].copy_from_slice(data);
    Ok(n)
}

// ---- key data: elements and KDEs (12.7.2, table 12-10) ---------------------------------------------------------

pub const KDE_GTK: u8 = 1;

/// One item of (decrypted) key data: an element (id, data), or a KDE (00-0F-AC type, data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item<'a> {
    Element { id: u8, data: &'a [u8] },
    Kde { kind: u8, data: &'a [u8] },
}

/// Walk key data. Padding (0xDD followed only by zeros, to the end) ends the walk. A truncated item makes the whole
/// key data malformed: `Err`, nothing returned.
pub fn items<'a>(d: &'a [u8], out: &mut [Option<Item<'a>>; 8]) -> Result<usize, Why> {
    let (mut i, mut n) = (0usize, 0usize);
    while i < d.len() {
        if d[i] == 0xDD && d[i + 1..].iter().all(|&b| b == 0) {
            break; // padding
        }
        let len = *d.get(i + 1).ok_or(Why::BadLength)? as usize;
        let body = d.get(i + 2..i + 2 + len).ok_or(Why::BadLength)?;
        let it = if d[i] == 0xDD && body.len() >= 4 && body[..3] == [0x00, 0x0F, 0xAC] {
            Item::Kde { kind: body[3], data: &body[4..] }
        } else {
            Item::Element { id: d[i], data: body }
        };
        if n == out.len() {
            return Err(Why::BadLength);
        }
        out[n] = Some(it);
        n += 1;
        i += 2 + len;
    }
    Ok(n)
}

/// The GTK KDE's data: key id (bits 0-1 of the first byte), a reserved byte, the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gtk<'a> {
    pub keyid: u8,
    pub key: &'a [u8],
}

pub fn gtk(kde: &[u8]) -> Result<Gtk<'_>, Why> {
    if kde.len() < 2 {
        return Err(Why::BadLength);
    }
    Ok(Gtk { keyid: kde[0] & 0x03, key: &kde[2..] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_then_parse() {
        let mut buf = [0u8; 200];
        let nonce = [7u8; 32];
        let n = build(2, KI_VERSION_2 | KI_PAIRWISE | KI_MIC, 0, 5, &nonce, &[48, 2, 1, 0], &mut buf).unwrap();
        assert_eq!(n, FIXED + 4);
        let k = parse(&buf[..n]).unwrap();
        assert_eq!((k.version(), k.replay, k.nonce, k.data), (2, 5, nonce, &[48u8, 2, 1, 0][..]));
        assert!(k.has(KI_PAIRWISE) && k.has(KI_MIC) && !k.has(KI_ACK));
        // padding after the stated length is not part of the frame
        let k2 = parse(&buf[..n + 20]).unwrap();
        assert_eq!(k2.frame.len(), n);
    }

    #[test]
    fn lengths_refused() {
        let mut buf = [0u8; 200];
        let n = build(2, KI_VERSION_2, 0, 1, &[0; 32], &[1, 2, 3, 4], &mut buf).unwrap();
        assert_eq!(parse(&buf[..FIXED - 1]).map(|_| ()), Err(Why::Short));
        let mut f = buf;
        f[FIXED - 1] = 5; // key data length one more than there is
        assert_eq!(parse(&f[..n]).map(|_| ()), Err(Why::BadLength));
        let mut f = buf;
        f[FIXED - 1] = 3; // one byte too few: a stray byte in the body
        assert_eq!(parse(&f[..n]).map(|_| ()), Err(Why::BadLength));
        let mut f = buf;
        f[3] = (n - 4 + 1) as u8; // body length past the end
        assert_eq!(parse(&f[..n]).map(|_| ()), Err(Why::BadLength));
        let mut f = buf;
        f[1] = 0; // EAP packet, not a key
        assert_eq!(parse(&f[..n]).map(|_| ()), Err(Why::Unsupported));
        let mut f = buf;
        f[4] = 254; // the old WPA1 descriptor
        assert_eq!(parse(&f[..n]).map(|_| ()), Err(Why::Unsupported));
    }

    #[test]
    fn key_data_items() {
        // an RSN element, a GTK KDE (key id 1), then padding
        let mut d = [0u8; 48];
        d[..4].copy_from_slice(&[48, 2, 1, 0]);
        d[4..10].copy_from_slice(&[0xDD, 22, 0x00, 0x0F, 0xAC, KDE_GTK]);
        d[10] = 0x01;
        d[12..28].copy_from_slice(&[0x5A; 16]);
        d[28] = 0xDD;
        let mut out = [None; 8];
        let n = items(&d[..40], &mut out).unwrap();
        assert_eq!(n, 2);
        assert_eq!(out[0], Some(Item::Element { id: 48, data: &[1, 0] }));
        let Some(Item::Kde { kind, data }) = out[1] else { panic!() };
        assert_eq!(kind, KDE_GTK);
        assert_eq!(gtk(data).unwrap(), Gtk { keyid: 1, key: &[0x5A; 16] });
        // a KDE whose length runs past the end
        let mut bad = d;
        bad[5] = 60;
        assert_eq!(items(&bad[..40], &mut out), Err(Why::BadLength));
        // a lone element id at the end
        assert_eq!(items(&[48], &mut out), Err(Why::BadLength));
    }
}
