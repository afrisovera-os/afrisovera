//! DNS on a real network (Dira M1b; the architect's must-fix, ARCHITECT_REVIEWS 14 Nov "Dira M1b"): the question the
//! PC asks, and the check every received frame meets before its answer is believed. On QEMU's fixed plan the old
//! `dns_query` (id 0x1234, port 5000) is kept byte for byte; this path is used only with a route open (`route_open`).
//!
//! Someone who can send UDP to the PC could answer first with a forged address. So the id and the source port are
//! random (the DRBG, chosen by the caller), and an answer counts only if all of these hold:
//! - it comes from the name server's address and port 53, to our port;
//! - it carries our id, the answer bit, one question, and that question is ours (name, type A, class IN).
//! Anything else that looks like a DNS answer is dropped and counted.
//!
//! Pure code: no `unsafe`, no kernel calls but `crate::netparse` (which fuzz-host includes too).
#![forbid(unsafe_code)]

/// The lowest source port a query uses (the dynamic range, 49152 to 65535).
pub const PORT_LO: u16 = 49152;
/// The longest DNS message the PC builds: 12-byte header, a name of at most 255 bytes on the wire, type and class.
pub const QUERY_MAX: usize = 12 + 255 + 4;

/// The question part of a query for `name` (A, IN): labels of 1 to 63 letters, digits and hyphens, at most 253
/// letters in all. `None` for anything else.
pub fn question(name: &str, out: &mut [u8; 260]) -> Option<usize> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return None;
    }
    let mut n = 0;
    for label in name.split('.') {
        let b = label.as_bytes();
        if b.is_empty() || b.len() > 63 || !b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-') {
            return None;
        }
        out[n] = b.len() as u8;
        out[n + 1..n + 1 + b.len()].copy_from_slice(b);
        n += 1 + b.len();
    }
    out[n] = 0;
    out[n + 1..n + 5].copy_from_slice(&[0, 1, 0, 1]); // QTYPE A, QCLASS IN
    Some(n + 5)
}

/// The whole query message: header (our id, recursion desired, one question), then the question.
pub fn query(id: u16, q: &[u8], out: &mut [u8; QUERY_MAX]) -> usize {
    let q = &q[..q.len().min(QUERY_MAX - 12)];
    out[..12].copy_from_slice(&[(id >> 8) as u8, id as u8, 0x01, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    out[12..12 + q.len()].copy_from_slice(q);
    12 + q.len()
}

/// What one received frame is to a query that waits for its answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not DNS traffic for us at all (ARP, TCP, other UDP): ignored, not counted.
    Other,
    /// A DNS-looking answer that fails a check: dropped and counted.
    Dropped,
    /// Our answer: the first A record, if it has one.
    Answer(Option<[u8; 4]>),
}

/// Check one received Ethernet frame against the query: `server` and port 53 to `port`, our `id`, our question `q`.
pub fn check(fr: &[u8], server: [u8; 4], port: u16, id: u16, q: &[u8]) -> Verdict {
    let u = match crate::netparse::udp(fr) {
        Some(u) => u,
        None => return Verdict::Other,
    };
    if u.dport != port && u.sport != 53 {
        return Verdict::Other;
    }
    // the IPv4 source sits at the same place whatever the header's options (netparse::udp checked IPv4)
    let src = match fr.get(26..30) {
        Some(s) => [s[0], s[1], s[2], s[3]],
        None => return Verdict::Dropped,
    };
    if src != server || u.sport != 53 || u.dport != port {
        return Verdict::Dropped;
    }
    let m = u.payload;
    if m.len() < 12 + q.len() {
        return Verdict::Dropped;
    }
    let (rid, _) = match crate::netparse::dns_header(m) {
        Some(h) => h,
        None => return Verdict::Dropped,
    };
    let answer_bit = m[2] & 0x80 != 0;
    let qdcount = u16::from_be_bytes([m[4], m[5]]);
    let same_q = m[12..12 + q.len()].iter().zip(q).all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase());
    if rid != id || !answer_bit || qdcount != 1 || !same_q {
        return Verdict::Dropped;
    }
    Verdict::Answer(crate::netparse::dns_first_a(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_names() {
        let mut q = [0u8; 260];
        let n = question("Dira.Test", &mut q).unwrap();
        assert_eq!(&q[..n], b"\x04Dira\x04Test\x00\x00\x01\x00\x01");
        assert!(question("a..b", &mut q).is_none());
        assert!(question("a_b.c", &mut q).is_none());
        assert!(question(&"a".repeat(64), &mut q).is_none());
        assert!(question("", &mut q).is_none());
        assert_eq!(question("x.", &mut q), Some(7)); // length 1, "x", the root, type, class
    }
}
