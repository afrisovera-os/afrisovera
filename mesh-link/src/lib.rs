//! AFR-0009 link layer (B1): the hop-by-hop Noise session between two neighbours.
//!
//! Noise_XX_25519_ChaChaPoly_SHA256 for a first meeting (pairing), Noise_KK for two already-paired contacts
//! (AFR-0009 section 4.1). The handshake is run by the audited `snow` crate; this module adds AFR-0009's
//! prologue (`AFRISOVERA-MESH-LINK-V1\0` + a transport byte), the record framing (section 4.2) and the short
//! authentication string tie-in (section 4.3, via mesh-crypto's validated `sas`).
//!
//! The static key is the node's X25519 identity key (section 5.1). In XX both statics are exchanged, so each
//! side learns the other's identity key (that is how pairing learns keys); in KK both are known in advance.
//!
//! Host-tested (round-trip handshakes, SAS agreement, a sealed bundle carried as a record, wrong-key reject).
//! The eventual in-kernel port replaces snow with a no_std Noise driver over the same wire contract.

use snow::{Builder, Error, HandshakeState};

// Re-exported so integration tests (and future callers) reach the validated lower layers through this crate.
pub use mesh_crypto;
pub use mesh_hpke;

/// AFR-0009 section 3 link prologue context.
pub const CTX_LINK: &[u8] = b"AFRISOVERA-MESH-LINK-V1\0";

/// Transport bytes appended to the prologue (AFR-0009 section 4.1).
pub mod transport {
    pub const BLE_L2CAP: u8 = 0x01;
    pub const WIFI_DIRECT: u8 = 0x02;
    pub const TCP: u8 = 0x03; // Phase 0
    pub const BRIDGE: u8 = 0x04;
}

/// Record types (AFR-0009 section 4.2).
pub mod record {
    pub const BUNDLE: u8 = 0x01;
    pub const OFFER: u8 = 0x02;
    pub const WANT: u8 = 0x03;
    pub const IDENTITY: u8 = 0x04;
    pub const PAIR_CONFIRM: u8 = 0x05;
    pub const CLOSE: u8 = 0x06;
    pub const PING: u8 = 0x07;
}

fn prologue(transport_byte: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(CTX_LINK.len() + 1);
    p.extend_from_slice(CTX_LINK);
    p.push(transport_byte);
    p
}

const XX: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
const KK: &str = "Noise_KK_25519_ChaChaPoly_SHA256";

/// XX initiator (the scanner/connector at a first meeting), authenticated by `static_priv`.
pub fn xx_initiator(static_priv: &[u8; 32], transport_byte: u8) -> Result<HandshakeState, Error> {
    Builder::new(XX.parse().expect("noise params"))
        .prologue(&prologue(transport_byte))
        .local_private_key(static_priv)
        .build_initiator()
}

/// XX responder.
pub fn xx_responder(static_priv: &[u8; 32], transport_byte: u8) -> Result<HandshakeState, Error> {
    Builder::new(XX.parse().expect("noise params"))
        .prologue(&prologue(transport_byte))
        .local_private_key(static_priv)
        .build_responder()
}

/// KK initiator (both statics known in advance), dialing the contact whose key is `remote_pub`.
pub fn kk_initiator(static_priv: &[u8; 32], remote_pub: &[u8; 32], transport_byte: u8) -> Result<HandshakeState, Error> {
    Builder::new(KK.parse().expect("noise params"))
        .prologue(&prologue(transport_byte))
        .local_private_key(static_priv)
        .remote_public_key(remote_pub)
        .build_initiator()
}

/// KK responder, expecting the contact whose key is `remote_pub`.
pub fn kk_responder(static_priv: &[u8; 32], remote_pub: &[u8; 32], transport_byte: u8) -> Result<HandshakeState, Error> {
    Builder::new(KK.parse().expect("noise params"))
        .prologue(&prologue(transport_byte))
        .local_private_key(static_priv)
        .remote_public_key(remote_pub)
        .build_responder()
}

/// The six-digit short authentication string for a finished handshake (AFR-0009 section 4.3): both sides derive
/// the same value from the Noise handshake hash. Call before `into_transport_mode`.
pub fn sas(hs: &HandshakeState) -> u32 {
    let h = hs.get_handshake_hash();
    let mut h32 = [0u8; 32];
    h32.copy_from_slice(&h[..32]);
    mesh_crypto::sas(&h32)
}

/// Frame one record (AFR-0009 section 4.2): Noise-encrypt `[type] || body`, prefixed with a u16 LE length.
pub fn frame_record(ts: &mut snow::TransportState, rtype: u8, body: &[u8]) -> Result<Vec<u8>, Error> {
    let mut pt = Vec::with_capacity(1 + body.len());
    pt.push(rtype);
    pt.extend_from_slice(body);
    let mut ct = vec![0u8; pt.len() + 16];
    let n = ts.write_message(&pt, &mut ct)?;
    let mut out = Vec::with_capacity(2 + n);
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&ct[..n]);
    Ok(out)
}

/// Parse one framed record: returns (type, body). `None` on a short/garbled frame or auth failure.
pub fn parse_record(ts: &mut snow::TransportState, frame: &[u8]) -> Option<(u8, Vec<u8>)> {
    if frame.len() < 2 {
        return None;
    }
    let len = u16::from_le_bytes([frame[0], frame[1]]) as usize;
    if frame.len() < 2 + len {
        return None;
    }
    let mut pt = vec![0u8; len];
    let n = ts.read_message(&frame[2..2 + len], &mut pt).ok()?;
    if n == 0 {
        return None;
    }
    Some((pt[0], pt[1..n].to_vec()))
}

// =====================================================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn arr32(s: &str) -> [u8; 32] {
        let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }

    const XA: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
    const XB: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
    const T: u64 = 1794225600;

    /// Run an XX or KK handshake to completion between two states, returning their transport states and the SAS.
    fn drive(mut init: HandshakeState, mut resp: HandshakeState, msgs: usize) -> (snow::TransportState, snow::TransportState, u32, u32) {
        let mut buf = [0u8; 4096];
        let mut tmp = [0u8; 4096];
        for i in 0..msgs {
            if i % 2 == 0 {
                let n = init.write_message(&[], &mut buf).unwrap();
                resp.read_message(&buf[..n], &mut tmp).unwrap();
            } else {
                let n = resp.write_message(&[], &mut buf).unwrap();
                init.read_message(&buf[..n], &mut tmp).unwrap();
            }
        }
        assert!(init.is_handshake_finished() && resp.is_handshake_finished());
        let si = sas(&init);
        let sr = sas(&resp);
        (init.into_transport_mode().unwrap(), resp.into_transport_mode().unwrap(), si, sr)
    }

    #[test]
    fn xx_pairing_sas_and_bundle_over_the_link() {
        let a = arr32(XA);
        let b = arr32(XB);
        let init = xx_initiator(&a, transport::TCP).unwrap();
        let resp = xx_responder(&b, transport::TCP).unwrap();
        let (mut it, mut rt, si, sr) = drive(init, resp, 3); // XX is three messages
        assert_eq!(si, sr, "both sides derive the same SAS");

        // Seal a bundle A->B and carry it as a BUNDLE record over the established link.
        let pa = mesh_hpke::x25519_public(&a);
        let pb = mesh_hpke::x25519_public(&b);
        let dh = mesh_hpke::x25519_dh(&a, &pb);
        let root = mesh_crypto::contact_root(&pa, &pb, &dh);
        let tag = mesh_crypto::recipient_tag(&root, &pa, &pb, mesh_crypto::tag_period(T));
        let ske = [0x11u8; 32];
        let mut bid = [0u8; 16];
        for i in 0..16 {
            bid[i] = i as u8;
        }
        let bundle = mesh_hpke::bundle_seal(0, 8, &bid, &tag, T + 86400, T, 1, b"Habari, Bob!", &ske, &a, &pb).expect("seal");

        let frame = frame_record(&mut it, record::BUNDLE, &bundle).unwrap();
        let (rtype, got) = parse_record(&mut rt, &frame).expect("record parses");
        assert_eq!(rtype, record::BUNDLE);
        let opened = mesh_hpke::bundle_open(&got, &b, &pa).expect("recipient opens the carried bundle");
        assert_eq!(opened, (1u8, T, b"Habari, Bob!".to_vec()));

        // A PING both ways keeps working on the transport.
        let f = frame_record(&mut rt, record::PING, &[]).unwrap();
        assert_eq!(parse_record(&mut it, &f).unwrap(), (record::PING, Vec::new()));
    }

    #[test]
    fn kk_contacts_handshake() {
        let a = arr32(XA);
        let b = arr32(XB);
        let pa = mesh_hpke::x25519_public(&a);
        let pb = mesh_hpke::x25519_public(&b);
        let init = kk_initiator(&a, &pb, transport::TCP).unwrap();
        let resp = kk_responder(&b, &pa, transport::TCP).unwrap();
        let (mut it, mut rt, si, sr) = drive(init, resp, 2); // KK is two messages
        assert_eq!(si, sr);
        // The link carries a record.
        let f = frame_record(&mut it, record::PING, &[]).unwrap();
        assert_eq!(parse_record(&mut rt, &f).unwrap(), (record::PING, Vec::new()));
    }

    #[test]
    fn kk_wrong_peer_static_is_rejected() {
        let a = arr32(XA);
        let b = arr32(XB);
        let pa = mesh_hpke::x25519_public(&a);
        // Initiator dials with the WRONG responder static (uses A's own key instead of B's).
        let mut init_bad = kk_initiator(&a, &pa, transport::TCP).unwrap();
        let mut resp = kk_responder(&b, &pa, transport::TCP).unwrap();
        let mut buf = [0u8; 4096];
        let mut tmp = [0u8; 4096];
        let n = init_bad.write_message(&[], &mut buf).unwrap();
        assert!(resp.read_message(&buf[..n], &mut tmp).is_err(), "KK with a wrong peer static must fail");
    }
}
