//! 802.11 data frames to Ethernet frames and back (W2), so the TCP/IP stack above sees an ordinary Ethernet card
//! (WIFI_PLAN.md section 3). A station's data frames: to the AP with ToDS set (addr1 BSSID, addr2 us, addr3 the
//! destination), from the AP with FromDS set (addr1 us, addr2 BSSID, addr3 the source). The body starts with an
//! LLC/SNAP header (RFC 1042, or 802.1H for the two bridge-tunnel types) and the EtherType. A-MSDU subframes
//! (802.11-2020 9.3.2.2.2) are split here too, because an HT station must accept them.
//! Pure functions over slices; every length checked; malformed frames refused whole.
#![allow(dead_code)]

use super::frame::{parse_header, Mac, Why, TYPE_DATA};

pub const ETH_HDR: usize = 14;
pub const SNAP_LEN: usize = 8;
const RFC1042: [u8; 6] = [0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00];
const BRIDGE_TUNNEL: [u8; 6] = [0xAA, 0xAA, 0x03, 0x00, 0x00, 0xF8];
/// EtherTypes that 802.1H says go with the bridge-tunnel header (AppleTalk ARP, IPX).
const TUNNELED: [u16; 2] = [0x80F3, 0x8137];
pub const ETHERTYPE_EAPOL: u16 = 0x888E;

/// What came out of one received data frame (after decryption, if it was protected).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rx<'a> {
    pub dst: Mac,
    pub src: Mac,
    pub ethertype: u16,
    pub payload: &'a [u8],
}

fn snap(body: &[u8]) -> Result<(u16, &[u8]), Why> {
    let h = body.get(..SNAP_LEN).ok_or(Why::Short)?;
    if h[..6] != RFC1042 && h[..6] != BRIDGE_TUNNEL {
        return Err(Why::Unsupported);
    }
    Ok((u16::from_be_bytes([h[6], h[7]]), &body[SNAP_LEN..]))
}

/// The addresses of a data frame a station receives from its AP (FromDS only). Other directions are refused.
fn from_ap(f: &[u8], own: &Mac, bssid: &Mac) -> Result<(Mac, Mac, bool, usize), Why> {
    let h = parse_header(f)?;
    if h.ftype != TYPE_DATA || !h.from_ds || h.to_ds {
        return Err(Why::Unsupported);
    }
    if h.addr2 != *bssid {
        return Err(Why::Unsupported); // not from our AP
    }
    // a frame for someone else (unicast to another station) is not ours; group frames are
    if h.addr1 != *own && h.addr1[0] & 1 == 0 {
        return Err(Why::Unsupported);
    }
    if h.subtype & 0x4 != 0 {
        return Err(Why::Unsupported); // a "no data" subtype (null function and its QoS form)
    }
    let amsdu = h.qos.is_some_and(|q| q & 0x0080 != 0);
    Ok((h.addr1, h.addr3, amsdu, h.hdr_len))
}

/// Read the plaintext data frame `f` (header included; any CCMP header and MIC already removed by the caller) and
/// hand each MSDU in it to `each`. Returns how many MSDUs were handed over. A malformed frame hands over nothing.
pub fn rx_data<'a>(f: &'a [u8], own: &Mac, bssid: &Mac, mut each: impl FnMut(Rx<'a>)) -> Result<usize, Why> {
    let (dst, src, amsdu, hl) = from_ap(f, own, bssid)?;
    let body = f.get(hl..).ok_or(Why::Short)?;
    if !amsdu {
        let (ethertype, payload) = snap(body)?;
        each(Rx { dst, src, ethertype, payload });
        return Ok(1);
    }
    // A-MSDU: check every subframe first, then hand them over, so a bad tail drops the whole frame
    let mut out: [Option<Rx<'a>>; 16] = [None; 16];
    let (mut n, mut at) = (0usize, 0usize);
    while at < body.len() {
        let sub = body.get(at..at + 14).ok_or(Why::Short)?;
        let mut d = [0u8; 6];
        let mut s = [0u8; 6];
        d.copy_from_slice(&sub[0..6]);
        s.copy_from_slice(&sub[6..12]);
        let len = u16::from_be_bytes([sub[12], sub[13]]) as usize;
        let msdu = body.get(at + 14..at + 14 + len).ok_or(Why::BadLength)?;
        let (ethertype, payload) = snap(msdu)?;
        if n == out.len() {
            return Err(Why::BadLength);
        }
        out[n] = Some(Rx { dst: d, src: s, ethertype, payload });
        n += 1;
        at += 14 + len;
        // each subframe but the last is padded to a multiple of 4 bytes
        if at < body.len() {
            at = (at + 3) & !3;
        }
    }
    if n == 0 {
        return Err(Why::Short);
    }
    for r in out.iter().take(n).flatten() {
        each(*r);
    }
    Ok(n)
}

/// Turn one received MSDU into an Ethernet frame in `out`. Returns its length.
pub fn to_ethernet(r: &Rx<'_>, out: &mut [u8]) -> Result<usize, Why> {
    let n = ETH_HDR.checked_add(r.payload.len()).ok_or(Why::NoRoom)?;
    let o = out.get_mut(..n).ok_or(Why::NoRoom)?;
    o[0..6].copy_from_slice(&r.dst);
    o[6..12].copy_from_slice(&r.src);
    o[12..14].copy_from_slice(&r.ethertype.to_be_bytes());
    o[14..].copy_from_slice(r.payload);
    Ok(n)
}

/// Build a data frame for the AP from an Ethernet frame the stack wants to send: ToDS, addr1 the BSSID, addr2 our
/// address (the Ethernet source must be us), addr3 the Ethernet destination. Non-QoS data, unprotected (the caller
/// encrypts). Returns the frame's length (header + SNAP + payload).
pub fn from_ethernet(eth: &[u8], own: &Mac, bssid: &Mac, seq: u16, out: &mut [u8]) -> Result<usize, Why> {
    let h = eth.get(..ETH_HDR).ok_or(Why::Short)?;
    if h[6..12] != own[..] {
        return Err(Why::Unsupported); // a station sends only as itself (no bridging)
    }
    let ethertype = u16::from_be_bytes([h[12], h[13]]);
    if ethertype < 0x0600 {
        return Err(Why::Unsupported); // an 802.3 length field, not an EtherType
    }
    let payload = &eth[ETH_HDR..];
    let n = (24 + SNAP_LEN).checked_add(payload.len()).ok_or(Why::NoRoom)?;
    let o = out.get_mut(..n).ok_or(Why::NoRoom)?;
    let fc: u16 = ((TYPE_DATA as u16) << 2) | 0x0100; // data, subtype 0, ToDS
    o[0..2].copy_from_slice(&fc.to_le_bytes());
    o[2..4].copy_from_slice(&[0, 0]);
    o[4..10].copy_from_slice(bssid);
    o[10..16].copy_from_slice(own);
    o[16..22].copy_from_slice(&h[0..6]);
    o[22..24].copy_from_slice(&((seq & 0x0FFF) << 4).to_le_bytes());
    o[24..30].copy_from_slice(if TUNNELED.contains(&ethertype) { &BRIDGE_TUNNEL } else { &RFC1042 });
    o[30..32].copy_from_slice(&ethertype.to_be_bytes());
    o[32..].copy_from_slice(payload);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AP: Mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    const ME: Mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    const PEER: Mac = [0x02, 0x99, 0x88, 0x77, 0x66, 0x55];

    /// Turn a frame we would send into the frame the AP would forward to us (FromDS, addresses swapped as the
    /// standard says), for round trips.
    fn as_from_ap(tx: &[u8], src: &Mac) -> [u8; 128] {
        let mut f = [0u8; 128];
        f[..tx.len()].copy_from_slice(tx);
        f[0..2].copy_from_slice(&(((TYPE_DATA as u16) << 2) | 0x0200).to_le_bytes());
        f[4..10].copy_from_slice(&ME);
        f[10..16].copy_from_slice(&AP);
        f[16..22].copy_from_slice(src);
        f
    }

    #[test]
    fn ethernet_round_trip() {
        let mut eth = [0u8; 60];
        eth[0..6].copy_from_slice(&PEER);
        eth[6..12].copy_from_slice(&ME);
        eth[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        for (i, b) in eth[14..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut tx = [0u8; 128];
        let n = from_ethernet(&eth, &ME, &AP, 9, &mut tx).unwrap();
        assert_eq!(n, 24 + 8 + 46);
        let h = parse_header(&tx[..n]).unwrap();
        assert!(h.to_ds && !h.from_ds && h.addr1 == AP && h.addr2 == ME && h.addr3 == PEER);
        assert_eq!(&h.body[..8], &[0xAA, 0xAA, 3, 0, 0, 0, 8, 0]);
        // the same bytes coming back from the AP give the same Ethernet frame, addressed to us
        let f = as_from_ap(&tx[..n], &PEER);
        let mut got = [0u8; 128];
        let mut len = 0;
        assert_eq!(rx_data(&f[..n], &ME, &AP, |r| len = to_ethernet(&r, &mut got).unwrap()), Ok(1));
        assert_eq!(&got[0..6], &ME);
        assert_eq!(&got[6..12], &PEER);
        assert_eq!(&got[12..len], &eth[12..]);
    }

    #[test]
    fn refusals() {
        let mut eth = [0u8; 20];
        eth[6..12].copy_from_slice(&PEER); // not our own source
        eth[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        let mut tx = [0u8; 64];
        assert_eq!(from_ethernet(&eth, &ME, &AP, 0, &mut tx), Err(Why::Unsupported));
        eth[6..12].copy_from_slice(&ME);
        eth[12..14].copy_from_slice(&0x0100u16.to_be_bytes()); // a length, not a type
        assert_eq!(from_ethernet(&eth, &ME, &AP, 0, &mut tx), Err(Why::Unsupported));
        eth[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
        assert_eq!(from_ethernet(&eth, &ME, &AP, 0, &mut tx[..30]), Err(Why::NoRoom));
        let n = from_ethernet(&eth, &ME, &AP, 0, &mut tx).unwrap();
        let mut f = as_from_ap(&tx[..n], &PEER);
        // from another AP, for another station, a null-data subtype, a broken SNAP: all refused
        let mut g = f;
        g[10] ^= 1;
        assert_eq!(rx_data(&g[..n], &ME, &AP, |_| {}), Err(Why::Unsupported));
        let mut g = f;
        g[4] = 0x04; // unicast, not us
        assert_eq!(rx_data(&g[..n], &ME, &AP, |_| {}), Err(Why::Unsupported));
        let mut g = f;
        g[0] |= 0x40; // subtype 4: null function
        assert_eq!(rx_data(&g[..n], &ME, &AP, |_| {}), Err(Why::Unsupported));
        f[24] = 0x42;
        assert_eq!(rx_data(&f[..n], &ME, &AP, |_| {}), Err(Why::Unsupported));
        assert_eq!(rx_data(&f[..30], &ME, &AP, |_| {}), Err(Why::Short));
    }

    #[test]
    fn amsdu_split_and_truncation() {
        // QoS data from the AP with the A-MSDU bit, two subframes: 8+2 then 8+4 bytes of MSDU, the first padded
        let mut f = [0u8; 128];
        f[0..2].copy_from_slice(&(((8u16) << 4) | ((TYPE_DATA as u16) << 2) | 0x0200).to_le_bytes());
        f[4..10].copy_from_slice(&ME);
        f[10..16].copy_from_slice(&AP);
        f[16..22].copy_from_slice(&AP);
        f[24..26].copy_from_slice(&0x0080u16.to_le_bytes());
        let mut at = 26;
        for (k, plen) in [(1u8, 2usize), (2u8, 4usize)] {
            f[at..at + 6].copy_from_slice(&ME);
            f[at + 6..at + 12].copy_from_slice(&[2, 0, 0, 0, 0, k]);
            f[at + 12..at + 14].copy_from_slice(&((8 + plen) as u16).to_be_bytes());
            f[at + 14..at + 22].copy_from_slice(&[0xAA, 0xAA, 3, 0, 0, 0, 0x08, 0x06]);
            at += 22 + plen;
            if k == 1 {
                at = (at - 26 + 3) / 4 * 4 + 26;
            }
        }
        let mut seen = [0u8; 2];
        let mut i = 0;
        assert_eq!(rx_data(&f[..at], &ME, &AP, |r| { seen[i] = r.src[5]; assert_eq!(r.ethertype, 0x0806); i += 1; }), Ok(2));
        assert_eq!(seen, [1, 2]);
        // cut anywhere inside the last subframe: refused whole, nothing handed over
        for cut in at - 10..at {
            let mut got = 0;
            assert!(rx_data(&f[..cut], &ME, &AP, |_| got += 1).is_err(), "cut {cut}");
            assert_eq!(got, 0);
        }
    }
}
