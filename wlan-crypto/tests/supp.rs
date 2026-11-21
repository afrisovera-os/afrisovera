//! The kernel's WPA2 supplicant (kernel/src/wlan/supp.rs, unchanged by `#[path]`) against a test access point
//! written HERE, separately, from IEEE 802.11-2020 12.7.2 and 12.7.6 (EAPOL-Key layout, the PTK, the MIC, the key
//! data with the GTK KDE and its padding, AES key wrap). The AP shares no code with the kernel's eapol.rs or
//! supp.rs: only the primitives of wlan-crypto, which are checked against the published vectors in tests/kat.rs.
//! Every refusal the architect asked for has its case: wrong MIC, old replay counter, changed RSN element,
//! retransmitted message 3 (KRACK), same group key again, TKIP-sized keys, unexpected descriptor version, key data
//! longer than the frame, timeouts, no random source.
#[path = "../../kernel/src/wlan/mod.rs"]
#[allow(dead_code, unused_imports)]
mod wlan;

use wlan::supp::{Fail, Outcome, Refused, State, Supplicant};
use wlan_crypto::{aes_wrap, hmac_sha1, hmac_sha1_verify, prf_sha1, psk_from_passphrase};

const AA: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
const SPA: [u8; 6] = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
/// The AP's RSN element data: version 1, CCMP group, CCMP pairwise, PSK.
const AP_RSN: [u8; 20] = [1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 2, 0x0C, 0];
/// The station's whole element as sent in its association request.
const STA_RSN_IE: [u8; 22] = [48, 20, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 2, 0, 0];

// key information bits, written out again from figure 12-33
const V2: u16 = 2;
const PAIR: u16 = 1 << 3;
const INSTALL: u16 = 1 << 6;
const ACK: u16 = 1 << 7;
const MICB: u16 = 1 << 8;
const SECURE: u16 = 1 << 9;
const ENC: u16 = 1 << 12;
const MIC_OFF: usize = 81;

struct Ap {
    pmk: [u8; 32],
    anonce: [u8; 32],
    replay: u64,
    ptk: [u8; 48],
    gtk: [u8; 16],
    gtk_id: u8,
    rsn: Vec<u8>,
}

fn ptk(pmk: &[u8; 32], anonce: &[u8; 32], snonce: &[u8; 32]) -> [u8; 48] {
    let mut d = Vec::new();
    d.extend_from_slice(AA.min(SPA).as_slice());
    d.extend_from_slice(AA.max(SPA).as_slice());
    d.extend_from_slice((*anonce).min(*snonce).as_slice());
    d.extend_from_slice((*anonce).max(*snonce).as_slice());
    let mut p = [0u8; 48];
    prf_sha1(pmk, b"Pairwise key expansion", &d, &mut p).unwrap();
    p
}

/// An EAPOL-Key frame, field by field (EAPOL version 2).
fn key_frame(info: u16, key_len: u16, replay: u64, nonce: &[u8; 32], rsc: u64, data: &[u8]) -> Vec<u8> {
    let mut b = vec![2u8];
    b.extend_from_slice(&info.to_be_bytes());
    b.extend_from_slice(&key_len.to_be_bytes());
    b.extend_from_slice(&replay.to_be_bytes());
    b.extend_from_slice(nonce);
    b.extend_from_slice(&[0; 16]); // key IV
    b.extend_from_slice(&rsc.to_le_bytes());
    b.extend_from_slice(&[0; 8]); // reserved
    b.extend_from_slice(&[0; 16]); // MIC
    b.extend_from_slice(&(data.len() as u16).to_be_bytes());
    b.extend_from_slice(data);
    let mut f = vec![2u8, 3];
    f.extend_from_slice(&(b.len() as u16).to_be_bytes());
    f.extend_from_slice(&b);
    f
}

fn sign(kck: &[u8], f: &mut [u8]) {
    let m = hmac_sha1(kck, &[&*f]);
    f[MIC_OFF..MIC_OFF + 16].copy_from_slice(&m[..16]);
}

fn mic_good(kck: &[u8], f: &[u8]) -> bool {
    let mut z = f.to_vec();
    z[MIC_OFF..MIC_OFF + 16].fill(0);
    hmac_sha1_verify(kck, &[&z[..]], &f[MIC_OFF..MIC_OFF + 16])
}

impl Ap {
    fn new(pass: &[u8]) -> Ap {
        let mut anonce = [0u8; 32];
        for (i, b) in anonce.iter_mut().enumerate() {
            *b = 0xA0 ^ i as u8;
        }
        Ap { pmk: psk_from_passphrase(pass, b"home").unwrap(), anonce, replay: 1, ptk: [0; 48], gtk: [0x47; 16], gtk_id: 1, rsn: AP_RSN.to_vec() }
    }
    fn msg1(&mut self) -> Vec<u8> {
        self.replay += 1;
        key_frame(V2 | PAIR | ACK, 16, self.replay, &self.anonce, 0, &[])
    }
    /// Check message 2 as an AP would: its MIC under the PTK made from its SNonce, and our RSN element in it.
    fn take_msg2(&mut self, f: &[u8]) -> bool {
        let snonce: [u8; 32] = f[17..49].try_into().unwrap();
        self.ptk = ptk(&self.pmk, &self.anonce, &snonce);
        let info = u16::from_be_bytes([f[5], f[6]]);
        let dlen = u16::from_be_bytes([f[97], f[98]]) as usize;
        info == V2 | PAIR | MICB && mic_good(&self.ptk[..16], f) && &f[99..99 + dlen] == &STA_RSN_IE[..]
    }
    /// Key data: our RSN element, the GTK KDE, padded (0xDD then zeros) to a multiple of 8, then AES key wrap.
    fn wrapped(&self, rsn: Option<&[u8]>, gtk: &[u8]) -> Vec<u8> {
        let mut kd = Vec::new();
        if let Some(r) = rsn {
            kd.push(48);
            kd.push(r.len() as u8);
            kd.extend_from_slice(r);
        }
        kd.extend_from_slice(&[0xDD, (6 + gtk.len()) as u8, 0x00, 0x0F, 0xAC, 1, self.gtk_id, 0]);
        kd.extend_from_slice(gtk);
        if kd.len() % 8 != 0 || kd.len() < 16 {
            kd.push(0xDD);
            while kd.len() % 8 != 0 || kd.len() < 16 {
                kd.push(0);
            }
        }
        let kek: [u8; 16] = self.ptk[16..32].try_into().unwrap();
        let mut w = vec![0u8; kd.len() + 8];
        aes_wrap(&kek, &kd, &mut w).unwrap();
        w
    }
    fn msg3_with(&mut self, info: u16, key_len: u16, rsn: Option<&[u8]>, gtk: &[u8]) -> Vec<u8> {
        self.replay += 1;
        let data = self.wrapped(rsn, gtk);
        let mut f = key_frame(info, key_len, self.replay, &self.anonce, 0x1234, &data);
        sign(&self.ptk[..16], &mut f);
        f
    }
    fn msg3(&mut self) -> Vec<u8> {
        let (rsn, gtk) = (self.rsn.clone(), self.gtk);
        self.msg3_with(V2 | PAIR | INSTALL | ACK | MICB | SECURE | ENC, 16, Some(&rsn), &gtk)
    }
    fn take_msg4(&self, f: &[u8]) -> bool {
        let info = u16::from_be_bytes([f[5], f[6]]);
        info == V2 | PAIR | MICB | SECURE && mic_good(&self.ptk[..16], f) && f[17..49] == [0; 32]
    }
    fn group1(&mut self, gtk: [u8; 16], id: u8) -> Vec<u8> {
        self.replay += 1;
        self.gtk = gtk;
        self.gtk_id = id;
        let data = self.wrapped(None, &gtk);
        let mut f = key_frame(V2 | ACK | MICB | SECURE | ENC, 16, self.replay, &[0; 32], 0x99, &data);
        sign(&self.ptk[..16], &mut f);
        f
    }
}

fn rng_ok() -> impl FnMut(&mut [u8]) -> bool {
    let mut c = 0u8;
    move |b: &mut [u8]| {
        for x in b.iter_mut() {
            c = c.wrapping_add(37);
            *x = c;
        }
        true
    }
}

struct Run {
    s: Supplicant,
    ap: Ap,
    out: [u8; 512],
    now: u64,
}

impl Run {
    fn new(sta_pass: &[u8], ap_pass: &[u8]) -> Run {
        let mut s = Supplicant::new();
        let pmk = psk_from_passphrase(sta_pass, b"home").unwrap();
        s.start(&SPA, &AA, &pmk, &AP_RSN, &STA_RSN_IE, 0).unwrap();
        Run { s, ap: Ap::new(ap_pass), out: [0; 512], now: 0 }
    }
    fn feed(&mut self, f: &[u8]) -> Outcome {
        self.now += 10;
        let mut r = rng_ok();
        self.s.on_eapol(f, self.now, &mut r, &mut self.out)
    }
    fn sent(&self, o: &Outcome) -> Vec<u8> {
        self.out[..o.send.expect("a reply")].to_vec()
    }
    /// Messages 1 to 4, checked on both sides.
    fn handshake(&mut self) {
        let m1 = self.ap.msg1();
        let o = self.feed(&m1);
        let m2 = self.sent(&o);
        assert!(self.ap.take_msg2(&m2), "the AP refused message 2");
        let m3 = self.ap.msg3();
        let o = self.feed(&m3);
        assert_eq!((o.install_ptk, o.install_gtk, o.fail, o.refused), (true, Some(1), None, None));
        let m4 = self.sent(&o);
        assert!(self.ap.take_msg4(&m4), "the AP refused message 4");
        assert_eq!(self.s.state, State::Done);
    }
}

#[test]
fn four_way_and_keys_agree() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    r.handshake();
    assert_eq!(r.s.tk().unwrap(), &r.ap.ptk[32..48], "both sides hold the same TK");
    assert_eq!(r.s.gtk(1).unwrap(), (&[0x47u8; 16], 0x1234), "the GTK and its RSC");
    assert_eq!(r.s.counters.refused, 0);
}

#[test]
fn retransmitted_msg3_answered_but_not_installed_again() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    r.handshake();
    let again = r.ap.msg3(); // a new replay counter, as an AP retransmitting does
    let o = r.feed(&again);
    assert!(o.send.is_some(), "message 4 sent again");
    assert_eq!((o.install_ptk, o.install_gtk), (false, None), "KRACK: nothing installed twice");
    let m4 = r.sent(&o);
    assert!(r.ap.take_msg4(&m4));
    // the very same frame again: an old counter
    let o = r.feed(&again);
    assert_eq!(o.refused, Some(Refused::Replay));
}

#[test]
fn forged_mic_changes_nothing() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    assert!(r.ap.take_msg2(&r.sent(&o)));
    let mut m3 = r.ap.msg3();
    m3[MIC_OFF] ^= 1;
    assert_eq!(r.feed(&m3).refused, Some(Refused::Mic));
    assert_eq!(r.s.state, State::WaitMsg3);
    assert!(r.s.tk().is_none());
    // a changed byte of key data with the old MIC: refused too
    let mut m3 = r.ap.msg3();
    let n = m3.len();
    m3[n - 1] ^= 1;
    assert_eq!(r.feed(&m3).refused, Some(Refused::Mic));
    // the honest one still works, because nothing moved
    let m3 = r.ap.msg3();
    assert!(r.feed(&m3).install_ptk);
    assert_eq!(r.s.counters.bad_mic, 2);
}

#[test]
fn wrong_passphrase_fails_by_timeout() {
    let mut r = Run::new(b"wrong horse", b"correct horse");
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    assert!(!r.ap.take_msg2(&r.sent(&o)), "the AP must refuse a message 2 made with another PMK");
    // the AP would not go on; if a message 3 came anyway it is refused on its MIC
    let m3 = r.ap.msg3();
    assert_eq!(r.feed(&m3).refused, Some(Refused::Mic));
    assert_eq!(r.s.tick(1000), None);
    assert_eq!(r.s.tick(10 + wlan::supp::TIMEOUT_MS + 1), Some(Fail::Timeout));
    assert_eq!(r.s.state, State::Failed);
    assert!(r.s.tk().is_none());
}

#[test]
fn rsn_downgrade_in_msg3_fails() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    assert!(r.ap.take_msg2(&r.sent(&o)));
    let mut tkip = AP_RSN;
    tkip[11] = 2; // pairwise TKIP in message 3, CCMP in the beacon
    let (gtk, info) = (r.ap.gtk, V2 | PAIR | INSTALL | ACK | MICB | SECURE | ENC);
    let m3 = r.ap.msg3_with(info, 16, Some(&tkip), &gtk);
    assert_eq!(r.feed(&m3).fail, Some(Fail::RsnMismatch));
    assert_eq!(r.s.state, State::Failed);
    assert!(r.s.tk().is_none() && r.s.gtk(1).is_none(), "nothing kept after a failure");
}

#[test]
fn only_ccmp() {
    for (key_len, gtk_len) in [(32u16, 16usize), (16, 32)] {
        let mut r = Run::new(b"correct horse", b"correct horse");
        let m1 = r.ap.msg1();
        let o = r.feed(&m1);
        assert!(r.ap.take_msg2(&r.sent(&o)));
        let gtk = vec![0x47u8; gtk_len];
        let rsn = r.ap.rsn.clone();
        let m3 = r.ap.msg3_with(V2 | PAIR | INSTALL | ACK | MICB | SECURE | ENC, key_len, Some(&rsn), &gtk);
        assert_eq!(r.feed(&m3).fail, Some(Fail::NotCcmp), "key length {key_len}, GTK {gtk_len}");
    }
}

#[test]
fn refusals_before_any_key() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    // a group message before the 4-way, a message 3 before message 1
    let g = r.ap.group1([1; 16], 2);
    assert_eq!(r.feed(&g).refused, Some(Refused::State));
    let mut m1 = r.ap.msg1();
    // key descriptor version 1 (the old HMAC-MD5/RC4 one)
    m1[6] = (m1[6] & !7) | 1;
    assert_eq!(r.feed(&m1).refused, Some(Refused::Version));
    // key data length larger than the frame
    let mut m1 = r.ap.msg1();
    m1[98] = 1;
    assert_eq!(r.feed(&m1).refused, Some(Refused::Parse));
    // message 3 without the encrypted-key-data bit, after a good message 1
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    assert!(r.ap.take_msg2(&r.sent(&o)));
    let (gtk, rsn) = (r.ap.gtk, r.ap.rsn.clone());
    let m3 = r.ap.msg3_with(V2 | PAIR | INSTALL | ACK | MICB | SECURE, 16, Some(&rsn), &gtk);
    assert_eq!(r.feed(&m3).fail, Some(Fail::KeyData));
}

#[test]
fn msg3_with_another_anonce_refused() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    assert!(r.ap.take_msg2(&r.sent(&o)));
    let mut m3 = r.ap.msg3();
    m3[20] ^= 1; // in the nonce
    assert_eq!(r.feed(&m3).refused, Some(Refused::Nonce));
}

#[test]
fn group_rekey() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    r.handshake();
    let g = r.ap.group1([0x22; 16], 2);
    let o = r.feed(&g);
    assert_eq!(o.install_gtk, Some(2));
    let reply = r.sent(&o);
    assert_eq!(u16::from_be_bytes([reply[5], reply[6]]), V2 | MICB | SECURE);
    assert!(mic_good(&r.ap.ptk[..16], &reply));
    assert_eq!(r.s.gtk(2).unwrap(), (&[0x22u8; 16], 0x99));
    // the same key again under a new counter: answered, not installed again (KRACK, group)
    let g2 = r.ap.group1([0x22; 16], 2);
    let o = r.feed(&g2);
    assert!(o.send.is_some() && o.install_gtk.is_none());
    // the old frame again: replay
    assert_eq!(r.feed(&g).refused, Some(Refused::Replay));
    // a new 4-way message 1 now is refused: no PTK rekey, the keys stay
    let m1 = r.ap.msg1();
    assert_eq!(r.feed(&m1).refused, Some(Refused::State));
    assert!(r.s.tk().is_some());
}

#[test]
fn no_random_source_fails() {
    let mut s = Supplicant::new();
    let pmk = [0u8; 32];
    s.start(&SPA, &AA, &pmk, &AP_RSN, &STA_RSN_IE, 0).unwrap();
    let mut ap = Ap::new(b"correct horse");
    let m1 = ap.msg1();
    let mut out = [0u8; 512];
    let mut none = |_: &mut [u8]| false;
    assert_eq!(s.on_eapol(&m1, 1, &mut none, &mut out).fail, Some(Fail::NoRandom));
    assert_eq!(s.state, State::Failed);
}

#[test]
fn msg1_retransmit_while_waiting_keeps_snonce() {
    let mut r = Run::new(b"correct horse", b"correct horse");
    let m1 = r.ap.msg1();
    let o = r.feed(&m1);
    let a = r.sent(&o);
    let m1b = r.ap.msg1();
    let o = r.feed(&m1b);
    let b = r.sent(&o);
    assert_eq!(a[17..49], b[17..49], "the same SNonce");
    assert!(r.ap.take_msg2(&b));
    let m3 = r.ap.msg3();
    assert!(r.feed(&m3).install_ptk);
}
