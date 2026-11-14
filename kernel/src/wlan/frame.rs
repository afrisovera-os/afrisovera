//! 802.11 frames (W2 of the Wi-Fi plan): the MAC header, the management frame bodies a station needs, the
//! information-element walk and the RSN element. Parse and build. Written from IEEE 802.11-2020 (clauses 9.2 to
//! 9.4); OpenBSD's net80211 (ISC) was read only to cross-check field order.
//!
//! Everything here reads bytes that ANYONE in radio range can send, before any key exists (the attacker row the
//! security engineer added on 14 Nov 2026). So: slices only, every length checked against the slice, no indexing
//! that can panic, no allocation, and a malformed frame is refused whole (fuzz target `wlan_frame`).
//! Frames are given without the FCS (the driver strips it).
#![allow(dead_code)]

pub type Mac = [u8; 6];
pub const BROADCAST: Mac = [0xFF; 6];

/// Why a frame was refused. No detail beyond this: the caller counts refusals, it never prints frame bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// shorter than its header or fixed fields say
    Short,
    /// an element's length runs past the end, or a field has a length the standard does not allow
    BadLength,
    /// a frame kind or version this station does not handle
    Unsupported,
    /// the output buffer is too small (build only)
    NoRoom,
}

// Frame control: protocol version (2 bits), type (2), subtype (4), then flags.
pub const TYPE_MGMT: u8 = 0;
pub const TYPE_CTRL: u8 = 1;
pub const TYPE_DATA: u8 = 2;

pub const ST_ASSOC_REQ: u8 = 0;
pub const ST_ASSOC_RESP: u8 = 1;
pub const ST_REASSOC_RESP: u8 = 3;
pub const ST_PROBE_REQ: u8 = 4;
pub const ST_PROBE_RESP: u8 = 5;
pub const ST_BEACON: u8 = 8;
pub const ST_DISASSOC: u8 = 10;
pub const ST_AUTH: u8 = 11;
pub const ST_DEAUTH: u8 = 12;
pub const ST_ACTION: u8 = 13;

const FC_TO_DS: u16 = 0x0100;
const FC_FROM_DS: u16 = 0x0200;
const FC_MORE_FRAG: u16 = 0x0400;
const FC_RETRY: u16 = 0x0800;
const FC_PROTECTED: u16 = 0x4000;
const FC_ORDER: u16 = 0x8000;

/// A parsed MAC header and the body after it.
#[derive(Clone, Copy, Debug)]
pub struct Header<'a> {
    pub fc: u16,
    pub ftype: u8,
    pub subtype: u8,
    pub to_ds: bool,
    pub from_ds: bool,
    pub more_frag: bool,
    pub retry: bool,
    pub protected: bool,
    pub addr1: Mac,
    pub addr2: Mac,
    pub addr3: Mac,
    pub addr4: Option<Mac>,
    /// sequence number (12 bits) and fragment number (4 bits)
    pub seq: u16,
    pub frag: u8,
    /// QoS control of a QoS data frame
    pub qos: Option<u16>,
    /// the header's length in bytes (where the body starts)
    pub hdr_len: usize,
    pub body: &'a [u8],
}

fn mac_at(f: &[u8], at: usize) -> Result<Mac, Why> {
    let s = f.get(at..at + 6).ok_or(Why::Short)?;
    let mut m = [0u8; 6];
    m.copy_from_slice(s);
    Ok(m)
}

fn le16(f: &[u8], at: usize) -> Result<u16, Why> {
    let s = f.get(at..at + 2).ok_or(Why::Short)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// The header length a frame control word implies (management and data frames; control frames are not parsed).
pub fn header_len(fc: u16) -> Result<usize, Why> {
    if fc & 0x0003 != 0 {
        return Err(Why::Unsupported); // protocol version other than 0
    }
    let ftype = ((fc >> 2) & 3) as u8;
    let subtype = ((fc >> 4) & 0xF) as u8;
    match ftype {
        TYPE_MGMT => Ok(24 + if fc & FC_ORDER != 0 { 4 } else { 0 }),
        TYPE_DATA => {
            let qos = subtype & 0x8 != 0;
            let mut n = 24;
            if fc & FC_TO_DS != 0 && fc & FC_FROM_DS != 0 {
                n += 6;
            }
            if qos {
                n += 2;
                if fc & FC_ORDER != 0 {
                    n += 4; // HT Control, only in QoS data frames
                }
            }
            Ok(n)
        }
        _ => Err(Why::Unsupported),
    }
}

/// Parse a management or data frame's MAC header (no FCS). Control and extension frames are refused.
pub fn parse_header(f: &[u8]) -> Result<Header<'_>, Why> {
    let fc = le16(f, 0)?;
    let hdr_len = header_len(fc)?;
    if f.len() < hdr_len {
        return Err(Why::Short);
    }
    let ftype = ((fc >> 2) & 3) as u8;
    let subtype = ((fc >> 4) & 0xF) as u8;
    let to_ds = fc & FC_TO_DS != 0;
    let from_ds = fc & FC_FROM_DS != 0;
    let sc = le16(f, 22)?;
    let four = ftype == TYPE_DATA && to_ds && from_ds;
    let qos = if ftype == TYPE_DATA && subtype & 0x8 != 0 { Some(le16(f, if four { 30 } else { 24 })?) } else { None };
    Ok(Header {
        fc,
        ftype,
        subtype,
        to_ds,
        from_ds,
        more_frag: fc & FC_MORE_FRAG != 0,
        retry: fc & FC_RETRY != 0,
        protected: fc & FC_PROTECTED != 0,
        addr1: mac_at(f, 4)?,
        addr2: mac_at(f, 10)?,
        addr3: mac_at(f, 16)?,
        addr4: if four { Some(mac_at(f, 24)?) } else { None },
        seq: sc >> 4,
        frag: (sc & 0xF) as u8,
        qos,
        hdr_len,
        body: f.get(hdr_len..).ok_or(Why::Short)?,
    })
}

// ---- information elements -------------------------------------------------------------------------------------

pub const IE_SSID: u8 = 0;
pub const IE_RATES: u8 = 1;
pub const IE_DS_PARAMS: u8 = 3;
pub const IE_TIM: u8 = 5;
pub const IE_COUNTRY: u8 = 7;
pub const IE_HT_CAPS: u8 = 45;
pub const IE_RSN: u8 = 48;
pub const IE_EXT_RATES: u8 = 50;
pub const IE_HT_OP: u8 = 61;
pub const IE_VENDOR: u8 = 221;
pub const IE_EXTENSION: u8 = 255;

/// One information element: its id, and for id 255 the extension id (the first data byte, not part of `data`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Element<'a> {
    pub id: u8,
    pub ext: Option<u8>,
    pub data: &'a [u8],
}

/// Walks a run of elements. Yields each well-formed element; if the run ends in a truncated element the walk stops
/// and `malformed()` turns true (the caller then refuses the whole frame).
pub struct Elements<'a> {
    rest: &'a [u8],
    bad: bool,
}

pub fn elements(b: &[u8]) -> Elements<'_> {
    Elements { rest: b, bad: false }
}

impl<'a> Elements<'a> {
    pub fn malformed(&self) -> bool {
        self.bad
    }
}

impl<'a> Iterator for Elements<'a> {
    type Item = Element<'a>;
    fn next(&mut self) -> Option<Element<'a>> {
        if self.bad || self.rest.is_empty() {
            return None;
        }
        let (id, len) = match self.rest {
            [id, len, ..] => (*id, *len as usize),
            _ => {
                self.bad = true;
                return None;
            }
        };
        let Some(data) = self.rest.get(2..2 + len) else {
            self.bad = true;
            return None;
        };
        self.rest = self.rest.get(2 + len..).unwrap_or(&[]);
        if id == IE_EXTENSION {
            match data.split_first() {
                Some((e, d)) => return Some(Element { id, ext: Some(*e), data: d }),
                None => {
                    self.bad = true;
                    return None;
                }
            }
        }
        Some(Element { id, ext: None, data })
    }
}

/// True if the whole run is well formed.
pub fn elements_ok(b: &[u8]) -> bool {
    let mut it = elements(b);
    for _ in it.by_ref() {}
    !it.malformed()
}

/// The first element with this id (ignoring extension elements), if the run is well formed up to it.
pub fn find(b: &[u8], id: u8) -> Option<&[u8]> {
    elements(b).find(|e| e.id == id && e.ext.is_none()).map(|e| e.data)
}

// ---- the RSN element (802.11-2020 9.4.2.24) -------------------------------------------------------------------

pub const OUI_IEEE: [u8; 3] = [0x00, 0x0F, 0xAC];
pub const CIPHER_GROUP_NOT_ALLOWED: u8 = 7;
pub const CIPHER_WEP40: u8 = 1;
pub const CIPHER_TKIP: u8 = 2;
pub const CIPHER_CCMP: u8 = 4;
pub const CIPHER_WEP104: u8 = 5;
pub const CIPHER_BIP_CMAC: u8 = 6;
pub const AKM_8021X: u8 = 1;
pub const AKM_PSK: u8 = 2;
pub const AKM_8021X_SHA256: u8 = 5;
pub const AKM_PSK_SHA256: u8 = 6;
pub const AKM_SAE: u8 = 8;
pub const RSN_CAP_MFPR: u16 = 0x0040;
pub const RSN_CAP_MFPC: u16 = 0x0080;

/// A suite selector: OUI and type. Only 00-0F-AC suites mean anything to this station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suite {
    pub oui: [u8; 3],
    pub kind: u8,
}

impl Suite {
    pub const fn ieee(kind: u8) -> Suite {
        Suite { oui: OUI_IEEE, kind }
    }
    pub fn is(&self, kind: u8) -> bool {
        self.oui == OUI_IEEE && self.kind == kind
    }
}

pub const MAX_SUITES: usize = 8;

/// A parsed RSN element. Lists longer than MAX_SUITES are refused (no real AP sends more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rsn {
    pub group: Suite,
    pub pairwise: [Suite; MAX_SUITES],
    pub n_pairwise: usize,
    pub akm: [Suite; MAX_SUITES],
    pub n_akm: usize,
    pub caps: u16,
    /// number of PMKIDs listed (the PMKIDs themselves are not kept)
    pub n_pmkid: usize,
    pub group_mgmt: Option<Suite>,
}

impl Rsn {
    pub fn pairwise(&self) -> &[Suite] {
        &self.pairwise[..self.n_pairwise]
    }
    pub fn akms(&self) -> &[Suite] {
        &self.akm[..self.n_akm]
    }
    pub fn has_pairwise(&self, kind: u8) -> bool {
        self.pairwise().iter().any(|s| s.is(kind))
    }
    pub fn has_akm(&self, kind: u8) -> bool {
        self.akms().iter().any(|s| s.is(kind))
    }
}

fn suite_at(d: &[u8], at: usize) -> Result<Suite, Why> {
    let s = d.get(at..at + 4).ok_or(Why::BadLength)?;
    Ok(Suite { oui: [s[0], s[1], s[2]], kind: s[3] })
}

fn suite_list(d: &[u8], at: &mut usize, out: &mut [Suite; MAX_SUITES]) -> Result<usize, Why> {
    let n = le16(d, *at).map_err(|_| Why::BadLength)? as usize;
    *at += 2;
    if n == 0 || n > MAX_SUITES {
        return Err(Why::BadLength);
    }
    for slot in out.iter_mut().take(n) {
        *slot = suite_at(d, *at)?;
        *at += 4;
    }
    Ok(n)
}

/// Parse an RSN element's data (without id and length). Fields after the group cipher are optional in the
/// standard, with defaults (CCMP pairwise, 802.1X AKM); a field cut off in the middle is refused.
pub fn parse_rsn(d: &[u8]) -> Result<Rsn, Why> {
    if le16(d, 0).map_err(|_| Why::BadLength)? != 1 {
        return Err(Why::Unsupported);
    }
    let mut r = Rsn {
        group: Suite::ieee(CIPHER_CCMP),
        pairwise: [Suite::ieee(0); MAX_SUITES],
        n_pairwise: 1,
        akm: [Suite::ieee(0); MAX_SUITES],
        n_akm: 1,
        caps: 0,
        n_pmkid: 0,
        group_mgmt: None,
    };
    r.pairwise[0] = Suite::ieee(CIPHER_CCMP);
    r.akm[0] = Suite::ieee(AKM_8021X);
    let mut at = 2;
    if d.len() == at {
        return Ok(r);
    }
    r.group = suite_at(d, at)?;
    at += 4;
    if d.len() == at {
        return Ok(r);
    }
    r.n_pairwise = suite_list(d, &mut at, &mut r.pairwise)?;
    if d.len() == at {
        return Ok(r);
    }
    r.n_akm = suite_list(d, &mut at, &mut r.akm)?;
    if d.len() == at {
        return Ok(r);
    }
    r.caps = le16(d, at).map_err(|_| Why::BadLength)?;
    at += 2;
    if d.len() == at {
        return Ok(r);
    }
    let n = le16(d, at).map_err(|_| Why::BadLength)? as usize;
    at += 2;
    let skip = n.checked_mul(16).ok_or(Why::BadLength)?;
    if d.len() < at + skip {
        return Err(Why::BadLength);
    }
    r.n_pmkid = n;
    at += skip;
    if d.len() == at {
        return Ok(r);
    }
    r.group_mgmt = Some(suite_at(d, at)?);
    at += 4;
    // anything after the last known field is allowed by the standard (future extensions) and ignored
    let _ = at;
    Ok(r)
}

/// Write an RSN element (id, length and data) for a station's association request: version 1, the given group
/// and the ONE pairwise and ONE AKM suite chosen, the capabilities, no PMKIDs, no group management cipher.
/// Returns the bytes written.
pub fn build_rsn(group: Suite, pairwise: Suite, akm: Suite, caps: u16, out: &mut [u8]) -> Result<usize, Why> {
    let need = 2 + 2 + 4 + 2 + 4 + 2 + 4 + 2;
    let o = out.get_mut(..need).ok_or(Why::NoRoom)?;
    o[0] = IE_RSN;
    o[1] = (need - 2) as u8;
    o[2..4].copy_from_slice(&1u16.to_le_bytes());
    let mut at = 4;
    for s in [group] {
        o[at..at + 3].copy_from_slice(&s.oui);
        o[at + 3] = s.kind;
        at += 4;
    }
    for s in [pairwise, akm] {
        o[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        o[at + 2..at + 5].copy_from_slice(&s.oui);
        o[at + 5] = s.kind;
        at += 6;
    }
    o[at..at + 2].copy_from_slice(&caps.to_le_bytes());
    Ok(need)
}

// ---- what a beacon or probe response says about a network -----------------------------------------------------

/// Which security a network offers, as this station sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// no encryption at all
    Open,
    /// WPA2-Personal this station can join: CCMP pairwise and group, AKM PSK (2) or PSK-SHA256 (6), and protected
    /// management frames not required. `sha256` is true when AKM 6 is the one to use.
    Wpa2Psk { sha256: bool },
    /// WPA3-Personal only (SAE), or PMF required: not yet (W9)
    Wpa3Only,
    /// 802.1X (enterprise): not supported
    Enterprise,
    /// WEP, WPA1 or TKIP only: old, broken security; never joined
    Old,
    /// an RSN element this station could not read, or suites it does not know
    Unknown,
}

impl Security {
    pub fn word(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Wpa2Psk { .. } => "WPA2",
            Security::Wpa3Only => "WPA3",
            Security::Enterprise => "enterprise",
            Security::Old => "old (WEP/WPA1)",
            Security::Unknown => "unknown",
        }
    }
}

/// Decide the security from the capability's privacy bit and the RSN and WPA1 elements.
pub fn classify(privacy: bool, rsn: Option<Result<Rsn, Why>>, has_wpa1: bool) -> Security {
    match rsn {
        None if !privacy => Security::Open,
        None => {
            // privacy without RSN: WEP, or WPA1 (vendor element); both refused
            let _ = has_wpa1;
            Security::Old
        }
        Some(Err(_)) => Security::Unknown,
        Some(Ok(r)) => {
            let ccmp_group = r.group.is(CIPHER_CCMP);
            let ccmp_pair = r.has_pairwise(CIPHER_CCMP);
            let psk = r.has_akm(AKM_PSK);
            let psk256 = r.has_akm(AKM_PSK_SHA256);
            let mfpr = r.caps & RSN_CAP_MFPR != 0;
            if !ccmp_pair || !ccmp_group {
                if r.has_pairwise(CIPHER_TKIP) || r.group.is(CIPHER_TKIP) || r.group.is(CIPHER_WEP40) || r.group.is(CIPHER_WEP104) {
                    return Security::Old;
                }
                return Security::Unknown;
            }
            if (psk || psk256) && !mfpr {
                return Security::Wpa2Psk { sha256: psk256 && !psk };
            }
            if r.has_akm(AKM_SAE) || ((psk || psk256) && mfpr) {
                return Security::Wpa3Only;
            }
            if r.has_akm(AKM_8021X) || r.has_akm(AKM_8021X_SHA256) {
                return Security::Enterprise;
            }
            Security::Unknown
        }
    }
}

pub const RSN_MAX: usize = 64;
pub const RATES_MAX: usize = 16;

/// A network as one beacon or probe response describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BssInfo {
    pub bssid: Mac,
    pub ssid: [u8; 32],
    pub ssid_len: usize,
    pub channel: u8,
    pub beacon_int: u16,
    pub capability: u16,
    pub security: Security,
    /// the RSN element's data exactly as sent (for the check against message 3 of the handshake); empty if none
    /// or longer than RSN_MAX (then `security` is Unknown and the network is never joined)
    pub rsn: [u8; RSN_MAX],
    pub rsn_len: usize,
    /// the supported-rates and extended-rates element data, joined (echoed in the association request)
    pub rates: [u8; RATES_MAX],
    pub n_rates: usize,
    pub n_basic_rates: usize,
}

impl BssInfo {
    pub fn ssid(&self) -> &[u8] {
        &self.ssid[..self.ssid_len]
    }
    pub fn rsn(&self) -> &[u8] {
        &self.rsn[..self.rsn_len]
    }
    pub fn rates(&self) -> &[u8] {
        &self.rates[..self.n_rates]
    }
    /// A hidden network sends an empty SSID or one of zero bytes.
    pub fn hidden(&self) -> bool {
        self.ssid().iter().all(|&b| b == 0)
    }
}

const CAP_PRIVACY: u16 = 0x0010;
const WPA1_OUI_TYPE: [u8; 4] = [0x00, 0x50, 0xF2, 0x01];

/// Read a beacon or probe response (the whole frame, no FCS). `rx_channel` is the channel the radio was on, used
/// when the frame carries no DS parameter set. Refused if anything in it is malformed.
pub fn parse_beacon(f: &[u8], rx_channel: u8) -> Result<BssInfo, Why> {
    let h = parse_header(f)?;
    if h.ftype != TYPE_MGMT || !(h.subtype == ST_BEACON || h.subtype == ST_PROBE_RESP) {
        return Err(Why::Unsupported);
    }
    let b = h.body;
    if b.len() < 12 {
        return Err(Why::Short);
    }
    let beacon_int = le16(b, 8)?;
    let capability = le16(b, 10)?;
    let ies = b.get(12..).ok_or(Why::Short)?;
    if !elements_ok(ies) {
        return Err(Why::BadLength);
    }
    let mut info = BssInfo {
        bssid: h.addr3,
        ssid: [0; 32],
        ssid_len: 0,
        channel: rx_channel,
        beacon_int,
        capability,
        security: Security::Unknown,
        rsn: [0; RSN_MAX],
        rsn_len: 0,
        rates: [0; RATES_MAX],
        n_rates: 0,
        n_basic_rates: 0,
    };
    let (mut rsn, mut wpa1, mut have_ssid, mut have_ds) = (None, false, false, false);
    for e in elements(ies) {
        match (e.id, e.ext) {
            (IE_SSID, None) if !have_ssid => {
                if e.data.len() > 32 {
                    return Err(Why::BadLength);
                }
                info.ssid[..e.data.len()].copy_from_slice(e.data);
                info.ssid_len = e.data.len();
                have_ssid = true;
            }
            (IE_RATES, None) | (IE_EXT_RATES, None) => {
                for &r in e.data {
                    if info.n_rates < RATES_MAX {
                        info.rates[info.n_rates] = r;
                        info.n_rates += 1;
                        if r & 0x80 != 0 {
                            info.n_basic_rates += 1;
                        }
                    }
                }
            }
            (IE_DS_PARAMS, None) => {
                if e.data.len() != 1 {
                    return Err(Why::BadLength);
                }
                if !have_ds {
                    info.channel = e.data[0]; // the first one counts, as for the SSID and RSN elements
                    have_ds = true;
                }
            }
            (IE_RSN, None) if rsn.is_none() => {
                if e.data.len() <= RSN_MAX {
                    info.rsn[..e.data.len()].copy_from_slice(e.data);
                    info.rsn_len = e.data.len();
                    rsn = Some(parse_rsn(e.data));
                } else {
                    rsn = Some(Err(Why::BadLength));
                }
            }
            (IE_VENDOR, None) if e.data.get(..4) == Some(&WPA1_OUI_TYPE[..]) => wpa1 = true,
            _ => {}
        }
    }
    if !have_ssid {
        return Err(Why::BadLength); // the SSID element is mandatory in both frames
    }
    info.security = classify(capability & CAP_PRIVACY != 0, rsn, wpa1);
    Ok(info)
}

// ---- authentication, association, deauthentication ------------------------------------------------------------

/// Status code 0: success.
pub const STATUS_OK: u16 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auth {
    pub algorithm: u16,
    pub seq: u16,
    pub status: u16,
}

pub fn parse_auth(body: &[u8]) -> Result<Auth, Why> {
    if body.len() < 6 {
        return Err(Why::Short);
    }
    if !elements_ok(&body[6..]) {
        return Err(Why::BadLength);
    }
    Ok(Auth { algorithm: le16(body, 0)?, seq: le16(body, 2)?, status: le16(body, 4)? })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssocResp {
    pub capability: u16,
    pub status: u16,
    /// association id, the two top bits cleared
    pub aid: u16,
}

pub fn parse_assoc_resp(body: &[u8]) -> Result<AssocResp, Why> {
    if body.len() < 6 {
        return Err(Why::Short);
    }
    if !elements_ok(&body[6..]) {
        return Err(Why::BadLength);
    }
    let aid = le16(body, 4)? & 0x3FFF;
    Ok(AssocResp { capability: le16(body, 0)?, status: le16(body, 2)?, aid })
}

/// The reason code of a deauthentication or disassociation.
pub fn parse_reason(body: &[u8]) -> Result<u16, Why> {
    le16(body, 0).map_err(|_| Why::Short)
}

fn put_header(out: &mut [u8], subtype: u8, ftype: u8, flags: u16, a1: &Mac, a2: &Mac, a3: &Mac, seq: u16) -> Result<usize, Why> {
    let o = out.get_mut(..24).ok_or(Why::NoRoom)?;
    let fc: u16 = ((subtype as u16) << 4) | ((ftype as u16) << 2) | flags;
    o[0..2].copy_from_slice(&fc.to_le_bytes());
    o[2..4].copy_from_slice(&0u16.to_le_bytes()); // duration: the radio fills it
    o[4..10].copy_from_slice(a1);
    o[10..16].copy_from_slice(a2);
    o[16..22].copy_from_slice(a3);
    o[22..24].copy_from_slice(&((seq & 0x0FFF) << 4).to_le_bytes());
    Ok(24)
}

fn put(out: &mut [u8], at: usize, bytes: &[u8]) -> Result<usize, Why> {
    let end = at.checked_add(bytes.len()).ok_or(Why::NoRoom)?;
    out.get_mut(at..end).ok_or(Why::NoRoom)?.copy_from_slice(bytes);
    Ok(end)
}

fn put_ie(out: &mut [u8], at: usize, id: u8, data: &[u8]) -> Result<usize, Why> {
    if data.len() > 255 {
        return Err(Why::BadLength);
    }
    let at = put(out, at, &[id, data.len() as u8])?;
    put(out, at, data)
}

/// Open System authentication, request (sequence 1).
pub fn build_auth_open(own: &Mac, bssid: &Mac, seq: u16, out: &mut [u8]) -> Result<usize, Why> {
    let at = put_header(out, ST_AUTH, TYPE_MGMT, 0, bssid, own, bssid, seq)?;
    put(out, at, &[0, 0, 1, 0, 0, 0])
}

/// Deauthentication with a reason code.
pub fn build_deauth(own: &Mac, bssid: &Mac, seq: u16, reason: u16, out: &mut [u8]) -> Result<usize, Why> {
    let at = put_header(out, ST_DEAUTH, TYPE_MGMT, 0, bssid, own, bssid, seq)?;
    put(out, at, &reason.to_le_bytes())
}

/// Association request: our capability, a listen interval, the SSID, the AP's own rates echoed back (split into
/// supported rates, at most 8, and extended rates), and the RSN element when joining a protected network.
pub fn build_assoc_req(own: &Mac, bss: &BssInfo, capability: u16, listen: u16, rsn_ie: &[u8], seq: u16, out: &mut [u8]) -> Result<usize, Why> {
    let mut at = put_header(out, ST_ASSOC_REQ, TYPE_MGMT, 0, &bss.bssid, own, &bss.bssid, seq)?;
    at = put(out, at, &capability.to_le_bytes())?;
    at = put(out, at, &listen.to_le_bytes())?;
    at = put_ie(out, at, IE_SSID, bss.ssid())?;
    let rates = bss.rates();
    let (first, rest) = rates.split_at(rates.len().min(8));
    if !first.is_empty() {
        at = put_ie(out, at, IE_RATES, first)?;
    }
    if !rest.is_empty() {
        at = put_ie(out, at, IE_EXT_RATES, rest)?;
    }
    if !rsn_ie.is_empty() {
        at = put(out, at, rsn_ie)?;
    }
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AP: Mac = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    const ME: Mac = [0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];

    /// A beacon built field by field from 802.11-2020 9.3.3.2 (not captured from any real network).
    pub fn beacon(ssid: &[u8], rsn: Option<&[u8]>, privacy: bool, channel: u8) -> ([u8; 256], usize) {
        let mut f = [0u8; 256];
        let mut at = put_header(&mut f, ST_BEACON, TYPE_MGMT, 0, &BROADCAST, &AP, &AP, 7).unwrap();
        at = put(&mut f, at, &[0; 8]).unwrap(); // timestamp
        at = put(&mut f, at, &100u16.to_le_bytes()).unwrap();
        at = put(&mut f, at, &(0x0001u16 | if privacy { CAP_PRIVACY } else { 0 }).to_le_bytes()).unwrap();
        at = put_ie(&mut f, at, IE_SSID, ssid).unwrap();
        at = put_ie(&mut f, at, IE_RATES, &[0x82, 0x84, 0x8B, 0x96, 0x0C, 0x12, 0x18, 0x24]).unwrap();
        at = put_ie(&mut f, at, IE_DS_PARAMS, &[channel]).unwrap();
        if let Some(r) = rsn {
            at = put_ie(&mut f, at, IE_RSN, r).unwrap();
        }
        at = put_ie(&mut f, at, IE_EXT_RATES, &[0x30, 0x48, 0x60, 0x6C]).unwrap();
        (f, at)
    }

    pub const RSN_WPA2: [u8; 20] = [1, 0, 0x00, 0x0F, 0xAC, 4, 1, 0, 0x00, 0x0F, 0xAC, 4, 1, 0, 0x00, 0x0F, 0xAC, 2, 0x0C, 0];

    #[test]
    fn wpa2_beacon() {
        let (f, n) = beacon(b"home", Some(&RSN_WPA2), true, 6);
        let b = parse_beacon(&f[..n], 1).unwrap();
        assert_eq!(b.ssid(), b"home");
        assert_eq!(b.bssid, AP);
        assert_eq!(b.channel, 6);
        assert_eq!(b.security, Security::Wpa2Psk { sha256: false });
        assert_eq!(b.rsn(), &RSN_WPA2[..]);
        assert_eq!(b.n_rates, 12);
        assert_eq!(b.n_basic_rates, 4);
    }

    #[test]
    fn security_kinds() {
        let (f, n) = beacon(b"cafe", None, false, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Open);
        let (f, n) = beacon(b"wep", None, true, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Old);
        let mut tkip = RSN_WPA2;
        tkip[5] = CIPHER_TKIP;
        tkip[11] = CIPHER_TKIP;
        let (f, n) = beacon(b"tkip", Some(&tkip), true, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Old);
        let mut sae = RSN_WPA2;
        sae[17] = AKM_SAE;
        let (f, n) = beacon(b"wpa3", Some(&sae), true, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Wpa3Only);
        let mut ent = RSN_WPA2;
        ent[17] = AKM_8021X;
        let (f, n) = beacon(b"work", Some(&ent), true, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Enterprise);
        let mut mfpr = RSN_WPA2;
        mfpr[18] |= (RSN_CAP_MFPR | RSN_CAP_MFPC) as u8;
        let (f, n) = beacon(b"pmf", Some(&mfpr), true, 1);
        assert_eq!(parse_beacon(&f[..n], 1).unwrap().security, Security::Wpa3Only);
    }

    #[test]
    fn mixed_wpa2_wpa3_joins_as_wpa2() {
        // AKM list 2 and 8, MFPC set but not MFPR: the transition mode most home routers offer
        let rsn = [1, 0, 0, 0x0F, 0xAC, 4, 1, 0, 0, 0x0F, 0xAC, 4, 2, 0, 0, 0x0F, 0xAC, 2, 0, 0x0F, 0xAC, 8, 0x80, 0];
        let r = parse_rsn(&rsn).unwrap();
        assert_eq!(r.n_akm, 2);
        assert_eq!(classify(true, Some(Ok(r)), false), Security::Wpa2Psk { sha256: false });
    }

    #[test]
    fn truncations_refused() {
        let (f, n) = beacon(b"home", Some(&RSN_WPA2), true, 6);
        // a cut is accepted only at an element boundary after the SSID; any cut inside an element is refused
        let mut ends = [0usize; 16];
        let (mut k, mut at) = (0, 36);
        for e in elements(&f[36..n]) {
            at += 2 + e.data.len() + e.ext.map_or(0, |_| 1);
            ends[k] = at;
            k += 1;
        }
        for cut in 0..=n {
            let boundary = ends[..k].contains(&cut);
            assert_eq!(parse_beacon(&f[..cut], 1).is_ok(), boundary, "cut at {cut}");
        }
        // RSN lists that lie about their count
        let mut bad = RSN_WPA2;
        bad[6] = 9; // pairwise count past MAX_SUITES
        assert_eq!(parse_rsn(&bad), Err(Why::BadLength));
        let mut bad = RSN_WPA2;
        bad[12] = 2; // two AKMs claimed, one present
        assert_eq!(parse_rsn(&bad), Err(Why::BadLength));
        assert_eq!(parse_rsn(&[2, 0]), Err(Why::Unsupported));
        assert_eq!(parse_rsn(&[1]), Err(Why::BadLength));
        // PMKID count that overflows the frame
        let mut p = [0u8; 24];
        p[..20].copy_from_slice(&RSN_WPA2);
        p[20] = 0xFF;
        p[21] = 0xFF;
        assert_eq!(parse_rsn(&p), Err(Why::BadLength));
    }

    #[test]
    fn ssid_too_long_and_missing() {
        let (mut f, n) = beacon(b"x", None, false, 1);
        // patch the SSID length to 33 and give it the bytes: refused
        let mut g = [0u8; 300];
        g[..36].copy_from_slice(&f[..36]);
        g[36] = IE_SSID;
        g[37] = 33;
        assert_eq!(parse_beacon(&g[..38 + 33], 1), Err(Why::BadLength));
        // no SSID element at all
        f[36] = IE_TIM;
        assert_eq!(parse_beacon(&f[..n], 1), Err(Why::BadLength));
    }

    #[test]
    fn rsn_build_parses_back() {
        let mut buf = [0u8; 32];
        let n = build_rsn(Suite::ieee(CIPHER_CCMP), Suite::ieee(CIPHER_CCMP), Suite::ieee(AKM_PSK), 0, &mut buf).unwrap();
        assert_eq!(buf[0], IE_RSN);
        assert_eq!(buf[1] as usize, n - 2);
        assert_eq!(&buf[2..n - 2], &RSN_WPA2[..18]);
        assert_eq!(&buf[n - 2..n], &[0, 0]);
        let r = parse_rsn(&buf[2..n]).unwrap();
        assert!(r.has_akm(AKM_PSK) && r.has_pairwise(CIPHER_CCMP) && r.group.is(CIPHER_CCMP));
        assert_eq!(build_rsn(Suite::ieee(4), Suite::ieee(4), Suite::ieee(2), 0, &mut buf[..10]), Err(Why::NoRoom));
    }

    #[test]
    fn headers() {
        // data frame from the AP (FromDS), QoS, with HT control (Order bit): 24 + 2 + 4
        let fc: u16 = (8 << 4) | ((TYPE_DATA as u16) << 2) | FC_FROM_DS | FC_ORDER;
        assert_eq!(header_len(fc), Ok(30));
        // four-address QoS data: 24 + 6 + 2
        assert_eq!(header_len((8 << 4) | (2 << 2) | FC_TO_DS | FC_FROM_DS), Ok(32));
        // non-QoS data with the Order bit has no HT control
        assert_eq!(header_len((2 << 2) | FC_ORDER), Ok(24));
        assert_eq!(header_len(1), Err(Why::Unsupported)); // version 1
        assert_eq!(header_len((TYPE_CTRL as u16) << 2), Err(Why::Unsupported));
        let mut f = [0u8; 32];
        f[0..2].copy_from_slice(&((8u16 << 4) | (2 << 2) | FC_TO_DS | FC_FROM_DS).to_le_bytes());
        assert_eq!(parse_header(&f[..31]).unwrap_err(), Why::Short);
        let h = parse_header(&f).unwrap();
        assert!(h.addr4.is_some() && h.qos.is_some() && h.body.is_empty());
    }

    #[test]
    fn auth_assoc_deauth() {
        let mut buf = [0u8; 128];
        let n = build_auth_open(&ME, &AP, 3, &mut buf).unwrap();
        let h = parse_header(&buf[..n]).unwrap();
        assert_eq!((h.subtype, h.addr1, h.addr2, h.seq), (ST_AUTH, AP, ME, 3));
        assert_eq!(parse_auth(h.body).unwrap(), Auth { algorithm: 0, seq: 1, status: 0 });
        assert_eq!(parse_auth(&h.body[..5]), Err(Why::Short));
        let n = build_deauth(&ME, &AP, 4, 3, &mut buf).unwrap();
        assert_eq!(parse_reason(parse_header(&buf[..n]).unwrap().body), Ok(3));
        let resp = [0x11, 0x04, 0, 0, 0x05, 0xC0, IE_RATES, 1, 0x82];
        assert_eq!(parse_assoc_resp(&resp).unwrap(), AssocResp { capability: 0x0411, status: 0, aid: 5 });
        assert_eq!(parse_assoc_resp(&resp[..8]), Err(Why::BadLength));
        let (f, fl) = beacon(b"home", Some(&RSN_WPA2), true, 6);
        let bss = parse_beacon(&f[..fl], 1).unwrap();
        let mut rsn = [0u8; 32];
        let rn = build_rsn(Suite::ieee(4), Suite::ieee(4), Suite::ieee(2), 0, &mut rsn).unwrap();
        let n = build_assoc_req(&ME, &bss, 0x0431, 10, &rsn[..rn], 5, &mut buf).unwrap();
        let h = parse_header(&buf[..n]).unwrap();
        assert_eq!(h.subtype, ST_ASSOC_REQ);
        let ies = &h.body[4..];
        assert!(elements_ok(ies));
        assert_eq!(find(ies, IE_SSID), Some(&b"home"[..]));
        assert_eq!(find(ies, IE_RATES).map(|r| r.len()), Some(8));
        assert_eq!(find(ies, IE_EXT_RATES).map(|r| r.len()), Some(4));
        assert_eq!(find(ies, IE_RSN), Some(&rsn[2..rn]));
        assert_eq!(build_assoc_req(&ME, &bss, 0, 10, &rsn[..rn], 5, &mut buf[..40]), Err(Why::NoRoom));
    }

    #[test]
    fn extension_elements() {
        let ies = [IE_EXTENSION, 3, 35, 0xAA, 0xBB, IE_EXTENSION, 0];
        let mut it = elements(&ies);
        assert_eq!(it.next(), Some(Element { id: 255, ext: Some(35), data: &[0xAA, 0xBB] }));
        assert_eq!(it.next(), None);
        assert!(it.malformed(), "an extension element with no extension id is malformed");
    }
}
