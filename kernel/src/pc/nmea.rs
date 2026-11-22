//! NMEA 0183 sentences from a GPS receiver (sensors S1: SENSORS_PLAN.md, the architect's OK TO BUILD and the security
//! engineer's rules of 21 Nov 2026).
//!
//! A USB GPS receiver is, for the PC, a serial line that prints one line of text per sentence: `$`, an address (two
//! letters for the talker, three for the sentence), comma separated fields, `*`, two hex digits of checksum, CR LF.
//! The standard is NMEA 0183 (National Marine Electronics Association, version 4.11): a sentence is at most 82
//! characters from `$` to CR LF, and the checksum is the exclusive or of every character between `$` and `*`. Four
//! sentences are read:
//! - GGA (time, position, fix quality, satellites in use, HDOP, altitude above mean sea level);
//! - RMC (time, status A or V, position, speed over ground in knots, course over ground in degrees true, date);
//! - GSA (fix mode 1 none, 2 two-dimensional, 3 three-dimensional; the satellites used; PDOP, HDOP, VDOP);
//! - GSV (satellites in view).
//! Proprietary sentences (`$P...`) and other sentence types are counted and left alone.
//!
//! Rules (the architect's and the security engineer's):
//! - the checksum is required: a sentence without one, or with a wrong one, is refused and counted, never read;
//! - more than 82 characters is refused, and the line buffer never grows past it;
//! - every number is parsed in fixed point with its range checked (no floats, no wrap): degrees times 10^7,
//!   millimetres, millimetres per second, hundredths of a degree, milliseconds;
//! - a receiver with no fix gives "no fix", never zeros dressed up as a place (`Fix` keeps the position only while
//!   the receiver says it has one);
//! - the time of day and the date come only from RMC (t_wall only when RMC gave a date, the architect's units).
//!
//! `Link` also decides whether a USB serial cable is a GPS at all (xhci.rs drives FTDI cables as a console): a line
//! that starts with `$` is held back from the shell until it ends; if it is a good GGA, RMC, GSA or GSV sentence the
//! cable is a GPS from then on, and nothing it sends reaches the shell again. Any other line is handed to the shell
//! byte for byte, in order, so a person typing on the cable loses nothing.
//!
//! Pure: no hardware, no allocation, no printing (a reading never reaches a log from here). Included unchanged by
//! fuzz-host (`nmea` target, and its host tests).

/// The longest sentence, `$` through CR LF (NMEA 0183).
pub const MAX: usize = 82;
/// The longest sentence without its CR LF: what `parse` takes and the line buffer holds.
pub const BODY: usize = MAX - 2;
/// The receiver's assumed range error for one unit of HDOP, in millimetres: the accuracy shown is HDOP times this,
/// marked estimated (NMEA's GGA and GSA give dilutions, not a measured error).
pub const UERE_MM: u32 = 5_000;

/// Why a sentence was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// More than 82 characters.
    TooLong,
    /// Not `$`, a character outside printable ASCII, or a second `$` or `*`.
    Framing,
    /// No `*` with two hex digits at the end.
    NoChecksum,
    /// The checksum does not match.
    Checksum,
    /// A field out of its range or not a number where one is needed.
    Field,
}

/// A place: degrees times 10^7, north and east positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pos {
    pub lat_e7: i32,
    pub lon_e7: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gga {
    pub time_ms: Option<u32>,
    pub pos: Option<Pos>,
    /// 0 no fix; 1 GPS; 2 differential; 3 to 8 other kinds of fix.
    pub quality: u8,
    pub sats: Option<u8>,
    /// HDOP in hundredths.
    pub hdop_c: Option<u16>,
    pub alt_mm: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rmc {
    pub time_ms: Option<u32>,
    /// Status A: the data is valid.
    pub valid: bool,
    pub pos: Option<Pos>,
    pub speed_mms: Option<u32>,
    /// Course over ground in hundredths of a degree, 0 to 35999.
    pub course_cdeg: Option<u16>,
    /// Year (1980 to 2079), month, day.
    pub date: Option<(u16, u8, u8)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gsa {
    /// 1 no fix, 2 two-dimensional, 3 three-dimensional.
    pub mode: u8,
    /// Satellites listed as used (up to 12).
    pub used: u8,
    pub hdop_c: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sentence {
    Gga(Gga),
    Rmc(Rmc),
    Gsa(Gsa),
    /// Satellites in view.
    Gsv(u8),
    /// A good sentence of another type, or a proprietary one.
    Other,
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

fn digits(f: &[u8]) -> bool {
    f.iter().all(u8::is_ascii_digit)
}

/// An unsigned decimal ("123.4567"): the value times 10^`frac`, the digits past `frac` dropped. None when empty,
/// when the whole part is empty or longer than 9 digits, or when anything is not a digit.
fn udec(f: &[u8], frac: usize) -> Option<u64> {
    let (int, rest) = match f.iter().position(|&c| c == b'.') {
        Some(i) => (&f[..i], &f[i + 1..]),
        None => (f, &f[f.len()..]),
    };
    if int.is_empty() || int.len() > 9 || !digits(int) || !digits(rest) {
        return None;
    }
    let mut v: u64 = 0;
    for &c in int {
        v = v * 10 + (c - b'0') as u64;
    }
    for k in 0..frac {
        v = v * 10 + rest.get(k).map_or(0, |&c| (c - b'0') as u64);
    }
    Some(v)
}

/// A field that may be empty: Ok(None) when empty, Err when present but not what `f` accepts.
fn opt<T>(field: &[u8], f: impl FnOnce(&[u8]) -> Option<T>) -> Result<Option<T>, Bad> {
    if field.is_empty() {
        Ok(None)
    } else {
        f(field).map(Some).ok_or(Bad::Field)
    }
}

/// hhmmss with an optional fraction: milliseconds since midnight.
fn time(f: &[u8]) -> Option<u32> {
    let int_len = f.iter().position(|&c| c == b'.').unwrap_or(f.len());
    if int_len != 6 {
        return None;
    }
    let ms = udec(f, 3)?; // hhmmss times 1000
    let (hms, frac) = (ms / 1000, (ms % 1000) as u32);
    let (h, m, s) = ((hms / 10_000) as u32, (hms / 100 % 100) as u32, (hms % 100) as u32);
    if h > 23 || m > 59 || s > 60 {
        return None; // 60: a leap second
    }
    Some(((h * 60 + m) * 60 + s) * 1000 + frac)
}

/// ddmmyy: the year read as 1980 to 2079 (GPS time starts in 1980).
fn date(f: &[u8]) -> Option<(u16, u8, u8)> {
    if f.len() != 6 || !digits(f) {
        return None;
    }
    let n = |i: usize| (f[i] - b'0') * 10 + (f[i + 1] - b'0');
    let (d, m, yy) = (n(0), n(2), n(4) as u16);
    let y = if yy >= 80 { 1900 + yy } else { 2000 + yy };
    if m == 0 || m > 12 || d == 0 || d > month_days(y, m) {
        return None;
    }
    Some((y, m, d))
}

fn month_days(y: u16, m: u8) -> u8 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// One coordinate: `deg` whole-degree digits (2 for latitude, 3 for longitude), then minutes with an optional
/// fraction, and the hemisphere letter. Degrees times 10^7, rounded to the nearest.
fn coord(f: &[u8], deg: usize, hemi: &[u8], pos: u8, neg: u8, limit: i64) -> Option<i32> {
    let int_len = f.iter().position(|&c| c == b'.').unwrap_or(f.len());
    if int_len != deg + 2 {
        return None;
    }
    let all = udec(f, 7)?; // dddmm.mmmmmmm times 10^7
    let (d, min_e7) = (all / 1_000_000_000, all % 1_000_000_000);
    if min_e7 >= 600_000_000 {
        return None; // 60 minutes or more
    }
    let v = (d * 10_000_000 + (min_e7 + 30) / 60) as i64;
    if v > limit {
        return None;
    }
    match hemi {
        [c] if *c == pos => Some(v as i32),
        [c] if *c == neg => Some(-v as i32),
        _ => None,
    }
}

/// A position from four fields: all empty is no position; all present must all parse.
fn position(f: &[&[u8]]) -> Result<Option<Pos>, Bad> {
    if f.iter().all(|x| x.is_empty()) {
        return Ok(None);
    }
    let lat = coord(f[0], 2, f[1], b'N', b'S', 900_000_000).ok_or(Bad::Field)?;
    let lon = coord(f[2], 3, f[3], b'E', b'W', 1_800_000_000).ok_or(Bad::Field)?;
    Ok(Some(Pos { lat_e7: lat, lon_e7: lon }))
}

/// HDOP and the other dilutions: hundredths, at most 655.35.
fn dop(f: &[u8]) -> Option<u16> {
    udec(f, 2).and_then(|v| u16::try_from(v).ok())
}

/// A small count: at most two digits.
fn count(f: &[u8]) -> Option<u8> {
    (f.len() <= 2 && digits(f)).then(|| udec(f, 0)).flatten().map(|v| v as u8)
}

/// Altitude in metres, maybe negative: millimetres, within 100 km of sea level.
fn alt(f: &[u8]) -> Option<i32> {
    let (neg, f) = match f.first() {
        Some(b'-') => (true, &f[1..]),
        _ => (false, f),
    };
    let v = udec(f, 3)?;
    if v > 100_000_000 {
        return None;
    }
    Some(if neg { -(v as i32) } else { v as i32 })
}

const NF: usize = 24; // the most fields read: GSV has 21 with its signal id

/// One sentence, `$` through the two checksum digits, without its CR LF.
pub fn parse(s: &[u8]) -> Result<Sentence, Bad> {
    if s.len() > BODY {
        return Err(Bad::TooLong);
    }
    if s.first() != Some(&b'$') || s.iter().any(|&c| !(0x20..=0x7E).contains(&c)) {
        return Err(Bad::Framing);
    }
    let star = s.iter().rposition(|&c| c == b'*').ok_or(Bad::NoChecksum)?;
    if s.len() != star + 3 {
        return Err(Bad::NoChecksum);
    }
    let want = match (hex(s[star + 1]), hex(s[star + 2])) {
        (Some(h), Some(l)) => h << 4 | l,
        _ => return Err(Bad::NoChecksum),
    };
    let body = &s[1..star];
    if body.iter().any(|&c| c == b'$' || c == b'*') {
        return Err(Bad::Framing);
    }
    if body.iter().fold(0u8, |a, &c| a ^ c) != want {
        return Err(Bad::Checksum);
    }
    let mut v: [&[u8]; NF] = [&[]; NF];
    let mut n = 0;
    for x in body.split(|&c| c == b',') {
        if n == NF {
            return Err(Bad::Field);
        }
        v[n] = x;
        n += 1;
    }
    let addr = v[0];
    if addr.first() == Some(&b'P') {
        return Ok(Sentence::Other); // proprietary
    }
    if addr.len() != 5 || !addr.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
        return Err(Bad::Field);
    }
    let need = |k: usize| if n >= k { Ok(()) } else { Err(Bad::Field) };
    match &addr[2..] {
        b"GGA" => {
            need(10)?;
            let quality = match v[6] {
                [c @ b'0'..=b'8'] => c - b'0',
                _ => return Err(Bad::Field),
            };
            let alt_mm = opt(v[9], alt)?;
            if alt_mm.is_some() && n > 10 && v[10] != b"M" {
                return Err(Bad::Field); // the altitude's unit is metres
            }
            Ok(Sentence::Gga(Gga {
                time_ms: opt(v[1], time)?,
                pos: position(&v[2..6])?,
                quality,
                sats: opt(v[7], count)?,
                hdop_c: opt(v[8], dop)?,
                alt_mm,
            }))
        }
        b"RMC" => {
            need(10)?;
            let valid = match v[2] {
                b"A" => true,
                b"V" => false,
                _ => return Err(Bad::Field),
            };
            let speed_mms = opt(v[7], |f| {
                let k3 = udec(f, 3)?; // knots times 1000; 1 knot is 1852 m an hour
                (k3 <= 10_000_000).then(|| ((k3 * 1852 + 1800) / 3600) as u32)
            })?;
            let course_cdeg = opt(v[8], |f| udec(f, 2).filter(|&c| c < 36_000).map(|c| c as u16))?;
            Ok(Sentence::Rmc(Rmc {
                time_ms: opt(v[1], time)?,
                valid,
                pos: position(&v[3..7])?,
                speed_mms,
                course_cdeg,
                date: opt(v[9], date)?,
            }))
        }
        b"GSA" => {
            need(18)?;
            let mode = match v[2] {
                [c @ b'1'..=b'3'] => c - b'0',
                _ => return Err(Bad::Field),
            };
            let mut used = 0;
            for id in &v[3..15] {
                if !id.is_empty() {
                    opt(id, |f| (f.len() <= 3 && digits(f)).then_some(()))?;
                    used += 1;
                }
            }
            opt(v[15], dop)?;
            let hdop_c = opt(v[16], dop)?;
            opt(v[17], dop)?;
            Ok(Sentence::Gsa(Gsa { mode, used, hdop_c }))
        }
        b"GSV" => {
            need(4)?;
            Ok(Sentence::Gsv(count(v[3]).ok_or(Bad::Field)?))
        }
        _ => Ok(Sentence::Other),
    }
}

/// Days from 1970-01-01 to a date in the proleptic Gregorian calendar (H. Hinnant's days_from_civil).
pub fn days(y: u16, m: u8, d: u8) -> i64 {
    let y = y as i64 - if m <= 2 { 1 } else { 0 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    None,
    D2,
    D3,
}

/// What the receiver says now, put together from its sentences. While `kind` is None there is no position, altitude,
/// speed or course: never zeros dressed up as a place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fix {
    pub kind: Kind,
    pub pos: Option<Pos>,
    pub alt_mm: Option<i32>,
    pub speed_mms: Option<u32>,
    pub course_cdeg: Option<u16>,
    pub sats_used: Option<u8>,
    pub in_view: Option<u8>,
    pub hdop_c: Option<u16>,
    /// UTC in milliseconds since 1970, from RMC's date and time only.
    pub t_wall_ms: Option<u64>,
    gsa_mode: u8,
}

impl Fix {
    pub const fn new() -> Self {
        Fix {
            kind: Kind::None,
            pos: None,
            alt_mm: None,
            speed_mms: None,
            course_cdeg: None,
            sats_used: None,
            in_view: None,
            hdop_c: None,
            t_wall_ms: None,
            gsa_mode: 0,
        }
    }

    /// The horizontal accuracy in millimetres, estimated from HDOP (`UERE_MM` per unit), while there is a fix.
    pub fn accuracy_mm(&self) -> Option<u32> {
        match (self.kind, self.hdop_c) {
            (Kind::None, _) => None,
            (_, h) => h.map(|h| h as u32 * UERE_MM / 100),
        }
    }

    fn kind_with_pos(&self, alt: bool) -> Kind {
        match self.gsa_mode {
            2 => Kind::D2,
            3 => Kind::D3,
            _ if alt => Kind::D3,
            _ => Kind::D2,
        }
    }

    pub fn feed(&mut self, s: &Sentence) {
        match *s {
            Sentence::Gga(g) => {
                self.sats_used = g.sats;
                if g.hdop_c.is_some() {
                    self.hdop_c = g.hdop_c;
                }
                match g.pos {
                    Some(p) if g.quality != 0 => {
                        self.pos = Some(p);
                        self.alt_mm = g.alt_mm;
                        self.kind = self.kind_with_pos(g.alt_mm.is_some());
                    }
                    _ => self.kind = Kind::None,
                }
            }
            Sentence::Rmc(r) => {
                self.t_wall_ms = match (r.date, r.time_ms) {
                    (Some((y, m, d)), Some(t)) => Some((days(y, m, d) * 86_400_000 + t as i64) as u64),
                    _ => None,
                };
                match r.pos {
                    Some(p) if r.valid => {
                        self.pos = Some(p);
                        self.speed_mms = r.speed_mms;
                        self.course_cdeg = r.course_cdeg;
                        if self.kind == Kind::None {
                            self.kind = self.kind_with_pos(self.alt_mm.is_some());
                        }
                    }
                    _ => self.kind = Kind::None,
                }
            }
            Sentence::Gsa(a) => {
                self.gsa_mode = a.mode;
                if self.hdop_c.is_none() {
                    self.hdop_c = a.hdop_c;
                }
                self.kind = match (a.mode, self.pos) {
                    (2, Some(_)) => Kind::D2,
                    (3, Some(_)) => Kind::D3,
                    _ => Kind::None,
                };
            }
            Sentence::Gsv(n) => self.in_view = Some(n),
            Sentence::Other => {}
        }
        if self.kind == Kind::None || self.pos.is_none() {
            self.kind = Kind::None;
            self.pos = None;
            self.alt_mm = None;
            self.speed_mms = None;
            self.course_cdeg = None;
        }
    }
}

/// What the cable's sentences came to, counted (never their bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub good: u32,
    pub other: u32,
    pub checksum: u32,
    pub too_long: u32,
    pub malformed: u32,
}

/// A USB serial cable's bytes: a console until a good GGA, RMC, GSA or GSV sentence arrives, then a GPS.
pub struct Link {
    pub gps: bool,
    line: [u8; BODY],
    n: usize,
    holding: bool,
    at_start: bool,
    over: bool,
    pub fix: Fix,
    pub counts: Counts,
}

fn known(s: &Sentence) -> bool {
    !matches!(s, Sentence::Other)
}

impl Link {
    pub const fn new() -> Self {
        Link {
            gps: false,
            line: [0; BODY],
            n: 0,
            holding: false,
            at_start: true,
            over: false,
            fix: Fix::new(),
            counts: Counts { good: 0, other: 0, checksum: 0, too_long: 0, malformed: 0 },
        }
    }

    /// A new cable: a console again, nothing remembered.
    pub fn reset(&mut self) {
        *self = Link::new();
    }

    /// One byte from the cable. In console mode each byte meant for the shell goes to `console`, in the order it came.
    /// Returns true when this byte made the cable a GPS.
    pub fn byte(&mut self, b: u8, console: &mut impl FnMut(u8)) -> bool {
        if self.gps {
            self.gps_byte(b);
            return false;
        }
        if !self.holding {
            if self.at_start && b == b'$' {
                self.holding = true;
                self.line[0] = b;
                self.n = 1;
            } else {
                console(b);
                self.at_start = b == b'\r' || b == b'\n';
            }
            return false;
        }
        if b == b'\r' || b == b'\n' {
            let held = &self.line[..self.n];
            self.holding = false;
            self.at_start = true;
            if let Ok(s) = parse(held) {
                if known(&s) {
                    self.gps = true;
                    self.counts.good += 1;
                    self.fix.feed(&s);
                    self.n = 0;
                    return true;
                }
            }
            for &c in &self.line[..self.n] {
                console(c);
            }
            console(b);
            self.n = 0;
            return false;
        }
        if self.n == BODY || !(0x20..=0x7E).contains(&b) {
            // too long for a sentence, or an editing key: the person's line, handed over as it came
            for &c in &self.line[..self.n] {
                console(c);
            }
            console(b);
            self.holding = false;
            self.at_start = false;
            self.n = 0;
            return false;
        }
        self.line[self.n] = b;
        self.n += 1;
        false
    }

    fn gps_byte(&mut self, b: u8) {
        match b {
            b'$' => {
                if self.n > 0 {
                    self.counts.malformed += 1; // a sentence cut off by the next one
                }
                self.line[0] = b;
                self.n = 1;
                self.over = false;
            }
            b'\r' | b'\n' => {
                if self.n > 0 {
                    if self.over {
                        self.counts.too_long += 1;
                    } else {
                        match parse(&self.line[..self.n]) {
                            Ok(Sentence::Other) => self.counts.other += 1,
                            Ok(s) => {
                                self.counts.good += 1;
                                self.fix.feed(&s);
                            }
                            Err(Bad::Checksum) => self.counts.checksum += 1,
                            Err(Bad::TooLong) => self.counts.too_long += 1,
                            Err(_) => self.counts.malformed += 1,
                        }
                    }
                }
                self.n = 0;
                self.over = false;
            }
            _ if self.n > 0 => {
                if self.n < BODY {
                    self.line[self.n] = b;
                    self.n += 1;
                } else {
                    self.over = true;
                }
            }
            _ => {} // between sentences: nothing to read
        }
    }
}
