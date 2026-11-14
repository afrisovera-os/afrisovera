//! Bluetooth HCI bytes, parsed (Bluetooth slice 1, 14 Nov 2026; BLUETOOTH_PLAN.md, ARCHITECT_REVIEWS 14 Nov).
//!
//! Everything here is plain parsing with no hardware and no `crate::` paths, so fuzz-host compiles this file
//! unchanged (target `bt_event`). Events are untrusted bytes: they come from the air, relayed by the chip. Every
//! length is checked before a field is read; a bad event is dropped and counted, never half used.
//!
//! - `Asm` puts an event back together from the USB interrupt endpoint's packets (an event can span several).
//! - `Event` names the events slice 1 uses; `event` reads one.
//! - `Table` keeps what a scan saw: address, RSSI, kind, and a cleaned name (printable ASCII, at most 32).
//! - `bseq_next` walks an Intel legacy patch file (.bseq) record by record, every length checked against the file.
//! - `masked` prints a BD_ADDR cut to its maker half, as MAC addresses are.
//! Written from the Bluetooth Core specification (Vol 4 Part E: HCI) and Intel's published command list.

/// The largest HCI event: a 2-byte header and 255 bytes of parameters.
pub const EVENT_MAX: usize = 257;
/// Devices a scan keeps.
pub const TABLE_MAX: usize = 32;
/// A remote name, shown cut to this many characters.
pub const NAME_MAX: usize = 32;

/// Puts events together from interrupt-endpoint packets. An event whose packets carry more bytes than its header
/// says, or a packet that starts in the middle of nothing (a lone byte), is dropped and counted.
pub struct Asm {
    buf: [u8; EVENT_MAX],
    len: usize,
    pub events: u64,
    pub dropped: u64,
}

impl Asm {
    pub const fn new() -> Asm {
        Asm { buf: [0; EVENT_MAX], len: 0, events: 0, dropped: 0 }
    }

    /// One packet; `f` gets each whole event (header included). USB ends a transfer at the end of an event, so a
    /// packet never holds the end of one event and the start of the next: bytes past the end of the event a packet
    /// completes make that event untrusted, and it is dropped and counted.
    pub fn packet(&mut self, pkt: &[u8], f: &mut dyn FnMut(&[u8])) {
        for (i, &b) in pkt.iter().enumerate() {
            self.buf[self.len] = b;
            self.len += 1;
            if self.len >= 2 && self.len == 2 + self.buf[1] as usize {
                let whole = self.len;
                self.len = 0;
                if i + 1 != pkt.len() {
                    self.dropped += 1;
                    return;
                }
                self.events += 1;
                f(&self.buf[..whole]);
            }
        }
    }

    fn drop_it(&mut self) {
        self.len = 0;
        self.dropped += 1;
    }

    /// Forget a half-built event (the stream was restarted).
    pub fn reset(&mut self) {
        self.len = 0;
    }
}

/// An event slice 1 uses, its fields read with every length checked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event<'a> {
    /// Command Complete: the command's opcode and its return parameters (status first, for most).
    Complete { opcode: u16, ret: &'a [u8] },
    /// Command Status: the command's opcode and its status.
    Status { opcode: u16, status: u8 },
    /// Inquiry Complete.
    InquiryDone { status: u8 },
    /// Inquiry results (0x02, 0x22) and extended ones (0x2F): `n` responses in `body`, in the event's own layout.
    Inquiry { code: u8, n: u8, body: &'a [u8] },
    /// LE Advertising Report (LE Meta, subevent 0x02): `n` reports in `body`.
    LeAdv { n: u8, body: &'a [u8] },
    /// A vendor event (0xFF), unread.
    Vendor { body: &'a [u8] },
    /// Anything else, unread.
    Other { code: u8 },
}

/// Why an event was not taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bad {
    /// Shorter than its header says, or than its fixed fields.
    Short,
    /// Its header says more than the bytes hold.
    Long,
}

/// One whole event (header included).
pub fn event(e: &[u8]) -> Result<Event<'_>, Bad> {
    if e.len() < 2 {
        return Err(Bad::Short);
    }
    let (code, plen) = (e[0], e[1] as usize);
    if e.len() != 2 + plen {
        return Err(if e.len() < 2 + plen { Bad::Short } else { Bad::Long });
    }
    let p = &e[2..];
    Ok(match code {
        0x0E => {
            if p.len() < 3 {
                return Err(Bad::Short);
            }
            Event::Complete { opcode: u16::from_le_bytes([p[1], p[2]]), ret: &p[3..] }
        }
        0x0F => {
            if p.len() < 4 {
                return Err(Bad::Short);
            }
            Event::Status { opcode: u16::from_le_bytes([p[2], p[3]]), status: p[0] }
        }
        0x01 => Event::InquiryDone { status: *p.first().ok_or(Bad::Short)? },
        0x02 | 0x22 | 0x2F => {
            let n = *p.first().ok_or(Bad::Short)?;
            Event::Inquiry { code, n, body: &p[1..] }
        }
        0x3E => {
            if p.first() != Some(&0x02) {
                return Ok(Event::Other { code });
            }
            let n = *p.get(1).ok_or(Bad::Short)?;
            Event::LeAdv { n, body: &p[2..] }
        }
        0xFF => Event::Vendor { body: p },
        _ => Event::Other { code },
    })
}

/// One device a scan saw.
#[derive(Clone, Copy)]
pub struct Seen {
    /// BD_ADDR as the air gives it (little-endian: `addr[5]` is the first byte written).
    pub addr: [u8; 6],
    /// 0 classic (BR/EDR), 1 LE public, 2 LE random (and the rest).
    pub kind: u8,
    pub rssi: i8,
    /// Class of device (classic) or appearance (LE), 0 if not given.
    pub class: u32,
    pub name: [u8; NAME_MAX],
    pub name_len: usize,
    pub seen: u32,
}

const NONE: Seen = Seen { addr: [0; 6], kind: 0, rssi: 0, class: 0, name: [0; NAME_MAX], name_len: 0, seen: 0 };

/// What a scan saw, at most TABLE_MAX devices; more are counted, not kept.
pub struct Table {
    pub dev: [Seen; TABLE_MAX],
    pub n: usize,
    pub full: u64,
    /// Reports whose own lengths did not fit their event, dropped.
    pub bad: u64,
}

impl Table {
    pub const fn new() -> Table {
        Table { dev: [NONE; TABLE_MAX], n: 0, full: 0, bad: 0 }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.full = 0;
        self.bad = 0;
    }

    /// The entry for `addr`/`kind`, made if there is room.
    fn slot(&mut self, addr: [u8; 6], kind: u8) -> Option<&mut Seen> {
        let n = self.n.min(TABLE_MAX);
        let at = self.dev[..n].iter().position(|d| d.addr == addr && (d.kind == 0) == (kind == 0));
        let i = match at {
            Some(i) => i,
            None if n < TABLE_MAX => {
                self.dev[n] = Seen { addr, kind, ..NONE };
                self.n = n + 1;
                n
            }
            None => {
                self.full += 1;
                return None;
            }
        };
        let d = &mut self.dev[i];
        d.seen = d.seen.saturating_add(1);
        Some(d)
    }

    /// An event the scan cares about (inquiry results, LE advertising reports); others are ignored.
    pub fn take(&mut self, ev: &Event) {
        match *ev {
            Event::Inquiry { code, n, body } => self.inquiry(code, n, body),
            Event::LeAdv { n, body } => self.le(n, body),
            _ => {}
        }
    }

    /// Inquiry Result (0x02: 14 bytes each), with RSSI (0x22: 14 bytes each), extended (0x2F: one response, 14
    /// bytes then 240 bytes of EIR). Fields are in arrays per field for 0x02 and 0x22 (Core Vol 4 E 7.7.2/7.7.33).
    fn inquiry(&mut self, code: u8, n: u8, body: &[u8]) {
        let n = n as usize;
        match code {
            0x2F => {
                if n != 1 || body.len() < 14 + 240 {
                    self.bad += 1;
                    return;
                }
                let mut addr = [0u8; 6];
                addr.copy_from_slice(&body[0..6]);
                let class = u32::from_le_bytes([body[8], body[9], body[10], 0]);
                let rssi = body[13] as i8;
                let eir = &body[14..14 + 240];
                if let Some(d) = self.slot(addr, 0) {
                    d.rssi = rssi;
                    d.class = class;
                    let mut name = (0usize, [0u8; NAME_MAX]);
                    ad_walk(eir, &mut |t, v| {
                        if matches!(t, 0x08 | 0x09) && (name.0 == 0 || t == 0x09) {
                            name.0 = clean(v, &mut name.1);
                        }
                    });
                    if name.0 > 0 {
                        (d.name, d.name_len) = (name.1, name.0);
                    }
                }
            }
            0x02 | 0x22 => {
                // per-field arrays: addr 6n, page scan rep mode n, (0x02: reserved 2n), class 3n, clock 2n, (0x22: rssi n)
                let need = n * 14; // 14 bytes a response in both layouts
                if n == 0 || body.len() != need {
                    self.bad += 1;
                    return;
                }
                for k in 0..n {
                    let mut addr = [0u8; 6];
                    addr.copy_from_slice(&body[6 * k..6 * k + 6]);
                    let cls_at = if code == 0x02 { 6 * n + n + 2 * n + 3 * k } else { 6 * n + n + n + 3 * k };
                    let class = u32::from_le_bytes([body[cls_at], body[cls_at + 1], body[cls_at + 2], 0]);
                    let rssi = if code == 0x22 { body[6 * n + n + n + 3 * n + 2 * n + k] as i8 } else { 0 };
                    if let Some(d) = self.slot(addr, 0) {
                        d.class = class;
                        if code == 0x22 {
                            d.rssi = rssi;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// LE Advertising Report (Core Vol 4 E 7.7.65.2), per-field arrays: event type n, address type n, address 6n,
    /// data length n, data (the sum of the lengths), RSSI n.
    fn le(&mut self, n: u8, body: &[u8]) {
        let n = n as usize;
        if n == 0 || n > 25 || body.len() < n * 10 {
            self.bad += 1;
            return;
        }
        let lens_at = n + n + 6 * n;
        let total: usize = body[lens_at..lens_at + n].iter().map(|&l| l as usize).sum();
        if body.len() != lens_at + n + total + n {
            self.bad += 1;
            return;
        }
        let mut data_at = lens_at + n;
        let rssi_at = lens_at + n + total;
        for k in 0..n {
            let mut addr = [0u8; 6];
            addr.copy_from_slice(&body[n + n + 6 * k..n + n + 6 * k + 6]);
            let kind = if body[n + k] & 1 == 0 { 1 } else { 2 };
            let len = body[lens_at + k] as usize;
            let data = &body[data_at..data_at + len];
            data_at += len;
            let rssi = body[rssi_at + k] as i8;
            if let Some(d) = self.slot(addr, kind) {
                d.rssi = rssi;
                let mut name = (0usize, [0u8; NAME_MAX]);
                let mut app = None;
                ad_walk(data, &mut |t, v| match t {
                    0x08 | 0x09 if name.0 == 0 || t == 0x09 => name.0 = clean(v, &mut name.1),
                    0x19 if v.len() == 2 => app = Some(u16::from_le_bytes([v[0], v[1]]) as u32),
                    _ => {}
                });
                if name.0 > 0 {
                    (d.name, d.name_len) = (name.1, name.0);
                }
                if let Some(a) = app {
                    d.class = a;
                }
            }
        }
    }
}

/// Advertising data or EIR: length, type, value, repeated; a length of 0 ends it, and a length past the end
/// stops the walk (what came before is kept).
pub fn ad_walk(data: &[u8], f: &mut dyn FnMut(u8, &[u8])) {
    let mut i = 0usize;
    while i < data.len() {
        let len = data[i] as usize;
        if len == 0 || i + 1 + len > data.len() {
            return;
        }
        f(data[i + 1], &data[i + 2..i + 1 + len]);
        i += 1 + len;
    }
}

/// A remote name for the screen: printable ASCII kept, every other byte a '?', at most NAME_MAX; its length.
pub fn clean(v: &[u8], out: &mut [u8; NAME_MAX]) -> usize {
    let n = v.iter().position(|&c| c == 0).unwrap_or(v.len()).min(NAME_MAX);
    for i in 0..n {
        out[i] = if (0x20..0x7F).contains(&v[i]) { v[i] } else { b'?' };
    }
    n
}

/// A BD_ADDR (little-endian as sent) shown with its maker half only: "aa:bb:cc:--:--:--".
pub fn masked(addr: &[u8; 6], out: &mut [u8; 17]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (k, &b) in [addr[5], addr[4], addr[3]].iter().enumerate() {
        out[3 * k] = HEX[(b >> 4) as usize];
        out[3 * k + 1] = HEX[(b & 15) as usize];
        out[3 * k + 2] = b':';
    }
    out[9..17].copy_from_slice(b"--:--:--");
}

/// One record of an Intel legacy patch file (.bseq): a command to send, then the event(s) it must answer with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rec<'a> {
    /// 0x01: an HCI command packet (opcode, length, parameters), whole.
    Command(&'a [u8]),
    /// 0x02: the HCI event the command before must answer with (code, length, parameters), whole.
    Event(&'a [u8]),
}

/// The record at `at` and where the next one starts; None at the end; Err if a length runs past the file or a
/// record type is unknown.
pub fn bseq_next(file: &[u8], at: usize) -> Result<Option<(Rec<'_>, usize)>, &'static str> {
    if at >= file.len() {
        return Ok(None);
    }
    match file[at] {
        0x01 => {
            if at + 4 > file.len() {
                return Err("a command record is cut short");
            }
            let end = at + 4 + file[at + 3] as usize;
            if end > file.len() {
                return Err("a command record runs past the end of the file");
            }
            Ok(Some((Rec::Command(&file[at + 1..end]), end)))
        }
        0x02 => {
            if at + 3 > file.len() {
                return Err("an event record is cut short");
            }
            let end = at + 3 + file[at + 2] as usize;
            if end > file.len() {
                return Err("an event record runs past the end of the file");
            }
            Ok(Some((Rec::Event(&file[at + 1..end]), end)))
        }
        _ => Err("a record of an unknown type"),
    }
}

/// The whole .bseq file checked before anything is sent: records in the order command, then one or more events;
/// it starts with a command and ends after an event. The number of commands, or why not.
pub fn bseq_check(file: &[u8]) -> Result<usize, &'static str> {
    let (mut at, mut cmds, mut last_was_cmd) = (0usize, 0usize, false);
    let mut first = true;
    while let Some((r, next)) = bseq_next(file, at)? {
        match r {
            Rec::Command(c) => {
                if last_was_cmd {
                    return Err("a command with no event after it");
                }
                if c.len() < 3 {
                    return Err("a command record is cut short");
                }
                cmds += 1;
                last_was_cmd = true;
            }
            Rec::Event(_) => {
                if first {
                    return Err("the file starts with an event");
                }
                last_was_cmd = false;
            }
        }
        first = false;
        at = next;
    }
    if cmds == 0 {
        return Err("the file holds no command");
    }
    if last_was_cmd {
        return Err("the last command has no event after it");
    }
    Ok(cmds)
}
