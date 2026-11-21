//! The station (W4): scan results, choosing a network, open-system authentication, association, the WPA2
//! handshake through supp.rs, and data both ways once connected. Written from IEEE 802.11-2020 11.3 (states),
//! 11.1.4 (scanning) and 12.7 (RSNA). Pure code over a [`Radio`]: the Intel driver (W5) and the test radio of
//! suite pc-wifi-sim implement it. Nothing here prints; the shell reads [`Station::status`] and the events.
//!
//! The architect's rules for this layer (W2 verdict, 14 Nov 2026):
//! 1. EAPOL (EtherType 0x888E) from received data goes only to the supplicant, never to TCP/IP; nothing else
//!    reaches the supplicant.
//! 2. Before the PTK is installed only EAPOL is taken (data is dropped); after, an unprotected data frame is dropped
//!    and counted, EAPOL excepted (the standard lets a retransmitted message 3 come unprotected).
//! 3. No downgrade by a twin: `connect` chooses the best security offered under that name and then only among
//!    networks of that name with that security; it says when it ignores weaker ones.
//! Also: frames are taken only from the chosen BSSID; group frames only with the GTK; every PN through its replay
//! counter; keys and the PMK wiped on disconnect.
#![allow(dead_code)]

use super::bss::{Entry, Table};
use super::ccmp::{self, Replay, TxPn};
use super::crypto::Secret;
use super::frame::{
    build_assoc_req, build_auth_open, build_deauth, build_rsn, parse_assoc_resp, parse_auth, parse_beacon, parse_header, parse_reason, BssInfo, Mac,
    Security, Suite, AKM_PSK, CIPHER_CCMP, ST_ASSOC_RESP, ST_AUTH, ST_BEACON, ST_DEAUTH, ST_DISASSOC, ST_PROBE_RESP, ST_REASSOC_RESP, STATUS_OK,
    TYPE_DATA, TYPE_MGMT,
};
use super::llc::{self, ETHERTYPE_EAPOL};
use super::supp::{Outcome, Supplicant};

/// What a chip driver gives the station. Frames are whole 802.11 frames without FCS.
pub trait Radio {
    fn mac(&self) -> Mac;
    /// Send one frame (management or data, already encrypted if it must be). False if the radio could not.
    fn tx(&mut self, frame: &[u8]) -> bool;
    /// Start a scan of every channel the chip may use (passive where it must be). Results arrive as beacons and
    /// probe responses through [`Station::on_frame`].
    fn start_scan(&mut self) -> bool;
    /// Get ready to talk to this network (tune to its channel; Intel: MAC context, binding, time event).
    fn join(&mut self, bss: &BssInfo) -> bool;
    /// Association succeeded with this association id (Intel: the station entry in the firmware).
    fn associated(&mut self, bss: &BssInfo, aid: u16) -> bool;
    /// Back to idle.
    fn leave(&mut self);
}

pub const AUTH_TIMEOUT_MS: u64 = 300;
pub const ASSOC_TIMEOUT_MS: u64 = 300;
pub const TRIES: u8 = 3;
/// No beacon from our AP for this long: the connection is lost.
pub const BEACON_LOSS_MS: u64 = 4000;
pub const SCAN_AGE_MS: u64 = 30_000;
const RXQ: usize = 8;
const ETH_MAX: usize = 1600;
const FRAME_MAX: usize = 2400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Authenticating,
    Associating,
    Handshake,
    Connected,
}

/// Why `connect` could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoConnect {
    /// no network of that name heard (scan first)
    NotFound,
    /// the network needs a passphrase and none was given
    NeedsPassphrase,
    /// an open network was given a passphrase
    NotProtected,
    /// the best security under that name is one this station does not join (WPA3 only, enterprise, old)
    Unsupported(Security),
    /// the radio refused
    Radio,
}

/// Why a connection ended or failed. The shell says it in plain words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    AuthRefused(u16),
    AssocRefused(u16),
    NoAnswer,
    /// the 4-way handshake failed (most often a wrong passphrase: the AP never sends message 3)
    Handshake(super::supp::Fail),
    Deauth(u16),
    BeaconLoss,
    ByUser,
}

/// Something the shell may want to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Connected,
    Ended(Ended),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub rx_frames: u32,
    pub rx_bad: u32,
    /// data refused for rule 2 (before keys, or unprotected after keys)
    pub rx_plain_dropped: u32,
    pub rx_mic_fail: u32,
    pub rx_replay: u32,
    pub rx_not_ours: u32,
    pub rx_queue_full: u32,
    pub tx_frames: u32,
    pub tx_fail: u32,
}

pub struct Station {
    pub phase: Phase,
    pub table: Table,
    own: Mac,
    bss: Option<BssInfo>,
    pmk: Option<Secret<32>>,
    own_rsn: [u8; 32],
    own_rsn_len: usize,
    tries: u8,
    since_ms: u64,
    last_beacon_ms: u64,
    seq: u16,
    aid: u16,
    supp: Supplicant,
    tk: Option<Secret<16>>,
    tx_pn: TxPn,
    rx_replay: Replay,
    gtk: [Option<(Secret<16>, Replay)>; 4],
    rxq: [[u8; ETH_MAX]; RXQ],
    rxq_len: [usize; RXQ],
    rxq_head: usize,
    rxq_n: usize,
    pub counters: Counters,
    /// networks of the chosen name left out because their security was weaker (rule 3), for the shell to say
    pub weaker_ignored: usize,
    pub signal: i8,
}

fn rng_none(_: &mut [u8]) -> bool {
    false
}

impl Station {
    pub fn new(own: Mac) -> Self {
        Station {
            phase: Phase::Idle,
            table: Table::new(),
            own,
            bss: None,
            pmk: None,
            own_rsn: [0; 32],
            own_rsn_len: 0,
            tries: 0,
            since_ms: 0,
            last_beacon_ms: 0,
            seq: 0,
            aid: 0,
            supp: Supplicant::new(),
            tk: None,
            tx_pn: TxPn::new(),
            rx_replay: Replay::new(),
            gtk: [None, None, None, None],
            rxq: [[0; ETH_MAX]; RXQ],
            rxq_len: [0; RXQ],
            rxq_head: 0,
            rxq_n: 0,
            counters: Counters::default(),
            weaker_ignored: 0,
            signal: 0,
        }
    }

    pub fn own(&self) -> Mac {
        self.own
    }

    pub fn bss(&self) -> Option<&BssInfo> {
        self.bss.as_ref()
    }

    fn next_seq(&mut self) -> u16 {
        self.seq = (self.seq + 1) & 0x0FFF;
        self.seq
    }

    pub fn scan<R: Radio>(&mut self, radio: &mut R, now_ms: u64) -> bool {
        self.table.age(now_ms, SCAN_AGE_MS);
        radio.start_scan()
    }

    /// Choose the network for `ssid` by rule 3 and start joining it. `pmk` is given for a protected network
    /// (the caller made it from the passphrase and the exact SSID bytes).
    pub fn connect<R: Radio>(&mut self, radio: &mut R, ssid: &[u8], pmk: Option<[u8; 32]>, now_ms: u64) -> Result<(), NoConnect> {
        self.disconnect_quiet(radio);
        let rank = |s: Security| match s {
            Security::Wpa2Psk { sha256: false } => 3,
            Security::Wpa3Only | Security::Enterprise | Security::Wpa2Psk { sha256: true } | Security::Unknown => 2,
            Security::Open => 1,
            Security::Old => 0,
        };
        let mut out = [None; super::bss::MAX_BSS];
        let n = self.table.sorted(&mut out);
        let same: &[Option<&Entry>] = &out[..n];
        let named = same.iter().flatten().filter(|e| e.info.ssid() == ssid);
        let best_sec = named.clone().map(|e| e.info.security).max_by_key(|s| rank(*s)).ok_or(NoConnect::NotFound)?;
        // the strongest signal among those with the best security; everything weaker under that name is ignored
        let pick = named.clone().find(|e| e.info.security == best_sec).copied().ok_or(NoConnect::NotFound)?;
        self.weaker_ignored = named.filter(|e| rank(e.info.security) < rank(best_sec)).count();
        match best_sec {
            Security::Wpa2Psk { sha256: false } if pmk.is_none() => return Err(NoConnect::NeedsPassphrase),
            Security::Wpa2Psk { sha256: false } => {}
            Security::Open if pmk.is_some() => return Err(NoConnect::NotProtected),
            Security::Open => {}
            other => return Err(NoConnect::Unsupported(other)),
        }
        let info = pick.info;
        if !radio.join(&info) {
            return Err(NoConnect::Radio);
        }
        if let Some(k) = pmk {
            self.pmk = Some(Secret(k));
            let mut ie = [0u8; 32];
            let len = build_rsn(Suite::ieee(CIPHER_CCMP), Suite::ieee(CIPHER_CCMP), Suite::ieee(AKM_PSK), 0, &mut ie).map_err(|_| NoConnect::Radio)?;
            self.own_rsn = ie;
            self.own_rsn_len = len;
        } else {
            self.own_rsn_len = 0;
        }
        self.bss = Some(info);
        self.signal = pick.signal;
        self.tries = 0;
        self.send_auth(radio, now_ms);
        Ok(())
    }

    fn send_auth<R: Radio>(&mut self, radio: &mut R, now_ms: u64) {
        let Some(b) = self.bss else { return };
        let mut f = [0u8; 64];
        let seq = self.next_seq();
        if let Ok(n) = build_auth_open(&self.own, &b.bssid, seq, &mut f) {
            self.tx_raw(radio, &f[..n]);
        }
        self.phase = Phase::Authenticating;
        self.tries += 1;
        self.since_ms = now_ms;
    }

    fn send_assoc<R: Radio>(&mut self, radio: &mut R, now_ms: u64) {
        let Some(b) = self.bss else { return };
        let mut f = [0u8; 256];
        let seq = self.next_seq();
        // capability: ESS, short preamble, short slot time; privacy when the network is protected
        let cap: u16 = 0x0001 | 0x0020 | 0x0400 | if self.own_rsn_len > 0 { 0x0010 } else { 0 };
        let rsn = self.own_rsn;
        if let Ok(n) = build_assoc_req(&self.own, &b, cap, 10, &rsn[..self.own_rsn_len], seq, &mut f) {
            self.tx_raw(radio, &f[..n]);
        }
        self.phase = Phase::Associating;
        self.tries += 1;
        self.since_ms = now_ms;
    }

    fn tx_raw<R: Radio>(&mut self, radio: &mut R, f: &[u8]) {
        if radio.tx(f) {
            self.counters.tx_frames += 1;
        } else {
            self.counters.tx_fail += 1;
        }
    }

    fn wipe_keys(&mut self) {
        self.supp.reset();
        self.tk = None; // Secret wipes on drop
        self.gtk = [None, None, None, None];
        self.pmk = None;
        self.tx_pn = TxPn::new();
        self.rx_replay = Replay::new();
        self.rxq_n = 0;
        for q in self.rxq.iter_mut() {
            q.fill(0);
        }
    }

    fn disconnect_quiet<R: Radio>(&mut self, radio: &mut R) {
        if self.phase != Phase::Idle {
            radio.leave();
        }
        self.wipe_keys();
        self.phase = Phase::Idle;
        self.bss = None;
    }

    fn end<R: Radio>(&mut self, radio: &mut R, why: Ended) -> Option<Event> {
        self.disconnect_quiet(radio);
        Some(Event::Ended(why))
    }

    /// Leave the network (a deauthentication is sent if associated).
    pub fn disconnect<R: Radio>(&mut self, radio: &mut R) -> Option<Event> {
        if let (Some(b), Phase::Handshake | Phase::Connected) = (self.bss, self.phase) {
            let mut f = [0u8; 32];
            let seq = self.next_seq();
            if let Ok(n) = build_deauth(&self.own, &b.bssid, seq, 3, &mut f) {
                self.tx_raw(radio, &f[..n]); // reason 3: the station is leaving
            }
        }
        if self.phase == Phase::Idle {
            return None;
        }
        self.end(radio, Ended::ByUser)
    }

    /// Timeouts, retries and beacon loss. Call often.
    pub fn poll<R: Radio>(&mut self, radio: &mut R, now_ms: u64) -> Option<Event> {
        let waited = now_ms.saturating_sub(self.since_ms);
        match self.phase {
            Phase::Authenticating if waited > AUTH_TIMEOUT_MS => {
                if self.tries >= TRIES {
                    return self.end(radio, Ended::NoAnswer);
                }
                self.send_auth(radio, now_ms);
            }
            Phase::Associating if waited > ASSOC_TIMEOUT_MS => {
                if self.tries >= TRIES {
                    return self.end(radio, Ended::NoAnswer);
                }
                self.send_assoc(radio, now_ms);
            }
            Phase::Handshake => {
                if let Some(f) = self.supp.tick(now_ms) {
                    return self.end(radio, Ended::Handshake(f));
                }
            }
            Phase::Connected if now_ms.saturating_sub(self.last_beacon_ms) > BEACON_LOSS_MS => {
                return self.end(radio, Ended::BeaconLoss);
            }
            _ => {}
        }
        None
    }

    /// One received frame (no FCS), with the signal and channel the radio measured.
    pub fn on_frame<R: Radio>(&mut self, radio: &mut R, f: &[u8], signal: i8, channel: u8, now_ms: u64) -> Option<Event> {
        self.counters.rx_frames += 1;
        let Ok(h) = parse_header(f) else {
            self.counters.rx_bad += 1;
            return None;
        };
        if h.ftype == TYPE_MGMT {
            if h.subtype == ST_BEACON || h.subtype == ST_PROBE_RESP {
                match parse_beacon(f, channel) {
                    Ok(info) => {
                        if self.bss.is_some_and(|b| b.bssid == info.bssid) {
                            self.last_beacon_ms = now_ms;
                            self.signal = signal;
                        }
                        self.table.update(info, signal, now_ms);
                    }
                    Err(_) => self.counters.rx_bad += 1,
                }
                return None;
            }
            let Some(b) = self.bss else {
                return None;
            };
            if h.addr2 != b.bssid || h.addr1 != self.own {
                self.counters.rx_not_ours += 1;
                return None;
            }
            return self.on_mgmt(radio, h.subtype, h.body, now_ms);
        }
        if h.ftype == TYPE_DATA {
            return self.on_data(radio, f, now_ms);
        }
        None
    }

    fn on_mgmt<R: Radio>(&mut self, radio: &mut R, subtype: u8, body: &[u8], now_ms: u64) -> Option<Event> {
        match (self.phase, subtype) {
            (Phase::Authenticating, ST_AUTH) => {
                let Ok(a) = parse_auth(body) else {
                    self.counters.rx_bad += 1;
                    return None;
                };
                if a.algorithm != 0 || a.seq != 2 {
                    return None;
                }
                if a.status != STATUS_OK {
                    return self.end(radio, Ended::AuthRefused(a.status));
                }
                self.tries = 0;
                self.send_assoc(radio, now_ms);
                None
            }
            (Phase::Associating, ST_ASSOC_RESP | ST_REASSOC_RESP) => {
                let Ok(r) = parse_assoc_resp(body) else {
                    self.counters.rx_bad += 1;
                    return None;
                };
                if r.status != STATUS_OK {
                    return self.end(radio, Ended::AssocRefused(r.status));
                }
                let b = self.bss?;
                self.aid = r.aid;
                if !radio.associated(&b, r.aid) {
                    return self.end(radio, Ended::NoAnswer);
                }
                self.last_beacon_ms = now_ms;
                match &self.pmk {
                    None => {
                        self.phase = Phase::Connected;
                        Some(Event::Connected)
                    }
                    Some(p) => {
                        let pmk = p.0;
                        let rsn = self.own_rsn;
                        let r = self.supp.start(&self.own, &b.bssid, &pmk, b.rsn(), &rsn[..self.own_rsn_len], now_ms);
                        if let Err(f) = r {
                            return self.end(radio, Ended::Handshake(f));
                        }
                        self.phase = Phase::Handshake;
                        self.since_ms = now_ms;
                        None
                    }
                }
            }
            (Phase::Associating | Phase::Handshake | Phase::Connected, ST_DEAUTH | ST_DISASSOC) => {
                let reason = parse_reason(body).unwrap_or(0);
                self.end(radio, Ended::Deauth(reason))
            }
            (Phase::Authenticating, ST_DEAUTH) => {
                let reason = parse_reason(body).unwrap_or(0);
                self.end(radio, Ended::Deauth(reason))
            }
            _ => None,
        }
    }

    fn on_data<R: Radio>(&mut self, radio: &mut R, f: &[u8], now_ms: u64) -> Option<Event> {
        let b = self.bss?;
        if !matches!(self.phase, Phase::Handshake | Phase::Connected) {
            return None;
        }
        let Ok(h) = parse_header(f) else { return None };
        if h.addr2 != b.bssid {
            self.counters.rx_not_ours += 1;
            return None;
        }
        let mut plain = [0u8; FRAME_MAX];
        let frame: &[u8] = if h.protected {
            let Some(len) = self.decrypt(f, &h.addr1, &mut plain) else { return None };
            &plain[..len]
        } else {
            f
        };
        let protected = h.protected;
        let keys_in = self.tk.is_some();
        let mut eapol: [u8; 512] = [0; 512];
        let mut eapol_len = 0usize;
        let mut to_stack = [[0u8; ETH_MAX]; 2];
        let mut to_stack_len = [0usize; 2];
        let mut n_stack = 0usize;
        let mut dropped = 0u32;
        let own = self.own;
        let (open_net, connected) = (self.pmk.is_none(), self.phase == Phase::Connected);
        let r = llc::rx_data(frame, &own, &b.bssid, |m| {
            if m.ethertype == ETHERTYPE_EAPOL {
                // rule 1: EAPOL only to the supplicant; rule 2: EAPOL may come unprotected
                if eapol_len == 0 && m.payload.len() <= eapol.len() {
                    eapol[..m.payload.len()].copy_from_slice(m.payload);
                    eapol_len = m.payload.len();
                }
                return;
            }
            // rule 2: data only once keys are in (or on an open network once connected), and then only protected
            let open_ok = !keys_in && open_net && connected && !protected;
            let wpa_ok = keys_in && protected;
            if !(open_ok || wpa_ok) {
                dropped += 1;
                return;
            }
            if n_stack < to_stack.len() {
                if let Ok(len) = llc::to_ethernet(&m, &mut to_stack[n_stack]) {
                    to_stack_len[n_stack] = len;
                    n_stack += 1;
                }
            }
        });
        plain.fill(0);
        self.counters.rx_plain_dropped += dropped;
        if r.is_err() {
            self.counters.rx_bad += 1;
            return None;
        }
        for i in 0..n_stack {
            self.push_rx(&to_stack[i][..to_stack_len[i]]);
        }
        if eapol_len > 0 && self.pmk.is_some() {
            return self.on_eapol(radio, &eapol[..eapol_len], now_ms);
        }
        None
    }

    fn decrypt(&mut self, f: &[u8], addr1: &Mac, out: &mut [u8]) -> Option<usize> {
        let (keyid, pn) = ccmp::peek(f).ok()?;
        let group = addr1[0] & 1 != 0;
        let slot = ccmp::replay_slot(f).ok()?;
        let r = if group {
            let Some((key, rep)) = self.gtk.get_mut(keyid as usize).and_then(|g| g.as_mut()) else {
                self.counters.rx_mic_fail += 1;
                return None;
            };
            if !rep.fresh(slot, pn) {
                self.counters.rx_replay += 1;
                return None;
            }
            let r = ccmp::decrypt(&key.0, f, out).ok();
            if r.is_some() {
                rep.accept(slot, pn);
            }
            r
        } else {
            let Some(tk) = &self.tk else {
                self.counters.rx_plain_dropped += 1;
                return None;
            };
            if keyid != 0 || !self.rx_replay.fresh(slot, pn) {
                self.counters.rx_replay += 1;
                return None;
            }
            let r = ccmp::decrypt(&tk.0, f, out).ok();
            if r.is_some() {
                self.rx_replay.accept(slot, pn);
            }
            r
        };
        match r {
            Some(x) => Some(x.len),
            None => {
                self.counters.rx_mic_fail += 1;
                None
            }
        }
    }

    fn on_eapol<R: Radio>(&mut self, radio: &mut R, frame: &[u8], now_ms: u64) -> Option<Event> {
        let mut out = [0u8; 512];
        let mut rng = |b: &mut [u8]| (self_rng())(b);
        let o: Outcome = self.supp.on_eapol(frame, now_ms, &mut rng, &mut out);
        if let Some(n) = o.send {
            // message 4 goes out before the keys are installed, so it is unencrypted; later replies are protected
            self.send_eth_raw(radio, ETHERTYPE_EAPOL, &out[..n]);
        }
        if o.install_ptk {
            if let Some(tk) = self.supp.tk() {
                let mut k = Secret::<16>::zero();
                k.0.copy_from_slice(tk);
                self.tk = Some(k);
                self.tx_pn = TxPn::new();
                self.rx_replay = Replay::new();
            }
        }
        if let Some(id) = o.install_gtk {
            if let Some((g, rsc)) = self.supp.gtk(id) {
                self.gtk[id as usize] = Some((Secret(*g), Replay::starting_at(rsc)));
            }
        }
        if let Some(f) = o.fail {
            return self.end(radio, Ended::Handshake(f));
        }
        if o.install_ptk && self.phase == Phase::Handshake {
            self.phase = Phase::Connected;
            return Some(Event::Connected);
        }
        None
    }

    /// Send an Ethernet payload of this EtherType to the AP (EAPOL replies, and the stack's frames).
    fn send_eth_raw<R: Radio>(&mut self, radio: &mut R, ethertype: u16, payload: &[u8]) -> bool {
        let Some(b) = self.bss else { return false };
        let mut eth = [0u8; ETH_MAX];
        if payload.len() + 14 > eth.len() {
            return false;
        }
        eth[0..6].copy_from_slice(&b.bssid);
        eth[6..12].copy_from_slice(&self.own);
        eth[12..14].copy_from_slice(&ethertype.to_be_bytes());
        eth[14..14 + payload.len()].copy_from_slice(payload);
        self.send_eth(radio, &eth[..14 + payload.len()])
    }

    /// Send one Ethernet frame from the stack: only when connected; encrypted when keys are in.
    pub fn send_eth<R: Radio>(&mut self, radio: &mut R, eth: &[u8]) -> bool {
        let Some(b) = self.bss else { return false };
        let is_eapol = eth.len() >= 14 && u16::from_be_bytes([eth[12], eth[13]]) == ETHERTYPE_EAPOL;
        if self.phase != Phase::Connected && !(is_eapol && self.phase == Phase::Handshake) {
            return false;
        }
        let mut plain = [0u8; FRAME_MAX];
        let seq = self.next_seq();
        let Ok(n) = llc::from_ethernet(eth, &self.own, &b.bssid, seq, &mut plain) else {
            return false;
        };
        let ok = match &self.tk {
            Some(tk) => {
                let Some(pn) = self.tx_pn.take() else { return false };
                let mut enc = [0u8; FRAME_MAX];
                match ccmp::encrypt(&tk.0, 0, pn, &plain[..n], &mut enc) {
                    Ok(m) => {
                        let r = radio.tx(&enc[..m]);
                        enc.fill(0);
                        r
                    }
                    Err(_) => false,
                }
            }
            None if self.pmk.is_none() || is_eapol => radio.tx(&plain[..n]),
            None => false,
        };
        plain.fill(0);
        if ok {
            self.counters.tx_frames += 1;
        } else {
            self.counters.tx_fail += 1;
        }
        ok
    }

    fn push_rx(&mut self, eth: &[u8]) {
        if self.rxq_n == RXQ || eth.len() > ETH_MAX {
            self.counters.rx_queue_full += 1;
            return;
        }
        let i = (self.rxq_head + self.rxq_n) % RXQ;
        self.rxq[i][..eth.len()].copy_from_slice(eth);
        self.rxq_len[i] = eth.len();
        self.rxq_n += 1;
    }

    /// The next Ethernet frame for the stack, if any.
    pub fn recv_eth(&mut self, dst: &mut [u8]) -> Option<usize> {
        if self.rxq_n == 0 {
            return None;
        }
        let i = self.rxq_head;
        let n = self.rxq_len[i];
        let d = dst.get_mut(..n)?;
        d.copy_from_slice(&self.rxq[i][..n]);
        self.rxq[i][..n].fill(0);
        self.rxq_head = (self.rxq_head + 1) % RXQ;
        self.rxq_n -= 1;
        Some(n)
    }

    pub fn rx_ready(&self) -> bool {
        self.rxq_n > 0
    }

    /// Does the station still hold any key or received plaintext? False after a disconnect, a failure or a
    /// refused connect (the architect's wipe rule; host tests check it).
    pub fn holds_secrets(&self) -> bool {
        self.pmk.is_some()
            || self.tk.is_some()
            || self.gtk.iter().any(|g| g.is_some())
            || self.supp.holds_secrets()
            || self.rxq.iter().any(|q| q.iter().any(|&b| b != 0))
    }
}

/// The secret random source the supplicant uses for its SNonce. The kernel sets it once at start-up to its DRBG's
/// fill_secret; until then (and in a test that does not set it) there is none and a handshake fails with NoRandom.
static mut RNG: fn(&mut [u8]) -> bool = rng_none;

/// Set the secret random source (the kernel: drbg fill_secret). Called once, from the shell core, before any connect.
pub fn set_secret_rng(f: fn(&mut [u8]) -> bool) {
    // SAFETY: set once from the single shell core before any Wi-Fi work, read only from that same core
    unsafe { RNG = f };
}

fn self_rng() -> fn(&mut [u8]) -> bool {
    // SAFETY: see set_secret_rng
    unsafe { RNG }
}
