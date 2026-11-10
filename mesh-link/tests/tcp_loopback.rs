//! Phase 0 dry-run (host): two independent nodes complete the AFR-0009 handshake and pass a sealed message
//! over a REAL TCP socket on loopback, proving the whole wire protocol end to end before the in-kernel port.
//! No netstack, no ARM change, no cloud: plain std sockets and threads. Alice (initiator) dials Bob
//! (responder), they run Noise XX with AFR-0009's u16-LE-length framing, then Alice sends a sealed bundle as a
//! BUNDLE record and Bob opens it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use mesh_link::mesh_crypto;
use mesh_link::mesh_hpke;
use mesh_link::{frame_record, parse_record, record, transport, xx_initiator, xx_responder};

const XA: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
const XB: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
const T: u64 = 1794225600;

fn arr32(s: &str) -> [u8; 32] {
    let v: Vec<u8> = (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect();
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    a
}

// AFR-0009 section 4.2 stream framing: a u16 LE length prefix before each Noise message (handshake or record).
fn write_framed(sock: &mut TcpStream, payload: &[u8]) {
    sock.write_all(&(payload.len() as u16).to_le_bytes()).unwrap();
    sock.write_all(payload).unwrap();
}
fn read_payload(sock: &mut TcpStream) -> Vec<u8> {
    let mut l = [0u8; 2];
    sock.read_exact(&mut l).unwrap();
    let n = u16::from_le_bytes(l) as usize;
    let mut b = vec![0u8; n];
    sock.read_exact(&mut b).unwrap();
    b
}
// Read a whole mesh-link record frame (the u16 prefix + ciphertext), as parse_record expects it.
fn read_record_frame(sock: &mut TcpStream) -> Vec<u8> {
    let mut l = [0u8; 2];
    sock.read_exact(&mut l).unwrap();
    let n = u16::from_le_bytes(l) as usize;
    let mut ct = vec![0u8; n];
    sock.read_exact(&mut ct).unwrap();
    let mut frame = Vec::with_capacity(2 + n);
    frame.extend_from_slice(&l);
    frame.extend_from_slice(&ct);
    frame
}

#[test]
fn phase0_dry_run_sealed_message_over_real_tcp_loopback() {
    let a = arr32(XA);
    let b = arr32(XB);
    let pa = mesh_hpke::x25519_public(&a);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap(); // ephemeral; stands in for TCP 4401 in Phase 0
    let addr = listener.local_addr().unwrap();

    // Bob (responder) on its own thread.
    let bob = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut resp = xx_responder(&b, transport::TCP).unwrap();
        let mut tmp = [0u8; 4096];
        let mut buf = [0u8; 4096];
        // XX: read msg1, write msg2, read msg3.
        let m1 = read_payload(&mut sock);
        resp.read_message(&m1, &mut tmp).unwrap();
        let n = resp.write_message(&[], &mut buf).unwrap();
        write_framed(&mut sock, &buf[..n]);
        let m3 = read_payload(&mut sock);
        resp.read_message(&m3, &mut tmp).unwrap();
        assert!(resp.is_handshake_finished());
        let sas_bob = mesh_link::sas(&resp);
        let mut rt = resp.into_transport_mode().unwrap();
        // Receive the BUNDLE record and open it as Bob, with Alice named as the sender.
        let frame = read_record_frame(&mut sock);
        let (rtype, bundle) = parse_record(&mut rt, &frame).expect("record parses");
        assert_eq!(rtype, record::BUNDLE);
        let opened = mesh_hpke::bundle_open(&bundle, &b, &pa).expect("Bob opens the sealed bundle");
        (sas_bob, opened)
    });

    // Alice (initiator) on the main thread.
    let mut sock = TcpStream::connect(addr).unwrap();
    let mut init = xx_initiator(&a, transport::TCP).unwrap();
    let mut tmp = [0u8; 4096];
    let mut buf = [0u8; 4096];
    let n = init.write_message(&[], &mut buf).unwrap();
    write_framed(&mut sock, &buf[..n]); // msg1
    let m2 = read_payload(&mut sock);
    init.read_message(&m2, &mut tmp).unwrap();
    let n = init.write_message(&[], &mut buf).unwrap();
    write_framed(&mut sock, &buf[..n]); // msg3
    assert!(init.is_handshake_finished());
    let sas_alice = mesh_link::sas(&init);
    let mut it = init.into_transport_mode().unwrap();

    // Seal a bundle Alice -> Bob and send it as a BUNDLE record.
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
    sock.write_all(&frame).unwrap(); // frame already carries its own u16 length prefix

    let (sas_bob, opened) = bob.join().unwrap();
    assert_eq!(sas_alice, sas_bob, "both ends show the same SAS over the real socket");
    assert_eq!(opened, (1u8, T, b"Habari, Bob!".to_vec()), "Bob reads the offline sealed message");
}
