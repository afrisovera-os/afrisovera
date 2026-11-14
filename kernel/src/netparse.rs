//! Readers for received network frames, on slices (docs/security-model.md, gap G1).
//!
//! Every function here takes `&[u8]` holding exactly the bytes the NIC delivered (Ethernet header first, the
//! virtio-net header already removed) and reads nothing outside it. A length field in a packet is a claim by a
//! stranger: it is checked against the slice before anything is read through it. No `unsafe`, no hardware, no
//! `crate::` imports, so `fuzz-host` compiles this file unchanged and fuzzes it.
//!
//! Before this module, virtio.rs read frames through raw pointers and took the payload length from the IP total
//! length alone: a 60-byte TCP segment claiming 65,535 bytes made the kernel read about 64 KiB past its 2 KiB
//! receive buffer, and the DNS reader walked the answer section by the same claim.

#![forbid(unsafe_code)]

/// Ethernet header bytes before the IP header.
pub const ETH: usize = 14;

fn be16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(o)?, *b.get(o.checked_add(1)?)?]))
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o.checked_add(4)?)?;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// The EtherType, or `None` for a frame shorter than an Ethernet header.
pub fn ethertype(fr: &[u8]) -> Option<u16> {
    if fr.len() < ETH {
        return None;
    }
    be16(fr, 12)
}

/// An IPv4 packet inside a frame, its claims checked: `20 <= ihl <= total`, and `total` fits in the frame
/// (Ethernet padding may make the frame LONGER than `ETH + total`, never shorter).
pub struct Ipv4 {
    pub ihl: usize,
    pub proto: u8,
    pub total: usize,
    pub src: [u8; 4],
}

pub fn ipv4(fr: &[u8]) -> Option<Ipv4> {
    if ethertype(fr)? != 0x0800 || fr.len() < ETH + 20 {
        return None;
    }
    let ihl = (fr[ETH] & 0x0f) as usize * 4;
    let total = be16(fr, ETH + 2)? as usize;
    if ihl < 20 || total < ihl || total > fr.len() - ETH {
        return None;
    }
    Some(Ipv4 { ihl, proto: fr[ETH + 9], total, src: [fr[ETH + 12], fr[ETH + 13], fr[ETH + 14], fr[ETH + 15]] })
}

/// A TCP segment. `doff` and `dlen` are offsets into the FRAME, and `doff + dlen <= frame.len()` always holds.
pub struct Seg {
    pub src: [u8; 4],
    pub sport: u16,
    pub dport: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub doff: usize,
    pub dlen: usize,
}

/// Parse a frame as IPv4/TCP, or `None` if it is not one or any length in it lies.
pub fn tcp(fr: &[u8]) -> Option<Seg> {
    let ip = ipv4(fr)?;
    if ip.proto != 6 {
        return None;
    }
    let toff = ETH + ip.ihl;
    if ip.total < ip.ihl + 20 {
        return None;
    }
    let hlen = ((fr[toff + 12] >> 4) as usize) * 4;
    if hlen < 20 || ip.ihl + hlen > ip.total {
        return None;
    }
    let doff = toff + hlen;
    let dlen = ip.total - ip.ihl - hlen;
    debug_assert!(doff + dlen <= fr.len());
    Some(Seg {
        src: ip.src,
        sport: be16(fr, toff)?,
        dport: be16(fr, toff + 2)?,
        seq: be32(fr, toff + 4)?,
        ack: be32(fr, toff + 8)?,
        flags: fr[toff + 13],
        doff,
        dlen,
    })
}

/// A UDP datagram: ports and the payload, bounded by the UDP length AND the IP total length.
pub struct Udp<'a> {
    pub sport: u16,
    pub dport: u16,
    pub payload: &'a [u8],
}

pub fn udp(fr: &[u8]) -> Option<Udp<'_>> {
    let ip = ipv4(fr)?;
    if ip.proto != 17 || ip.total < ip.ihl + 8 {
        return None;
    }
    let u = ETH + ip.ihl;
    let ulen = be16(fr, u + 4)? as usize;
    let end = ETH + ip.total;
    if ulen < 8 || u + ulen > end {
        return None;
    }
    Some(Udp { sport: be16(fr, u)?, dport: be16(fr, u + 2)?, payload: &fr[u + 8..u + ulen] })
}

/// The ICMP type of an IPv4/ICMP frame.
pub fn icmp_type(fr: &[u8]) -> Option<u8> {
    let ip = ipv4(fr)?;
    if ip.proto != 1 || ip.total <= ip.ihl {
        return None;
    }
    Some(fr[ETH + ip.ihl])
}

/// The sender hardware address of an ARP frame (Ethernet 14 + ARP offset 8).
pub fn arp_sender_mac(fr: &[u8]) -> Option<[u8; 6]> {
    if ethertype(fr)? != 0x0806 {
        return None;
    }
    let s = fr.get(ETH + 8..ETH + 14)?;
    Some([s[0], s[1], s[2], s[3], s[4], s[5]])
}

/// Advance past a DNS name (labels, or a compression pointer which ends it). `None` if it runs off `msg`.
fn dns_skip_name(msg: &[u8], mut off: usize) -> Option<usize> {
    loop {
        let b = *msg.get(off)?;
        if b == 0 {
            return Some(off + 1);
        }
        if b & 0xc0 == 0xc0 {
            return if off + 2 <= msg.len() { Some(off + 2) } else { None };
        }
        off += 1 + b as usize;
    }
}

/// The header fields of a DNS message: id and answer count.
pub fn dns_header(msg: &[u8]) -> Option<(u16, u16)> {
    if msg.len() < 12 {
        return None;
    }
    Some((be16(msg, 0)?, be16(msg, 6)?))
}

/// The FIRST A record (type 1, rdlength 4) in a DNS response `msg` (the UDP payload), or `None`.
pub fn dns_first_a(msg: &[u8]) -> Option<[u8; 4]> {
    if msg.len() < 12 {
        return None;
    }
    let qd = be16(msg, 4)? as usize;
    let an = be16(msg, 6)? as usize;
    let mut off = 12usize;
    for _ in 0..qd {
        off = dns_skip_name(msg, off)?.checked_add(4)?; // + QTYPE + QCLASS
    }
    for _ in 0..an {
        off = dns_skip_name(msg, off)?;
        let rtype = be16(msg, off)?;
        let rdlen = be16(msg, off.checked_add(8)?)? as usize;
        let rdata = off + 10;
        let r = msg.get(rdata..rdata.checked_add(rdlen)?)?;
        if rtype == 1 && rdlen == 4 {
            return Some([r[0], r[1], r[2], r[3]]);
        }
        off = rdata + rdlen;
    }
    None
}

/// What a DHCP reply carried. Each option is `Some` only if it was present and whole.
#[derive(Default)]
pub struct Dhcp {
    pub msgtype: u8,
    pub yiaddr: [u8; 4],
    pub server: Option<[u8; 4]>,
    pub mask: Option<[u8; 4]>,
    pub router: Option<[u8; 4]>,
    pub dns: Option<[u8; 4]>,
}

/// A DHCP BOOTREPLY for transaction `xid` (the UDP payload of a frame to port 68), or `None`.
pub fn dhcp(fr: &[u8], xid: u32) -> Option<Dhcp> {
    let u = udp(fr)?;
    let p = u.payload;
    if *p.get(0)? != 2 || be32(p, 4)? != xid {
        return None; // not a BOOTREPLY for us
    }
    let mut d = Dhcp { yiaddr: four(p, 16)?, ..Dhcp::default() };
    // Options begin after the 236-byte BOOTP area and the 4-byte magic cookie.
    let mut o = 240usize;
    while o < p.len() {
        let opt = p[o];
        if opt == 255 {
            break;
        }
        if opt == 0 {
            o += 1;
            continue;
        }
        let len = *p.get(o + 1)? as usize;
        let v = p.get(o + 2..o + 2 + len)?;
        match opt {
            53 if len >= 1 => d.msgtype = v[0],
            54 => d.server = four(v, 0),
            1 => d.mask = four(v, 0),
            3 => d.router = four(v, 0),
            6 => d.dns = four(v, 0),
            _ => {}
        }
        o += 2 + len;
    }
    Some(d)
}

fn four(b: &[u8], o: usize) -> Option<[u8; 4]> {
    let s = b.get(o..o.checked_add(4)?)?;
    Some([s[0], s[1], s[2], s[3]])
}

/// An Ethernet/IPv4 ARP packet: operation, sender hardware and protocol addresses, target protocol address.
pub struct Arp {
    pub op: u16,
    pub sha: [u8; 6],
    pub spa: [u8; 4],
    pub tpa: [u8; 4],
}

pub fn arp(fr: &[u8]) -> Option<Arp> {
    if ethertype(fr)? != 0x0806 || fr.len() < ETH + 28 {
        return None;
    }
    let sha = arp_sender_mac(fr)?;
    Some(Arp { op: be16(fr, ETH + 6)?, sha, spa: four(fr, ETH + 14)?, tpa: four(fr, ETH + 24)? })
}
