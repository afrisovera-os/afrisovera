//! The WPA2-Personal supplicant (W3): the 4-way handshake and the group key handshake, written from IEEE
//! 802.11-2020 12.7.6 and 12.7.7. AKM 2 (PSK, key descriptor version 2: HMAC-SHA1 MIC, AES key wrap) with CCMP for
//! pairwise and group; anything else is refused. Pure code: frames in, frames and key events out; the station
//! layer (W4) sends, installs keys and keeps time. Nothing here prints or touches hardware.
//!
//! Rules kept (WIFI_PLAN.md section 4 and the architect's W3 conditions):
//! - The MIC is checked (constant time) before any field of a message 3 or group message 1 is used, and the
//!   replay counter moves only after the MIC checked; it must be strictly greater each time.
//! - No key is installed twice (KRACK): a retransmitted message 3 is answered with message 4 again, but the PTK is
//!   not installed again and its packet numbers are not reset; a group key equal to the one installed is not
//!   installed again either.
//! - The RSN element message 3 carries must equal, byte for byte, the one in the AP's beacon (no downgrade).
//! - Only CCMP: a pairwise key length other than 16 or a group key other than 16 bytes fails the handshake.
//! - Refused: an unexpected key descriptor version, a request or error bit, key data that runs past the frame, a
//!   message kind out of turn. A refused frame changes nothing; it is counted.
//! - Every waiting state times out (TIMEOUT_MS). SNonce comes from the caller's secret random source only, and the
//!   handshake fails if it has none.
//! - Secrets (PMK, PTK, the temporary PTK, GTKs) live in `Secret`s, wiped on reset, on failure and on drop.
#![allow(dead_code)]

use super::crypto::{aes_unwrap, hmac_sha1, hmac_sha1_verify, prf_sha1, wipe, Secret};
use super::eapol::{self, Item, KI_ACK, KI_ENCRYPTED, KI_ERROR, KI_INSTALL, KI_MIC, KI_PAIRWISE, KI_REQUEST, KI_SECURE, KI_VERSION_2};
use super::frame::{Mac, IE_RSN};

pub const PTK_LEN: usize = 48; // KCK 0..16, KEK 16..32, TK 32..48
pub const TIMEOUT_MS: u64 = 5000;
const RSN_MAX: usize = 64;
/// Largest EAPOL frame taken (a message 3 is about 200 bytes).
const FRAME_MAX: usize = 512;
const KEYDATA_MAX: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    WaitMsg1,
    WaitMsg3,
    Done,
    Failed,
}

/// Why the handshake ended without keys. The station disconnects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Timeout,
    /// no secret random bytes for the SNonce
    NoRandom,
    /// message 3's RSN element is not the beacon's: a possible downgrade
    RsnMismatch,
    /// the AP chose a cipher other than CCMP (pairwise key length, group key length, a second RSN element)
    NotCcmp,
    /// message 3 or group message 1 without (valid) encrypted key data or without a GTK
    KeyData,
}

/// Why one frame was ignored. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Parse,
    Version,
    Kind,
    State,
    Replay,
    Mic,
    Nonce,
}

/// What one received frame led to. Order for the station: send `send` first (message 4 still goes out
/// unencrypted), then install the PTK and the GTK.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// an EAPOL frame of this length was written to `out`: send it to the AP
    pub send: Option<usize>,
    pub install_ptk: bool,
    pub install_gtk: Option<u8>,
    pub fail: Option<Fail>,
    pub refused: Option<Refused>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub refused: u32,
    pub bad_mic: u32,
    pub replays: u32,
}

pub struct Supplicant {
    pub state: State,
    own: Mac,
    aa: Mac,
    pmk: Secret<32>,
    ap_rsn: [u8; RSN_MAX],
    ap_rsn_len: usize,
    own_rsn: [u8; RSN_MAX],
    own_rsn_len: usize,
    snonce: Secret<32>,
    have_snonce: bool,
    anonce: [u8; 32],
    msg1_replay: Option<u64>,
    /// the replay counter of the last frame whose MIC checked
    replay_ok: Option<u64>,
    tptk: Secret<PTK_LEN>,
    ptk: Secret<PTK_LEN>,
    ptk_installed: bool,
    gtk: [Secret<16>; 4],
    gtk_set: [bool; 4],
    gtk_rsc: [u64; 4],
    started_ms: u64,
    reply_version: u8,
    pub counters: Counters,
}

fn refused(r: Refused) -> Outcome {
    Outcome { refused: Some(r), ..Outcome::default() }
}

/// PTK = PRF-384(PMK, "Pairwise key expansion", Min(AA, SPA) || Max(AA, SPA) || Min(ANonce, SNonce) ||
/// Max(ANonce, SNonce)), 802.11-2020 12.7.1.3.
pub fn derive_ptk(pmk: &[u8; 32], aa: &Mac, spa: &Mac, anonce: &[u8; 32], snonce: &[u8; 32], out: &mut [u8; PTK_LEN]) {
    let (a1, a2) = if aa < spa { (aa, spa) } else { (spa, aa) };
    let (n1, n2) = if anonce < snonce { (anonce, snonce) } else { (snonce, anonce) };
    let mut data = [0u8; 76];
    data[0..6].copy_from_slice(a1);
    data[6..12].copy_from_slice(a2);
    data[12..44].copy_from_slice(n1);
    data[44..76].copy_from_slice(n2);
    // 48 bytes is far inside the PRF's limit, so this cannot fail
    let _ = prf_sha1(pmk, b"Pairwise key expansion", &data, out);
    wipe(&mut data);
}

/// The EAPOL-Key MIC (descriptor version 2): the first 16 bytes of HMAC-SHA1(KCK, frame with the MIC field zero).
pub fn mic(kck: &[u8], frame: &[u8]) -> Option<[u8; 16]> {
    let mut tmp = [0u8; FRAME_MAX];
    let t = tmp.get_mut(..frame.len())?;
    t.copy_from_slice(frame);
    t.get_mut(eapol::MIC_AT..eapol::MIC_AT + eapol::MIC_LEN)?.iter_mut().for_each(|b| *b = 0);
    let full = hmac_sha1(kck, &[&*t]);
    let mut m = [0u8; 16];
    m.copy_from_slice(&full[..16]);
    Some(m)
}

fn mic_ok(kck: &[u8], k: &eapol::Key<'_>) -> bool {
    let mut tmp = [0u8; FRAME_MAX];
    let Some(t) = tmp.get_mut(..k.frame.len()) else {
        return false;
    };
    t.copy_from_slice(k.frame);
    t[eapol::MIC_AT..eapol::MIC_AT + eapol::MIC_LEN].iter_mut().for_each(|b| *b = 0);
    hmac_sha1_verify(kck, &[&*t], &k.mic)
}

impl Supplicant {
    pub const fn new() -> Self {
        Supplicant {
            state: State::Idle,
            own: [0; 6],
            aa: [0; 6],
            pmk: Secret::zero(),
            ap_rsn: [0; RSN_MAX],
            ap_rsn_len: 0,
            own_rsn: [0; RSN_MAX],
            own_rsn_len: 0,
            snonce: Secret::zero(),
            have_snonce: false,
            anonce: [0; 32],
            msg1_replay: None,
            replay_ok: None,
            tptk: Secret::zero(),
            ptk: Secret::zero(),
            ptk_installed: false,
            gtk: [Secret::zero(), Secret::zero(), Secret::zero(), Secret::zero()],
            gtk_set: [false; 4],
            gtk_rsc: [0; 4],
            started_ms: 0,
            reply_version: 1,
            counters: Counters { refused: 0, bad_mic: 0, replays: 0 },
        }
    }

    /// Forget everything; secrets are wiped.
    pub fn reset(&mut self) {
        self.pmk.wipe();
        self.snonce.wipe();
        self.tptk.wipe();
        self.ptk.wipe();
        for g in self.gtk.iter_mut() {
            g.wipe();
        }
        self.gtk_set = [false; 4];
        self.gtk_rsc = [0; 4];
        self.have_snonce = false;
        self.ptk_installed = false;
        self.anonce = [0; 32];
        self.msg1_replay = None;
        self.replay_ok = None;
        self.state = State::Idle;
    }

    /// Begin after association: our address, the AP's, the PMK, the RSN element data from the AP's beacon (no id or
    /// length) and the whole RSN element we sent in our association request.
    pub fn start(&mut self, own: &Mac, aa: &Mac, pmk: &[u8; 32], ap_rsn: &[u8], own_rsn_ie: &[u8], now_ms: u64) -> Result<(), Fail> {
        self.reset();
        if ap_rsn.len() > RSN_MAX || own_rsn_ie.len() > RSN_MAX || own_rsn_ie.len() < 2 {
            self.state = State::Failed;
            return Err(Fail::KeyData);
        }
        self.own = *own;
        self.aa = *aa;
        self.pmk.0 = *pmk;
        self.ap_rsn[..ap_rsn.len()].copy_from_slice(ap_rsn);
        self.ap_rsn_len = ap_rsn.len();
        self.own_rsn[..own_rsn_ie.len()].copy_from_slice(own_rsn_ie);
        self.own_rsn_len = own_rsn_ie.len();
        self.started_ms = now_ms;
        self.state = State::WaitMsg1;
        Ok(())
    }

    fn fail(&mut self, f: Fail) -> Outcome {
        self.reset();
        self.state = State::Failed;
        Outcome { fail: Some(f), ..Outcome::default() }
    }

    /// Call now and then: a handshake that waits too long fails.
    pub fn tick(&mut self, now_ms: u64) -> Option<Fail> {
        if matches!(self.state, State::WaitMsg1 | State::WaitMsg3) && now_ms.saturating_sub(self.started_ms) > TIMEOUT_MS {
            self.fail(Fail::Timeout);
            return Some(Fail::Timeout);
        }
        None
    }

    /// Any key material left (PMK, SNonce, PTKs, GTKs)? False after `reset` and after a failure.
    pub fn holds_secrets(&self) -> bool {
        let any = |b: &[u8]| b.iter().any(|&x| x != 0);
        any(&self.pmk.0) || any(&self.snonce.0) || any(&self.tptk.0) || any(&self.ptk.0) || self.gtk.iter().any(|g| any(&g.0))
    }

    /// The pairwise temporal key, once installed.
    pub fn tk(&self) -> Option<&[u8]> {
        if self.ptk_installed {
            Some(&self.ptk.0[32..48])
        } else {
            None
        }
    }

    /// A group key and the receive sequence counter it started from.
    pub fn gtk(&self, keyid: u8) -> Option<(&[u8; 16], u64)> {
        let i = keyid as usize;
        if i < 4 && self.gtk_set[i] {
            Some((&self.gtk[i].0, self.gtk_rsc[i]))
        } else {
            None
        }
    }

    fn reply(&self, info: u16, replay: u64, nonce: &[u8; 32], data: &[u8], kck: &[u8], out: &mut [u8]) -> Option<usize> {
        let n = eapol::build(self.reply_version, info, 0, replay, nonce, data, out).ok()?;
        let m = mic(kck, &out[..n])?;
        out[eapol::MIC_AT..eapol::MIC_AT + 16].copy_from_slice(&m);
        Some(n)
    }

    /// Handle one EAPOL frame from the AP (the payload of a 0x888E frame from our BSSID). `rng` fills a buffer with
    /// secret random bytes (the kernel passes its DRBG's fill_secret) and says whether it could. A reply, if any, is
    /// written to `out` (at least 256 bytes).
    pub fn on_eapol(&mut self, f: &[u8], now_ms: u64, rng: &mut dyn FnMut(&mut [u8]) -> bool, out: &mut [u8]) -> Outcome {
        let o = self.on_eapol_inner(f, now_ms, rng, out);
        if let Some(r) = o.refused {
            self.counters.refused += 1;
            match r {
                Refused::Mic => self.counters.bad_mic += 1,
                Refused::Replay => self.counters.replays += 1,
                _ => {}
            }
        }
        o
    }

    fn on_eapol_inner(&mut self, f: &[u8], now_ms: u64, rng: &mut dyn FnMut(&mut [u8]) -> bool, out: &mut [u8]) -> Outcome {
        if matches!(self.state, State::Idle | State::Failed) {
            return refused(Refused::State);
        }
        let Ok(k) = eapol::parse(f) else {
            return refused(Refused::Parse);
        };
        if k.frame.len() > FRAME_MAX {
            return refused(Refused::Parse);
        }
        if k.version() != KI_VERSION_2 {
            return refused(Refused::Version);
        }
        if k.has(KI_REQUEST) || k.has(KI_ERROR) {
            return refused(Refused::Kind);
        }
        if k.eapol_version == 1 || k.eapol_version == 2 {
            self.reply_version = k.eapol_version;
        }
        match (k.has(KI_PAIRWISE), k.has(KI_ACK), k.has(KI_MIC), k.has(KI_INSTALL)) {
            (true, true, false, false) => self.msg1(&k, now_ms, rng, out),
            (true, true, true, true) => self.msg3(&k, out),
            (false, true, true, false) => self.group1(&k, out),
            _ => refused(Refused::Kind),
        }
    }

    fn msg1(&mut self, k: &eapol::Key<'_>, now_ms: u64, rng: &mut dyn FnMut(&mut [u8]) -> bool, out: &mut [u8]) -> Outcome {
        // a new 4-way handshake after the keys are in (a PTK rekey) is not supported: refusing it also keeps the
        // installed keys from being reset
        if !matches!(self.state, State::WaitMsg1 | State::WaitMsg3) {
            return refused(Refused::State);
        }
        if self.replay_ok.is_some_and(|r| k.replay <= r) || self.msg1_replay.is_some_and(|r| k.replay < r) {
            return refused(Refused::Replay);
        }
        if !self.have_snonce {
            if !rng(&mut self.snonce.0) {
                return self.fail(Fail::NoRandom);
            }
            self.have_snonce = true;
        }
        self.anonce = k.nonce;
        self.msg1_replay = Some(k.replay);
        let mut t = [0u8; PTK_LEN];
        derive_ptk(&self.pmk.0, &self.aa, &self.own, &self.anonce, &self.snonce.0, &mut t);
        self.tptk.0 = t;
        wipe(&mut t);
        let snonce = self.snonce.0;
        let own_rsn = self.own_rsn;
        let n = self.reply(KI_VERSION_2 | KI_PAIRWISE | KI_MIC, k.replay, &snonce, &own_rsn[..self.own_rsn_len], &self.tptk.0[..16], out);
        let Some(n) = n else {
            return refused(Refused::Parse);
        };
        if self.state == State::WaitMsg1 {
            self.started_ms = now_ms;
        }
        self.state = State::WaitMsg3;
        Outcome { send: Some(n), ..Outcome::default() }
    }

    /// Decrypt and read message 3's or group message 1's key data. Ok((GTK key id, GTK)) or the failure.
    fn key_data(&self, k: &eapol::Key<'_>, kek: &[u8], need_rsn: bool, plain: &mut [u8; KEYDATA_MAX]) -> Result<(u8, Secret<16>), Fail> {
        if !k.has(KI_ENCRYPTED) {
            return Err(Fail::KeyData);
        }
        let d = k.data;
        if d.len() < 24 || d.len() % 8 != 0 || d.len() - 8 > KEYDATA_MAX {
            return Err(Fail::KeyData);
        }
        let kek: &[u8; 16] = kek.try_into().map_err(|_| Fail::KeyData)?;
        let p = &mut plain[..d.len() - 8];
        aes_unwrap(kek, d, p).map_err(|_| Fail::KeyData)?;
        let mut items = [None; 8];
        let n = eapol::items(p, &mut items).map_err(|_| Fail::KeyData)?;
        let mut rsn_seen = 0;
        let mut found = None;
        for it in items.iter().take(n).flatten() {
            match *it {
                Item::Element { id: IE_RSN, data } => {
                    rsn_seen += 1;
                    if rsn_seen == 1 && data != &self.ap_rsn[..self.ap_rsn_len] {
                        return Err(Fail::RsnMismatch);
                    }
                    if rsn_seen > 1 {
                        return Err(Fail::NotCcmp); // a second RSN element picks a pairwise cipher: only CCMP is ours
                    }
                }
                Item::Kde { kind: eapol::KDE_GTK, data } => {
                    let g = eapol::gtk(data).map_err(|_| Fail::KeyData)?;
                    if g.key.len() != 16 {
                        return Err(Fail::NotCcmp);
                    }
                    let mut key = Secret::<16>::zero();
                    key.0.copy_from_slice(g.key);
                    found = Some((g.keyid, key));
                }
                _ => {}
            }
        }
        if need_rsn && rsn_seen == 0 {
            return Err(Fail::RsnMismatch);
        }
        found.ok_or(Fail::KeyData)
    }

    fn msg3(&mut self, k: &eapol::Key<'_>, out: &mut [u8]) -> Outcome {
        let retransmit = match self.state {
            State::WaitMsg3 => false,
            State::Done => true,
            _ => return refused(Refused::State),
        };
        if self.replay_ok.is_some_and(|r| k.replay <= r) || self.msg1_replay.is_some_and(|r| k.replay <= r) {
            return refused(Refused::Replay);
        }
        if k.nonce != self.anonce {
            return refused(Refused::Nonce);
        }
        let ptk = Secret(if retransmit { self.ptk.0 } else { self.tptk.0 });
        if !mic_ok(&ptk.0[..16], k) {
            return refused(Refused::Mic);
        }
        // from here the frame is the AP's own
        self.replay_ok = Some(k.replay);
        if k.key_len != 16 {
            return self.fail(Fail::NotCcmp);
        }
        let mut plain = [0u8; KEYDATA_MAX];
        let r = self.key_data(k, &ptk.0[16..32], true, &mut plain);
        wipe(&mut plain);
        let (keyid, gtk) = match r {
            Ok(x) => x,
            Err(e) => return self.fail(e),
        };
        let zero = [0u8; 32];
        let Some(n) = self.reply(KI_VERSION_2 | KI_PAIRWISE | KI_MIC | KI_SECURE, k.replay, &zero, &[], &ptk.0[..16], out) else {
            return refused(Refused::Parse);
        };
        if retransmit {
            // KRACK: answer again, install nothing again
            return Outcome { send: Some(n), ..Outcome::default() };
        }
        self.ptk.0 = self.tptk.0;
        self.tptk.wipe();
        self.ptk_installed = true;
        let i = keyid as usize;
        self.gtk[i].0 = gtk.0;
        self.gtk_set[i] = true;
        self.gtk_rsc[i] = k.rsc;
        self.state = State::Done;
        Outcome { send: Some(n), install_ptk: true, install_gtk: Some(keyid), ..Outcome::default() }
    }

    fn group1(&mut self, k: &eapol::Key<'_>, out: &mut [u8]) -> Outcome {
        if self.state != State::Done {
            return refused(Refused::State);
        }
        if self.replay_ok.is_some_and(|r| k.replay <= r) {
            return refused(Refused::Replay);
        }
        let ptk = Secret(self.ptk.0);
        if !mic_ok(&ptk.0[..16], k) {
            return refused(Refused::Mic);
        }
        self.replay_ok = Some(k.replay);
        if !k.has(KI_SECURE) {
            return self.fail(Fail::KeyData);
        }
        let mut plain = [0u8; KEYDATA_MAX];
        let r = self.key_data(k, &ptk.0[16..32], false, &mut plain);
        wipe(&mut plain);
        let (keyid, gtk) = match r {
            Ok(x) => x,
            Err(e) => return self.fail(e),
        };
        let zero = [0u8; 32];
        let Some(n) = self.reply(KI_VERSION_2 | KI_MIC | KI_SECURE, k.replay, &zero, &[], &ptk.0[..16], out) else {
            return refused(Refused::Parse);
        };
        let i = keyid as usize;
        if self.gtk_set[i] && self.gtk[i].0 == gtk.0 {
            // KRACK (group): the same key again is not installed again, and its counter is not reset
            return Outcome { send: Some(n), ..Outcome::default() };
        }
        self.gtk[i].0 = gtk.0;
        self.gtk_set[i] = true;
        self.gtk_rsc[i] = k.rsc;
        Outcome { send: Some(n), install_gtk: Some(keyid), ..Outcome::default() }
    }
}

impl Default for Supplicant {
    fn default() -> Self {
        Self::new()
    }
}
