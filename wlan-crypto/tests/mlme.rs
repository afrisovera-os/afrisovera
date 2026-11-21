//! The kernel's station (kernel/src/wlan/mlme.rs, unchanged by `#[path]`) end to end on the host: a pretend radio
//! that hands every transmitted frame to a test access point written HERE from IEEE 802.11-2020 (beacons, open
//! system authentication, association, the authenticator side of the 4-way handshake, data both ways). The AP's
//! handshake shares no code with the kernel's supp.rs or eapol.rs; it encrypts and decrypts data with the kernel's
//! ccmp.rs, which is checked against the published CCMP frames elsewhere (tests/stack.rs, `wifi selftest`).
//! The architect's three rules for this layer each have a case (W2 verdict, 14 Nov 2026).
#[path = "../../kernel/src/wlan/mod.rs"]
#[allow(dead_code, unused_imports)]
mod wlan;

use std::collections::VecDeque;
use wlan::frame::{BssInfo, Mac, Security};
use wlan::mlme::{Ended, Event, NoConnect, Phase, Radio, Station};
use wlan_crypto::{aes_wrap, hmac_sha1, hmac_sha1_verify, prf_sha1, psk_from_passphrase};

const AP: Mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
const TWIN: Mac = [0x02, 0x66, 0x66, 0x66, 0x66, 0x66];
const OTHER: Mac = [0x02, 0x77, 0x77, 0x77, 0x77, 0x77];
const STA: Mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
const PEER: Mac = [0x02, 0x99, 0x88, 0x77, 0x66, 0x55];
const RSN: [u8; 20] = [1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 2, 0x0C, 0];

#[derive(Default)]
struct TestRadio {
    sent: VecDeque<Vec<u8>>,
    joined: Option<Mac>,
    aid: Option<u16>,
    left: u32,
    scans: u32,
}

impl Radio for TestRadio {
    fn mac(&self) -> Mac {
        STA
    }
    fn tx(&mut self, f: &[u8]) -> bool {
        self.sent.push_back(f.to_vec());
        true
    }
    fn start_scan(&mut self) -> bool {
        self.scans += 1;
        true
    }
    fn join(&mut self, b: &BssInfo) -> bool {
        self.joined = Some(b.bssid);
        true
    }
    fn associated(&mut self, _b: &BssInfo, aid: u16) -> bool {
        self.aid = Some(aid);
        true
    }
    fn leave(&mut self) {
        self.left += 1;
        self.joined = None;
    }
}

// ---- frames, written from the standard ------------------------------------------------------------------------

fn mgmt(subtype: u8, a1: &Mac, a2: &Mac, a3: &Mac, body: &[u8]) -> Vec<u8> {
    let mut f = vec![subtype << 4, 0, 0, 0];
    f.extend_from_slice(a1);
    f.extend_from_slice(a2);
    f.extend_from_slice(a3);
    f.extend_from_slice(&[0x10, 0x00]);
    f.extend_from_slice(body);
    f
}

fn beacon(bssid: &Mac, ssid: &[u8], rsn: Option<&[u8]>) -> Vec<u8> {
    let mut b = vec![0u8; 8];
    b.extend_from_slice(&100u16.to_le_bytes());
    b.extend_from_slice(&(if rsn.is_some() { 0x0011u16 } else { 0x0001 }).to_le_bytes());
    b.extend_from_slice(&[0, ssid.len() as u8]);
    b.extend_from_slice(ssid);
    b.extend_from_slice(&[1, 4, 0x82, 0x84, 0x8B, 0x96, 3, 1, 6]);
    if let Some(r) = rsn {
        b.extend_from_slice(&[48, r.len() as u8]);
        b.extend_from_slice(r);
    }
    mgmt(8, &[0xFF; 6], bssid, bssid, &b)
}

/// A data frame from the AP (FromDS): addr1 the station, addr2 the BSSID, addr3 the source; SNAP and EtherType.
fn data_from_ap(dst: &Mac, src: &Mac, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x08, 0x02, 0, 0];
    f.extend_from_slice(dst);
    f.extend_from_slice(&AP);
    f.extend_from_slice(src);
    f.extend_from_slice(&[0x20, 0x00]);
    f.extend_from_slice(&[0xAA, 0xAA, 3, 0, 0, 0]);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

// ---- the authenticator ----------------------------------------------------------------------------------------

struct TestAp {
    pmk: [u8; 32],
    anonce: [u8; 32],
    replay: u64,
    ptk: [u8; 48],
    gtk: [u8; 16],
    pn: u64,
}

fn eapol_key(info: u16, key_len: u16, replay: u64, nonce: &[u8; 32], data: &[u8]) -> Vec<u8> {
    let mut b = vec![2u8];
    b.extend_from_slice(&info.to_be_bytes());
    b.extend_from_slice(&key_len.to_be_bytes());
    b.extend_from_slice(&replay.to_be_bytes());
    b.extend_from_slice(nonce);
    b.extend_from_slice(&[0; 48]);
    b.extend_from_slice(&(data.len() as u16).to_be_bytes());
    b.extend_from_slice(data);
    let mut f = vec![2u8, 3];
    f.extend_from_slice(&(b.len() as u16).to_be_bytes());
    f.extend_from_slice(&b);
    f
}

impl TestAp {
    fn new(pass: &[u8], ssid: &[u8]) -> Self {
        TestAp { pmk: psk_from_passphrase(pass, ssid).unwrap(), anonce: [0x3C; 32], replay: 0, ptk: [0; 48], gtk: [0x47; 16], pn: 0 }
    }
    fn msg1(&mut self) -> Vec<u8> {
        self.replay += 1;
        eapol_key(0x008A, 16, self.replay, &self.anonce, &[])
    }
    /// Take message 2: derive the PTK from its SNonce and check its MIC. True if it was right.
    fn take_msg2(&mut self, e: &[u8]) -> bool {
        let snonce: [u8; 32] = e[17..49].try_into().unwrap();
        let mut d = Vec::new();
        d.extend_from_slice(&AP.min(STA));
        d.extend_from_slice(&AP.max(STA));
        d.extend_from_slice(&self.anonce.min(snonce));
        d.extend_from_slice(&self.anonce.max(snonce));
        prf_sha1(&self.pmk, b"Pairwise key expansion", &d, &mut self.ptk).unwrap();
        let mut z = e.to_vec();
        z[81..97].fill(0);
        hmac_sha1_verify(&self.ptk[..16], &[&z[..]], &e[81..97])
    }
    fn msg3(&mut self) -> Vec<u8> {
        self.replay += 1;
        let mut kd = vec![48, RSN.len() as u8];
        kd.extend_from_slice(&RSN);
        kd.extend_from_slice(&[0xDD, 22, 0x00, 0x0F, 0xAC, 1, 1, 0]);
        kd.extend_from_slice(&self.gtk);
        kd.push(0xDD);
        while kd.len() % 8 != 0 {
            kd.push(0);
        }
        let mut w = vec![0u8; kd.len() + 8];
        aes_wrap(&self.ptk[16..32].try_into().unwrap(), &kd, &mut w).unwrap();
        let mut f = eapol_key(0x13CA, 16, self.replay, &self.anonce, &w);
        let m = hmac_sha1(&self.ptk[..16], &[&f[..]]);
        f[81..97].copy_from_slice(&m[..16]);
        f
    }
    fn tk(&self) -> [u8; 16] {
        self.ptk[32..48].try_into().unwrap()
    }
    /// Encrypt a FromDS data frame for the station with the TK (or the GTK for a group frame, key id 1).
    fn protect(&mut self, plain: &[u8], group: bool) -> Vec<u8> {
        self.pn += 1;
        let mut out = vec![0u8; plain.len() + 16];
        let (key, id) = if group { (self.gtk, 1) } else { (self.tk(), 0) };
        let n = wlan::ccmp::encrypt(&key, id, self.pn, plain, &mut out).unwrap();
        out.truncate(n);
        out
    }
}

/// The EAPOL payload of a data frame the station sent, if it is one (plaintext or decrypted with `tk`).
fn eapol_of(f: &[u8], tk: Option<[u8; 16]>) -> Option<Vec<u8>> {
    let mut buf = [0u8; 2400];
    let plain: &[u8] = if f[1] & 0x40 != 0 {
        let r = wlan::ccmp::decrypt(&tk?, f, &mut buf).ok()?;
        &buf[..r.len]
    } else {
        f
    };
    if plain.len() < 32 || plain[0] != 0x08 || plain[30..32] != [0x88, 0x8E] {
        return None;
    }
    Some(plain[32..].to_vec())
}

struct World {
    st: Station,
    r: TestRadio,
    ap: TestAp,
    now: u64,
}

impl World {
    fn new(sta_pass: &[u8], ap_pass: &[u8]) -> World {
        wlan::mlme::set_secret_rng(|b| {
            for (i, x) in b.iter_mut().enumerate() {
                *x = 0x5A ^ i as u8;
            }
            true
        });
        let mut w = World { st: Station::new(STA), r: TestRadio::default(), ap: TestAp::new(ap_pass, b"home"), now: 1000 };
        let _ = sta_pass;
        w
    }
    fn hear(&mut self, f: &[u8]) -> Option<Event> {
        self.now += 5;
        self.st.on_frame(&mut self.r, f, -50, 6, self.now)
    }
    fn next_sent(&mut self) -> Vec<u8> {
        self.r.sent.pop_front().expect("the station sent nothing")
    }
    /// Auth and association answered by the AP.
    fn join(&mut self) {
        let auth = self.next_sent();
        assert_eq!(auth[0], 0xB0, "an authentication request");
        assert!(self.hear(&mgmt(11, &STA, &AP, &AP, &[0, 0, 2, 0, 0, 0])).is_none());
        let assoc = self.next_sent();
        assert_eq!(assoc[0], 0x00, "an association request");
        self.hear(&mgmt(1, &STA, &AP, &AP, &[0x11, 0x04, 0, 0, 0x01, 0xC0, 1, 1, 0x82]));
    }
    /// The whole WPA2 join with `pass` at the station.
    fn connect_wpa2(&mut self, pass: &[u8]) -> Option<Event> {
        self.hear(&beacon(&AP, b"home", Some(&RSN)));
        let pmk = psk_from_passphrase(pass, b"home").unwrap();
        self.st.connect(&mut self.r, b"home", Some(pmk), self.now).unwrap();
        self.join();
        assert_eq!(self.st.phase, Phase::Handshake);
        let m1 = self.ap.msg1();
        self.hear(&data_from_ap(&STA, &AP, 0x888E, &m1));
        let m2 = eapol_of(&self.next_sent(), None).expect("message 2");
        if !self.ap.take_msg2(&m2) {
            return None;
        }
        let m3 = self.ap.msg3();
        let ev = self.hear(&data_from_ap(&STA, &AP, 0x888E, &m3));
        let m4 = eapol_of(&self.next_sent(), None).expect("message 4, sent before the keys are installed");
        assert_eq!(u16::from_be_bytes([m4[5], m4[6]]), 0x030A);
        ev
    }
}

#[test]
fn wpa2_join_and_data_both_ways() {
    let mut w = World::new(b"correct horse", b"correct horse");
    assert_eq!(w.connect_wpa2(b"correct horse"), Some(Event::Connected));
    assert_eq!(w.st.phase, Phase::Connected);
    assert_eq!(w.r.aid, Some(1));
    // station to AP: encrypted with the TK, and the AP reads the same Ethernet frame back
    let mut eth = vec![0u8; 14];
    eth[0..6].copy_from_slice(&PEER);
    eth[6..12].copy_from_slice(&STA);
    eth[12..14].copy_from_slice(&0x0800u16.to_be_bytes());
    eth.extend_from_slice(&[0x45; 40]);
    assert!(w.st.send_eth(&mut w.r, &eth));
    let f = w.next_sent();
    assert!(f[1] & 0x40 != 0, "data after the keys is protected");
    let mut plain = [0u8; 2400];
    let r = wlan::ccmp::decrypt(&w.ap.tk(), &f, &mut plain).expect("the AP decrypts it");
    assert_eq!(&plain[24 + 8..r.len], &eth[14..]);
    // AP to station: protected unicast and group frames reach the stack as Ethernet
    let d = data_from_ap(&STA, &PEER, 0x0800, &[0x11; 30]);
    let p = w.ap.protect(&d, false);
    w.hear(&p);
    let g = data_from_ap(&[0xFF; 6], &PEER, 0x0806, &[0x22; 28]);
    let pg = w.ap.protect(&g, true);
    w.hear(&pg);
    let mut got = [0u8; 1600];
    let n = w.st.recv_eth(&mut got).unwrap();
    assert_eq!((&got[0..6], &got[6..12], &got[14..n]), (&STA[..], &PEER[..], &[0x11u8; 30][..]));
    let n = w.st.recv_eth(&mut got).unwrap();
    assert_eq!(&got[14..n], &[0x22u8; 28][..]);
    assert!(w.st.recv_eth(&mut got).is_none());
    // the same protected frame again: a replay, dropped
    w.hear(&p);
    assert!(w.st.recv_eth(&mut got).is_none());
    assert_eq!(w.st.counters.rx_replay, 1);
}

#[test]
fn rule1_eapol_only_to_the_supplicant() {
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    // an EAPOL frame after the keys (protected): it goes to the supplicant (refused there, an old counter), never
    // to the stack
    let old = w.ap.msg1();
    let d = data_from_ap(&STA, &AP, 0x888E, &old);
    let p = w.ap.protect(&d, false);
    w.hear(&p);
    let mut got = [0u8; 1600];
    assert!(w.st.recv_eth(&mut got).is_none(), "EAPOL reached the stack");
    // the station never handed message 1..4 to the stack either
    assert_eq!(w.st.counters.rx_queue_full, 0);
}

#[test]
fn rule2_no_plaintext_data() {
    let mut w = World::new(b"correct horse", b"correct horse");
    w.hear(&beacon(&AP, b"home", Some(&RSN)));
    let pmk = psk_from_passphrase(b"correct horse", b"home").unwrap();
    w.st.connect(&mut w.r, b"home", Some(pmk), w.now).unwrap();
    w.join();
    // during the handshake: plain data is dropped
    w.hear(&data_from_ap(&STA, &PEER, 0x0800, &[1; 20]));
    let mut got = [0u8; 1600];
    assert!(w.st.recv_eth(&mut got).is_none());
    assert_eq!(w.st.counters.rx_plain_dropped, 1);
    let m1 = w.ap.msg1();
    w.hear(&data_from_ap(&STA, &AP, 0x888E, &m1));
    let m2 = eapol_of(&w.next_sent(), None).unwrap();
    assert!(w.ap.take_msg2(&m2));
    let m3 = w.ap.msg3();
    assert_eq!(w.hear(&data_from_ap(&STA, &AP, 0x888E, &m3)), Some(Event::Connected));
    w.next_sent();
    // after the keys: plain data is dropped and counted
    w.hear(&data_from_ap(&STA, &PEER, 0x0800, &[1; 20]));
    assert!(w.st.recv_eth(&mut got).is_none());
    assert_eq!(w.st.counters.rx_plain_dropped, 2);
    // but a retransmitted message 3 may come unprotected: answered (message 4 again, now protected)
    let m3b = w.ap.msg3();
    w.hear(&data_from_ap(&STA, &AP, 0x888E, &m3b));
    let tk = w.ap.tk();
    let m4 = eapol_of(&w.next_sent(), Some(tk)).expect("message 4 again");
    assert_eq!(u16::from_be_bytes([m4[5], m4[6]]), 0x030A);
}

#[test]
fn rule3_no_downgrade_by_a_twin() {
    let mut w = World::new(b"correct horse", b"correct horse");
    // an open network with the same name, heard louder, and a WPA2 one
    w.st.on_frame(&mut w.r, &beacon(&TWIN, b"home", None), -30, 6, 10);
    w.st.on_frame(&mut w.r, &beacon(&AP, b"home", Some(&RSN)), -70, 6, 10);
    assert_eq!(w.st.connect(&mut w.r, b"home", None, 20), Err(NoConnect::NeedsPassphrase));
    let pmk = psk_from_passphrase(b"correct horse", b"home").unwrap();
    w.st.connect(&mut w.r, b"home", Some(pmk), 20).unwrap();
    assert_eq!(w.r.joined, Some(AP), "the WPA2 network, not the louder open twin");
    assert_eq!(w.st.weaker_ignored, 1);
    assert_eq!(w.st.connect(&mut w.r, b"nobody", None, 30), Err(NoConnect::NotFound));
}

#[test]
fn wrong_passphrase_ends_by_timeout() {
    let mut w = World::new(b"wrong horse", b"correct horse");
    assert_eq!(w.connect_wpa2(b"wrong horse"), None, "the AP refused message 2");
    let t = w.now + wlan::supp::TIMEOUT_MS + 10;
    assert_eq!(w.st.poll(&mut w.r, t), Some(Event::Ended(Ended::Handshake(wlan::supp::Fail::Timeout))));
    assert_eq!(w.st.phase, Phase::Idle);
    assert_eq!(w.r.left, 1);
}

#[test]
fn deauth_beacon_loss_and_strangers() {
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    // a deauthentication from another BSSID is not ours
    w.hear(&mgmt(12, &STA, &OTHER, &OTHER, &[3, 0]));
    assert_eq!(w.st.phase, Phase::Connected);
    assert!(w.st.counters.rx_not_ours >= 1);
    // beacons keep the connection; their absence ends it
    w.hear(&beacon(&AP, b"home", Some(&RSN)));
    assert!(w.st.poll(&mut w.r, w.now + 1000).is_none());
    assert_eq!(w.st.poll(&mut w.r, w.now + wlan::mlme::BEACON_LOSS_MS + 1), Some(Event::Ended(Ended::BeaconLoss)));
    // connect again, then the AP deauthenticates us
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    assert_eq!(w.hear(&mgmt(12, &STA, &AP, &AP, &[2, 0])), Some(Event::Ended(Ended::Deauth(2))));
    let mut eth = [0u8; 60];
    eth[6..12].copy_from_slice(&STA);
    eth[12..14].copy_from_slice(&[8, 0]);
    assert!(!w.st.send_eth(&mut w.r, &eth), "nothing is sent after the connection ended");
}

#[test]
fn open_network() {
    let mut w = World::new(b"", b"x-unused-x");
    w.hear(&beacon(&AP, b"cafe", None));
    let pmk = psk_from_passphrase(b"correct horse", b"cafe").unwrap();
    assert_eq!(w.st.connect(&mut w.r, b"cafe", Some(pmk), w.now), Err(NoConnect::NotProtected));
    w.st.connect(&mut w.r, b"cafe", None, w.now).unwrap();
    let auth = w.next_sent();
    assert_eq!(auth[0], 0xB0);
    w.hear(&mgmt(11, &STA, &AP, &AP, &[0, 0, 2, 0, 0, 0]));
    w.next_sent();
    assert_eq!(w.hear(&mgmt(1, &STA, &AP, &AP, &[0x01, 0x04, 0, 0, 0x02, 0xC0])), Some(Event::Connected));
    w.hear(&data_from_ap(&STA, &PEER, 0x0800, &[7; 20]));
    let mut got = [0u8; 1600];
    assert!(w.st.recv_eth(&mut got).is_some(), "plain data on an open network");
    let mut eth = vec![0u8; 14];
    eth[6..12].copy_from_slice(&STA);
    eth[12..14].copy_from_slice(&[8, 0]);
    eth.extend_from_slice(&[1; 20]);
    assert!(w.st.send_eth(&mut w.r, &eth));
    assert_eq!(w.next_sent()[1] & 0x40, 0, "unprotected on an open network");
}

#[test]
fn refused_auth_and_no_answer() {
    let mut w = World::new(b"", b"x-unused-x");
    w.hear(&beacon(&AP, b"cafe", None));
    w.st.connect(&mut w.r, b"cafe", None, w.now).unwrap();
    w.next_sent();
    assert_eq!(w.hear(&mgmt(11, &STA, &AP, &AP, &[0, 0, 2, 0, 17, 0])), Some(Event::Ended(Ended::AuthRefused(17))));
    // nobody answers: three tries, then the end
    let mut w = World::new(b"", b"x-unused-x");
    w.hear(&beacon(&AP, b"cafe", None));
    w.st.connect(&mut w.r, b"cafe", None, 0).unwrap();
    let mut t = 0;
    let mut ended = None;
    for _ in 0..10 {
        t += wlan::mlme::AUTH_TIMEOUT_MS + 1;
        if let Some(e) = w.st.poll(&mut w.r, t) {
            ended = Some(e);
            break;
        }
    }
    assert_eq!(ended, Some(Event::Ended(Ended::NoAnswer)));
    assert_eq!(w.r.sent.len(), 3, "three authentication requests");
}

#[test]
fn scan_fills_the_table_and_unsupported_security_is_said() {
    let mut w = World::new(b"", b"x-unused-x");
    assert!(w.st.scan(&mut w.r, 0));
    assert_eq!(w.r.scans, 1);
    let mut wpa3 = RSN;
    wpa3[17] = 8;
    w.st.on_frame(&mut w.r, &beacon(&AP, b"new", Some(&wpa3)), -40, 1, 0);
    w.st.on_frame(&mut w.r, &beacon(&OTHER, b"cafe", None), -60, 11, 0);
    assert_eq!(w.st.table.len(), 2);
    let pmk = psk_from_passphrase(b"correct horse", b"new").unwrap();
    assert_eq!(w.st.connect(&mut w.r, b"new", Some(pmk), 1), Err(NoConnect::Unsupported(Security::Wpa3Only)));
}

#[test]
fn keys_and_plaintext_wiped_after_disconnect_failure_and_refusal() {
    // the architect's rule (14 Nov 2026): no key and no received plaintext left after a disconnect, an error or a
    // refused connect
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    assert!(w.st.holds_secrets(), "connected: keys in");
    // a frame for the stack, left unread, then the owner disconnects
    let d = data_from_ap(&STA, &PEER, 0x0800, &[0x33; 30]);
    let p = w.ap.protect(&d, false);
    w.hear(&p);
    assert!(w.st.rx_ready());
    assert_eq!(w.st.disconnect(&mut w.r), Some(Event::Ended(Ended::ByUser)));
    assert!(!w.st.holds_secrets(), "keys or plaintext left after disconnect");
    // the network sends us away
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    w.hear(&mgmt(12, &STA, &AP, &AP, &[2, 0]));
    assert!(!w.st.holds_secrets(), "keys left after a deauthentication");
    // the network goes quiet
    let mut w = World::new(b"correct horse", b"correct horse");
    w.connect_wpa2(b"correct horse");
    let t = w.now + wlan::mlme::BEACON_LOSS_MS + 1;
    assert_eq!(w.st.poll(&mut w.r, t), Some(Event::Ended(Ended::BeaconLoss)));
    assert!(!w.st.holds_secrets(), "keys left after beacon loss");
    // a wrong passphrase: the handshake times out
    let mut w = World::new(b"wrong horse", b"correct horse");
    w.connect_wpa2(b"wrong horse");
    let t = w.now + wlan::supp::TIMEOUT_MS + 10;
    w.st.poll(&mut w.r, t);
    assert!(!w.st.holds_secrets(), "keys left after a failed handshake");
    // a refused connect (a key given for an open network) keeps nothing
    let mut w = World::new(b"", b"x-unused-x");
    w.hear(&beacon(&AP, b"cafe", None));
    let pmk = psk_from_passphrase(b"correct horse", b"cafe").unwrap();
    assert_eq!(w.st.connect(&mut w.r, b"cafe", Some(pmk), w.now), Err(NoConnect::NotProtected));
    assert!(!w.st.holds_secrets(), "a refused connect kept the key");
}
