//! The pretend radio of the Wi-Fi test image (W4b). pc/main.rs builds this file as `air` with feature `wifi-sim`,
//! in place of pc/wifiradio.rs; nothing else in the kernel names the feature. Never in the normal PC image (suite
//! pc-wifi-sim checks that the image has none of the names below). It stands in for a Wi-Fi chip so a QEMU boot can run the whole of `wifi scan`,
//! `wifi connect`, DHCP and traffic through the kernel's real station (wlan/mlme.rs), supplicant and CCMP.
//!
//! Three networks are on the air: "AfriSim" (WPA2-Personal, CCMP), "AfriSim-open" (no security) and "AfriSim-wpa3"
//! (WPA3 only, which the station must refuse). The WPA2 one has an access point written here from IEEE 802.11-2020:
//! open system authentication, association, the authenticator side of the 4-way handshake (12.7.6) with a GTK, and
//! data both ways. Its uplink is the PC's e1000 card: the station's Ethernet frames leave by the e1000 once they
//! decrypt and pass the replay check, and the e1000's frames for the station go back over the air encrypted. The
//! access point uses the same primitives (wlan/crypto.rs) but none of the station's handshake or frame code.
//!
//! The passphrase below is a public test value of this test image, not a secret.

use crate::wlan::ccmp::{self, Replay};
use crate::wlan::crypto;
use crate::wlan::frame::{parse_header, BssInfo, Mac, TYPE_DATA, TYPE_MGMT};
use crate::wlan::mlme::Radio;

pub const SSID_WPA2: &[u8] = b"AfriSim";
pub const SSID_OPEN: &[u8] = b"AfriSim-open";
pub const SSID_WPA3: &[u8] = b"AfriSim-wpa3";
/// The WPA2 network's passphrase (a test value, printed by suite pc-wifi-sim's notes, never by the kernel).
pub const PASS: &[u8] = b"sim-passphrase-1";
const CHANNEL: u8 = 6;

const NETS: [(&[u8], Mac); 3] = [
    (SSID_WPA2, [0x02, 0x5A, 0x51, 0x4D, 0x00, 0x01]),
    (SSID_OPEN, [0x02, 0x5A, 0x51, 0x4D, 0x00, 0x02]),
    (SSID_WPA3, [0x02, 0x5A, 0x51, 0x4D, 0x00, 0x03]),
];
/// RSN element data: version 1, group CCMP, pairwise CCMP, AKM PSK, capabilities 0x000C.
const RSN_PSK: [u8; 20] = [1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 2, 0x0C, 0];
/// WPA3-Personal only: AKM SAE (8), protected management frames required and capable (0x00C0).
const RSN_SAE: [u8; 20] = [1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 8, 0xC0, 0];
const SNAP: [u8; 6] = [0xAA, 0xAA, 0x03, 0x00, 0x00, 0x00];
const EAPOL: u16 = 0x888E;

const Q: usize = 8;
const FMAX: usize = 2400;
const BEACON_MS: u64 = 100;

/// The authenticator of the WPA2 network.
struct Ap {
    pmk: Option<crypto::Secret<32>>,
    anonce: [u8; 32],
    replay: u64,
    ptk: crypto::Secret<48>,
    keys: bool,
    /// associated to the open network: data passes in the clear
    open: bool,
    gtk: [u8; 16],
    pn: u64,
    gpn: u64,
    rx: Replay,
    seq: u16,
}

impl Ap {
    fn new() -> Ap {
        Ap { pmk: None, anonce: [0x3C; 32], replay: 0, ptk: crypto::Secret::zero(), keys: false, open: false, gtk: [0x47; 16], pn: 0, gpn: 0, rx: Replay::new(), seq: 0 }
    }
    fn reset(&mut self) {
        self.ptk.0.fill(0);
        self.keys = false;
        self.open = false;
        self.pn = 0;
        self.gpn = 0;
        self.rx = Replay::new();
        // a new ANonce for every handshake (only its uniqueness matters to the station)
        self.anonce[0] = self.anonce[0].wrapping_add(1);
    }
}

pub struct Air {
    own: Mac,
    q: [[u8; FMAX]; Q],
    qlen: [usize; Q],
    qh: usize,
    qn: usize,
    joined: Option<usize>,
    ap: Ap,
    now: u64,
    last_beacon: u64,
    /// what the station sent, waiting for the access point (one handled per tick, so the station's step and the
    /// access point's answer never share one interrupts-off stretch)
    heard: [[u8; FMAX]; Q],
    heard_len: [usize; Q],
    heard_h: usize,
    heard_n: usize,
    /// frames the access point refused or could not pass on (wrong MIC, replays, plaintext data on WPA2, full queue)
    pub dropped: u32,
    /// Ethernet frames passed to and from the e1000 uplink
    pub up_out: u32,
    pub up_in: u32,
}

/// Wi-Fi comes before a cable in this image: the access point's uplink is the e1000, so the e1000 is not the card.
pub const PREFERRED: bool = true;
pub const DESCRIBE: &str = "Wi-Fi (the test image's pretend radio)";
pub const SHORT: &str = "wifi-sim";

impl Air {
    /// The radio, if this PC has one: here, whenever the e1000 is there for the access point's uplink.
    pub fn probe() -> Option<Air> {
        if crate::e1000::init() {
            let mut a = Air::new(crate::e1000::mac());
            // the access point's key, made once here at start-up (a few seconds), never later under the station's
            // interrupts-off guard (the architect's 20 ms masked rule)
            a.ap.pmk = crypto::pmk_from_passphrase(PASS, SSID_WPA2).ok();
            Some(a)
        } else {
            None
        }
    }

    /// One more line for `wifi status`.
    pub fn status_line(&self) {
        crate::kprintln!("  test access point: {} to the uplink, {} from it, {} refused", self.up_out, self.up_in, self.dropped);
    }

    fn new(own: Mac) -> Air {
        Air { own, q: [[0; FMAX]; Q], qlen: [0; Q], qh: 0, qn: 0, heard: [[0; FMAX]; Q], heard_len: [0; Q], heard_h: 0, heard_n: 0, joined: None, ap: Ap::new(), now: 0, last_beacon: 0, dropped: 0, up_out: 0, up_in: 0 }
    }

    fn bssid(&self) -> Option<Mac> {
        self.joined.map(|i| NETS[i].1)
    }

    fn push(&mut self, f: &[u8]) {
        if self.qn == Q || f.len() > FMAX {
            self.dropped += 1;
            return;
        }
        let i = (self.qh + self.qn) % Q;
        self.q[i][..f.len()].copy_from_slice(f);
        self.qlen[i] = f.len();
        self.qn += 1;
    }

    /// The next frame on the air for the station, into `dst`.
    pub fn pop(&mut self, dst: &mut [u8]) -> Option<usize> {
        if self.qn == 0 {
            return None;
        }
        let i = self.qh;
        let n = self.qlen[i];
        dst[..n].copy_from_slice(&self.q[i][..n]);
        self.q[i][..n].fill(0);
        self.qh = (self.qh + 1) % Q;
        self.qn -= 1;
        Some(n)
    }

    /// Time moves (ms since boot): beacons of the joined network, and the e1000's frames for the station.
    pub fn tick(&mut self, now: u64) {
        self.now = now;
        // at most ONE frame's crypto per tick (the architect's 20 ms masked rule: a tick may run from the timer
        // interrupt): either the access point answers one frame the station sent, or one frame comes up the uplink
        let answered = self.heard_n > 0;
        if answered {
            let i = self.heard_h;
            let n = self.heard_len[i];
            let mut f = [0u8; FMAX];
            f[..n].copy_from_slice(&self.heard[i][..n]);
            self.heard[i][..n].fill(0);
            self.heard_h = (self.heard_h + 1) % Q;
            self.heard_n -= 1;
            self.hear(&f[..n]);
            f.fill(0);
        }
        if let Some(i) = self.joined {
            if now.saturating_sub(self.last_beacon) >= BEACON_MS && self.qn < Q / 2 {
                self.last_beacon = now;
                self.beacon(i);
            }
        }
        if !answered {
            self.uplink();
        }
    }

    fn mgmt(&mut self, subtype: u8, a1: &Mac, a2: &Mac, body: &[u8]) {
        let mut f = [0u8; 256];
        let n = 24 + body.len();
        f[0] = subtype << 4;
        f[4..10].copy_from_slice(a1);
        f[10..16].copy_from_slice(a2);
        f[16..22].copy_from_slice(a2);
        self.ap.seq = (self.ap.seq + 1) & 0x0FFF;
        f[22..24].copy_from_slice(&(self.ap.seq << 4).to_le_bytes());
        f[24..n].copy_from_slice(body);
        self.push(&f[..n]);
    }

    fn beacon(&mut self, i: usize) {
        let (ssid, bssid) = NETS[i];
        let mut b = [0u8; 128];
        let mut n = 8; // timestamp
        b[n..n + 2].copy_from_slice(&100u16.to_le_bytes());
        let cap: u16 = if i == 1 { 0x0401 } else { 0x0411 };
        b[n + 2..n + 4].copy_from_slice(&cap.to_le_bytes());
        n += 4;
        b[n] = 0;
        b[n + 1] = ssid.len() as u8;
        b[n + 2..n + 2 + ssid.len()].copy_from_slice(ssid);
        n += 2 + ssid.len();
        b[n..n + 9].copy_from_slice(&[1, 4, 0x82, 0x84, 0x8B, 0x96, 3, 1, CHANNEL]);
        n += 9;
        let rsn: Option<&[u8; 20]> = match i {
            0 => Some(&RSN_PSK),
            2 => Some(&RSN_SAE),
            _ => None,
        };
        if let Some(r) = rsn {
            b[n] = 48;
            b[n + 1] = r.len() as u8;
            b[n + 2..n + 2 + r.len()].copy_from_slice(r);
            n += 2 + r.len();
        }
        self.mgmt(8, &[0xFF; 6], &bssid, &b[..n]);
    }

    /// A FromDS data frame to the station (addr1 `dst`, addr2 the BSSID, addr3 `src`), encrypted when keys are in.
    fn data_down(&mut self, dst: &Mac, src: &Mac, ethertype: u16, payload: &[u8]) -> bool {
        let Some(bssid) = self.bssid() else { return false };
        let mut f = [0u8; FMAX];
        let n = 32 + payload.len();
        if n + 16 > FMAX {
            return false;
        }
        f[0] = 0x08;
        f[1] = 0x02; // FromDS
        f[4..10].copy_from_slice(dst);
        f[10..16].copy_from_slice(&bssid);
        f[16..22].copy_from_slice(src);
        self.ap.seq = (self.ap.seq + 1) & 0x0FFF;
        f[22..24].copy_from_slice(&(self.ap.seq << 4).to_le_bytes());
        f[24..30].copy_from_slice(&SNAP);
        f[30..32].copy_from_slice(&ethertype.to_be_bytes());
        f[32..n].copy_from_slice(payload);
        if !self.ap.keys {
            self.push(&f[..n]);
            f.fill(0);
            return true;
        }
        let group = dst[0] & 1 != 0;
        let (key, id, pn) = if group {
            self.ap.gpn += 1;
            (self.ap.gtk, 1, self.ap.gpn)
        } else {
            self.ap.pn += 1;
            let mut tk = [0u8; 16];
            tk.copy_from_slice(&self.ap.ptk.0[32..48]);
            (tk, 0, self.ap.pn)
        };
        let mut enc = [0u8; FMAX];
        let ok = match ccmp::encrypt(&key, id, pn, &f[..n], &mut enc) {
            Ok(m) => {
                self.push(&enc[..m]);
                true
            }
            Err(_) => false,
        };
        f.fill(0);
        enc.fill(0);
        ok
    }

    /// One of the e1000's received frames for the station (its address, or a group address), sent down over the air.
    fn uplink(&mut self) {
        // only once the station may receive data: associated on the open network, or keys in on the WPA2 one
        if self.joined.is_none() || !(self.ap.open || self.ap.keys) {
            // drain the uplink so old frames do not arrive after joining
            let mut junk = [0u8; 1600];
            while crate::e1000::rx_ready() && crate::e1000::recv(&mut junk).is_some() {}
            return;
        }
        let mut eth = [0u8; 1600];
        for _ in 0..1 {
            if self.qn >= Q - 1 || !crate::e1000::rx_ready() {
                break;
            }
            let Some(n) = crate::e1000::recv(&mut eth) else { break };
            if n < 14 {
                continue;
            }
            let mut dst = [0u8; 6];
            let mut src = [0u8; 6];
            dst.copy_from_slice(&eth[0..6]);
            src.copy_from_slice(&eth[6..12]);
            if dst != self.own && dst[0] & 1 == 0 {
                continue;
            }
            let et = u16::from_be_bytes([eth[12], eth[13]]);
            if self.data_down(&dst, &src, et, &eth[14..n]) {
                self.up_in += 1;
            }
        }
    }

    // ---- the authenticator's handshake (802.11-2020 12.7.6) ------------------------------------------------------

    fn key_frame(&mut self, info: u16, data: &[u8]) -> ([u8; 256], usize) {
        let mut f = [0u8; 256];
        let body = 95 + data.len();
        f[0] = 2; // EAPOL version 2
        f[1] = 3; // EAPOL-Key
        f[2..4].copy_from_slice(&(body as u16).to_be_bytes());
        f[4] = 2; // RSN key descriptor
        f[5..7].copy_from_slice(&info.to_be_bytes());
        f[7..9].copy_from_slice(&16u16.to_be_bytes());
        f[9..17].copy_from_slice(&self.ap.replay.to_be_bytes());
        f[17..49].copy_from_slice(&self.ap.anonce);
        f[97..99].copy_from_slice(&(data.len() as u16).to_be_bytes());
        f[99..99 + data.len()].copy_from_slice(data);
        (f, 4 + body)
    }

    fn msg1(&mut self, sta: &Mac) {
        self.ap.replay += 1;
        // key descriptor version 2, pairwise, ack
        let (f, n) = self.key_frame(0x008A, &[]);
        let bssid = self.bssid().unwrap_or([0; 6]);
        self.data_down(sta, &bssid, EAPOL, &f[..n]);
    }

    fn msg3(&mut self, sta: &Mac) {
        self.ap.replay += 1;
        // key data: the beacon's RSN element and the GTK KDE (key id 1), padded with 0xDD then zeros, AES-wrapped
        let mut kd = [0u8; 56];
        kd[0] = 48;
        kd[1] = RSN_PSK.len() as u8;
        kd[2..22].copy_from_slice(&RSN_PSK);
        kd[22..30].copy_from_slice(&[0xDD, 22, 0x00, 0x0F, 0xAC, 1, 1, 0]);
        kd[30..46].copy_from_slice(&self.ap.gtk);
        kd[46] = 0xDD;
        let mut kek = [0u8; 16];
        kek.copy_from_slice(&self.ap.ptk.0[16..32]);
        let mut w = [0u8; 56];
        if crypto::aes_wrap(&kek, &kd[..48], &mut w).is_err() {
            return;
        }
        crypto::wipe(&mut kd);
        crypto::wipe(&mut kek);
        // version 2, pairwise, install, ack, MIC, secure, encrypted key data
        let (mut f, n) = self.key_frame(0x13CA, &w);
        let m = crypto::hmac_sha1(&self.ap.ptk.0[..16], &[&f[..n]]);
        f[81..97].copy_from_slice(&m[..16]);
        let bssid = self.bssid().unwrap_or([0; 6]);
        self.data_down(sta, &bssid, EAPOL, &f[..n]);
    }

    fn mic_ok(&self, e: &[u8]) -> bool {
        if e.len() < 99 {
            return false;
        }
        let mut z = [0u8; 512];
        if e.len() > z.len() {
            return false;
        }
        z[..e.len()].copy_from_slice(e);
        z[81..97].fill(0);
        crypto::hmac_sha1_verify(&self.ap.ptk.0[..16], &[&z[..e.len()]], &e[81..97])
    }

    fn on_eapol(&mut self, sta: &Mac, e: &[u8]) {
        if e.len() < 99 || e[1] != 3 {
            self.dropped += 1;
            return;
        }
        // the frame as its EAPOL header bounds it (the MIC covers exactly that)
        let e = &e[..(4 + u16::from_be_bytes([e[2], e[3]]) as usize).min(e.len())];
        let info = u16::from_be_bytes([e[5], e[6]]);
        let replay = u64::from_be_bytes(e[9..17].try_into().unwrap_or([0; 8]));
        if replay != self.ap.replay || info & 0x0100 == 0 {
            self.dropped += 1;
            return;
        }
        if info & 0x0200 == 0 {
            // message 2: the PTK from its SNonce (12.7.1.3), then its MIC
            let Some(pmk) = &self.ap.pmk else { return };
            let Some(bssid) = self.bssid() else { return };
            let mut snonce = [0u8; 32];
            snonce.copy_from_slice(&e[17..49]);
            let mut d = [0u8; 76];
            d[0..6].copy_from_slice(&bssid.min(*sta));
            d[6..12].copy_from_slice(&bssid.max(*sta));
            d[12..44].copy_from_slice(&self.ap.anonce.min(snonce));
            d[44..76].copy_from_slice(&self.ap.anonce.max(snonce));
            let pmk = pmk.0;
            if crypto::prf_sha1(&pmk, b"Pairwise key expansion", &d, &mut self.ap.ptk.0).is_err() {
                return;
            }
            if !self.mic_ok(e) {
                // a wrong passphrase: the station times out, as with a real access point
                self.ap.ptk.0.fill(0);
                self.dropped += 1;
                return;
            }
            self.msg3(sta);
        } else if self.mic_ok(e) {
            // message 4: the keys are in from now on
            self.ap.keys = true;
        } else {
            self.dropped += 1;
        }
    }

    fn on_data(&mut self, f: &[u8]) {
        let Some(bssid) = self.bssid() else { return };
        let mut plain = [0u8; FMAX];
        let was_protected = f.len() > 1 && f[1] & 0x40 != 0;
        let p: &[u8] = if was_protected {
            if !self.ap.keys {
                self.dropped += 1;
                return;
            }
            let mut tk = [0u8; 16];
            tk.copy_from_slice(&self.ap.ptk.0[32..48]);
            let r = ccmp::decrypt(&tk, f, &mut plain);
            crypto::wipe(&mut tk);
            match r {
                Ok(r) if self.ap.rx.accept(r.slot, r.pn) => &plain[..r.len],
                _ => {
                    self.dropped += 1;
                    return;
                }
            }
        } else {
            f
        };
        let Ok(h) = parse_header(p) else { return };
        if !h.to_ds || h.from_ds || h.addr1 != bssid || h.body.len() < 8 || h.body[..6] != SNAP {
            self.dropped += 1;
            return;
        }
        let sta = h.addr2;
        let et = u16::from_be_bytes([h.body[6], h.body[7]]);
        let payload = &h.body[8..];
        if et == EAPOL && self.joined == Some(0) {
            let mut e = [0u8; 512];
            let n = payload.len().min(e.len());
            e[..n].copy_from_slice(&payload[..n]);
            self.on_eapol(&sta, &e[..n]);
            return;
        }
        // the network's data: on WPA2 only encrypted frames after the handshake; on the open one only clear ones
        let pass = if self.ap.open { !was_protected } else { self.ap.keys && was_protected };
        if !pass {
            self.dropped += 1;
            return;
        }
        let mut eth = [0u8; 1600];
        let n = 14 + payload.len();
        if n > eth.len() {
            self.dropped += 1;
            return;
        }
        eth[0..6].copy_from_slice(&h.addr3);
        eth[6..12].copy_from_slice(&sta);
        eth[12..14].copy_from_slice(&et.to_be_bytes());
        eth[14..n].copy_from_slice(payload);
        if crate::e1000::send(&eth[..n]) {
            self.up_out += 1;
        }
        plain.fill(0);
    }

    fn on_mgmt(&mut self, subtype: u8, sta: &Mac, body: &[u8]) {
        let Some(i) = self.joined else { return };
        let bssid = NETS[i].1;
        match subtype {
            11 => {
                // open system authentication, sequence 2, success
                self.mgmt(11, sta, &bssid, &[0, 0, 2, 0, 0, 0]);
            }
            0 => {
                if i == 2 {
                    // the WPA3 network refuses a WPA2 association (status 43: invalid AKMP); the station should never
                    // get this far
                    self.mgmt(1, sta, &bssid, &[0x11, 0x04, 43, 0, 0, 0]);
                    return;
                }
                let cap: [u8; 2] = if i == 1 { [0x01, 0x04] } else { [0x11, 0x04] };
                self.mgmt(1, sta, &bssid, &[cap[0], cap[1], 0, 0, 0x01, 0xC0, 1, 4, 0x82, 0x84, 0x8B, 0x96]);
                self.ap.reset();
                if i == 0 {
                    self.msg1(sta);
                } else {
                    self.ap.open = true; // the open network passes data from now on
                }
                let _ = body;
            }
            12 | 10 => self.ap.reset(),
            _ => {}
        }
    }
}

impl Air {
    /// The access point hears one frame the station sent (from `tick`).
    fn hear(&mut self, f: &[u8]) {
        let Ok(h) = parse_header(f) else { return };
        if self.bssid() != Some(h.addr1) {
            return; // on the air, but for nobody here
        }
        match h.ftype {
            TYPE_MGMT => self.on_mgmt(h.subtype, &h.addr2, h.body),
            TYPE_DATA => self.on_data(f),
            _ => {}
        }
    }
}

impl Radio for Air {
    fn mac(&self) -> Mac {
        self.own
    }
    fn tx(&mut self, f: &[u8]) -> bool {
        if parse_header(f).is_err() || f.len() > FMAX {
            return false;
        }
        if self.heard_n == Q {
            self.dropped += 1;
            return true; // lost on the air, as a busy channel loses it
        }
        let i = (self.heard_h + self.heard_n) % Q;
        self.heard[i][..f.len()].copy_from_slice(f);
        self.heard_len[i] = f.len();
        self.heard_n += 1;
        true
    }
    fn start_scan(&mut self) -> bool {
        for i in 0..NETS.len() {
            self.beacon(i);
        }
        true
    }
    fn join(&mut self, b: &BssInfo) -> bool {
        match NETS.iter().position(|n| n.1 == b.bssid) {
            Some(i) => {
                self.joined = Some(i);
                self.ap.reset();
                self.ap.replay = 0;
                self.last_beacon = self.now;
                true
            }
            None => false,
        }
    }
    fn associated(&mut self, _b: &BssInfo, _aid: u16) -> bool {
        true
    }
    fn leave(&mut self) {
        self.joined = None;
        self.heard_n = 0;
        self.ap.reset();
        self.ap.replay = 0;
        self.qn = 0;
    }
}
