//! Dira's HTTP/1.1 (BROWSER_PLAN.md section 4): the request, and a streaming reader of the answer that hands the
//! body on as it arrives (after chunked decoding), so a page of any size costs no more memory than its headers.
//!
//! Pure code: no kernel calls, no `unsafe`, no allocation (host tests and fuzzing in tools/dira-host).
#![forbid(unsafe_code)]

use super::url::{Buf, Url, PATH_MAX};

pub const HDR_MAX: usize = 4096;
/// The longest answer Dira reads, headers and body together: past it the page stops with a line saying so.
pub const RAW_MAX: u64 = 4 * 1024 * 1024;
/// The longest chunk-size line (the hex size and any extension) Dira reads.
const CHUNK_LINE_MAX: u8 = 64;

/// What kind of body came.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Other, // first, so a fresh `Resp` is all zero bytes and its static sits in .bss
    Html,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Headers,
    Body,      // Content-Length, or read to close
    ChunkSize, // the hex size line
    ChunkExt,  // the rest of the size line, to its LF
    ChunkData,
    ChunkEnd, // the CRLF after a chunk's data
    Done,
}

/// The answer as it is read.
pub struct Resp {
    state: State,
    hdr: [u8; HDR_MAX],
    hdr_len: usize,
    pub status: u16,
    pub reason: Buf<40>,
    pub kind: Kind,
    pub ctype: Buf<48>,
    pub latin1: bool,
    pub location: Buf<PATH_MAX>,
    length: Option<u64>,
    chunked: bool,
    chunk_left: u64,
    chunk_digits: u8,
    chunk_line: u8,
    pub body_bytes: u64,
    pub raw_bytes: u64,
    pub error: Option<&'static str>,
    pub cut: bool,
}

impl Resp {
    pub const fn new() -> Self {
        Resp {
            state: State::Headers,
            hdr: [0; HDR_MAX],
            hdr_len: 0,
            status: 0,
            reason: Buf::new(),
            kind: Kind::Other,
            ctype: Buf::new(),
            latin1: false,
            location: Buf::new(),
            length: None,
            chunked: false,
            chunk_left: 0,
            chunk_digits: 0,
            chunk_line: 0,
            body_bytes: 0,
            raw_bytes: 0,
            error: None,
            cut: false,
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Headers;
        self.hdr_len = 0;
        self.status = 0;
        self.reason.clear();
        self.kind = Kind::Other;
        self.ctype.clear();
        self.latin1 = false;
        self.location.clear();
        self.length = None;
        self.chunked = false;
        self.chunk_left = 0;
        self.chunk_digits = 0;
        self.chunk_line = 0;
        self.body_bytes = 0;
        self.raw_bytes = 0;
        self.error = None;
        self.cut = false;
    }

    /// The headers have been read.
    pub fn have_headers(&self) -> bool {
        self.state != State::Headers
    }

    /// The whole body has arrived (by length or the last chunk), or the answer has no body.
    pub fn complete(&self) -> bool {
        self.state == State::Done
    }

    /// A redirect Dira should follow: a 3xx with a Location.
    pub fn redirect(&self) -> bool {
        matches!(self.status, 301 | 302 | 303 | 307 | 308) && self.location.len() > 0
    }

    /// The connection closed: a body read to close is then complete.
    pub fn closed(&mut self) {
        if self.state == State::Body && self.length.is_none() {
            self.state = State::Done;
        }
    }

    /// Read more of the answer. Body bytes of a page worth showing go to `body`, with the answer's headers read
    /// (its kind and charset) so the reader can start on the first bytes. Returns false when Dira wants no
    /// more (done, an error, a redirect, a body that is not a page, or the size cap).
    pub fn feed(&mut self, input: &[u8], body: &mut dyn FnMut(&Resp, &[u8])) -> bool {
        let mut i = 0;
        while i < input.len() && self.error.is_none() && self.state != State::Done {
            if self.raw_bytes >= RAW_MAX {
                self.cut = true;
                return false;
            }
            match self.state {
                State::Headers => {
                    if self.hdr_len == HDR_MAX {
                        self.error = Some("the server's headers are longer than 4 KiB");
                        break;
                    }
                    self.hdr[self.hdr_len] = input[i];
                    self.hdr_len += 1;
                    self.raw_bytes += 1;
                    i += 1;
                    let h = &self.hdr[..self.hdr_len];
                    if h.ends_with(b"\r\n\r\n") || h.ends_with(b"\n\n") {
                        self.headers_done();
                        if self.state == State::Headers {
                            // a 1xx answer: the real one follows
                            self.hdr_len = 0;
                        } else if !self.wanted() {
                            return false;
                        }
                    }
                }
                State::Body => {
                    let room = (RAW_MAX - self.raw_bytes) as usize;
                    let mut take = (input.len() - i).min(room);
                    if let Some(len) = self.length {
                        take = take.min((len - self.body_bytes) as usize);
                    }
                    body(self, &input[i..i + take]);
                    i += take;
                    self.body_bytes += take as u64;
                    self.raw_bytes += take as u64;
                    if self.length == Some(self.body_bytes) {
                        self.state = State::Done;
                    }
                }
                State::ChunkSize | State::ChunkExt => {
                    let c = input[i];
                    i += 1;
                    self.raw_bytes += 1;
                    self.chunk_line = self.chunk_line.saturating_add(1);
                    if self.chunk_line > CHUNK_LINE_MAX {
                        self.error = Some("a chunk-size line is longer than 64 bytes");
                        break;
                    }
                    if c == b'\n' {
                        self.chunk_line = 0;
                        if self.chunk_digits == 0 {
                            self.error = Some("a chunk of the page has no size");
                        } else if self.chunk_left == 0 {
                            self.state = State::Done; // the last chunk; trailers are not read
                        } else {
                            self.state = State::ChunkData;
                        }
                        continue;
                    }
                    if self.state == State::ChunkExt || c == b'\r' {
                        continue;
                    }
                    match (c as char).to_digit(16) {
                        Some(d) => {
                            // checked: a size past 4 MiB is refused before it can overflow anything
                            match self.chunk_left.checked_mul(16).and_then(|v| v.checked_add(d as u64)) {
                                Some(v) if v <= RAW_MAX => {
                                    self.chunk_left = v;
                                    self.chunk_digits = self.chunk_digits.saturating_add(1);
                                }
                                _ => self.error = Some("a chunk of the page claims more than 4 MiB"),
                            }
                        }
                        None if c == b';' || c == b' ' || c == b'\t' => self.state = State::ChunkExt,
                        None => self.error = Some("a chunk size is not a number"),
                    }
                }
                State::ChunkData => {
                    let room = (RAW_MAX - self.raw_bytes) as usize;
                    let take = (input.len() - i).min(self.chunk_left as usize).min(room);
                    body(self, &input[i..i + take]);
                    i += take;
                    self.chunk_left -= take as u64;
                    self.body_bytes += take as u64;
                    self.raw_bytes += take as u64;
                    if self.chunk_left == 0 {
                        self.state = State::ChunkEnd;
                    }
                }
                State::ChunkEnd => {
                    let c = input[i];
                    i += 1;
                    self.raw_bytes += 1;
                    if c == b'\n' {
                        self.state = State::ChunkSize;
                        self.chunk_digits = 0;
                        self.chunk_left = 0;
                    } else if c != b'\r' {
                        self.error = Some("a chunk of the page is longer than its size");
                    }
                }
                State::Done => {}
            }
        }
        if self.raw_bytes >= RAW_MAX && self.state != State::Done {
            self.cut = true;
            return false;
        }
        self.error.is_none() && self.state != State::Done
    }

    /// After the headers: is the body worth reading?
    fn wanted(&self) -> bool {
        self.error.is_none() && !self.redirect() && self.kind != Kind::Other && self.state != State::Done
    }

    fn headers_done(&mut self) {
        // fields are borrowed apart (hdr read, the others written), so no 4 KiB copy lands on the kernel stack
        let h = &self.hdr[..self.hdr_len];
        let mut lines = h.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
        let first = lines.next().unwrap_or(b"");
        if !first.starts_with(b"HTTP/1.") || first.len() < 12 || first[8] != b' ' || !first[9..12].iter().all(|c| c.is_ascii_digit()) {
            self.error = Some("the server's answer is not HTTP/1.x");
            self.state = State::Body;
            return;
        }
        self.status = first[9..12].iter().fold(0u16, |a, &c| a * 10 + (c - b'0') as u16);
        if (100..200).contains(&self.status) {
            return; // stay in Headers: 100 Continue and friends
        }
        self.reason.clear();
        let r = trim(first.get(13..).unwrap_or(b""));
        self.reason.push(&r[..r.len().min(40)]);
        let mut te_seen = false;
        for l in lines {
            let Some(colon) = l.iter().position(|&c| c == b':') else { continue };
            let (key, val) = (&l[..colon], trim(&l[colon + 1..]));
            if eq_ci(key, b"content-length") {
                if val.is_empty() || !val.iter().all(|c| c.is_ascii_digit()) {
                    self.error = Some("the server's Content-Length is not a number");
                    return;
                }
                // checked: a length past 4 MiB is refused, so no sum below can overflow
                let v = val.iter().try_fold(0u64, |a, &c| a.checked_mul(10)?.checked_add((c - b'0') as u64));
                let Some(v) = v.filter(|&v| v <= RAW_MAX) else {
                    self.error = Some("the server says the page is bigger than 4 MiB");
                    return;
                };
                if self.length.map_or(false, |old| old != v) {
                    self.error = Some("the server sent two different Content-Lengths");
                    return;
                }
                self.length = Some(v);
            } else if eq_ci(key, b"transfer-encoding") {
                te_seen = true;
                // only "chunked" (possibly after "identity"); anything else Dira cannot read
                let last = val.rsplit(|&c| c == b',').next().map(trim).unwrap_or(b"");
                if !eq_ci(last, b"chunked") {
                    self.error = Some("the server compressed the page in a way Dira cannot read yet");
                    return;
                }
                self.chunked = true;
            } else if eq_ci(key, b"content-encoding") {
                if !(val.is_empty() || eq_ci(val, b"identity")) {
                    self.error = Some("the server compressed the page, and Dira cannot read compressed pages yet");
                    return;
                }
            } else if eq_ci(key, b"location") {
                self.location.clear();
                if !self.location.push(val) {
                    self.location.clear();
                }
            } else if eq_ci(key, b"content-type") {
                self.ctype.clear();
                let main = trim(&val[..val.iter().position(|&c| c == b';').unwrap_or(val.len())]);
                self.ctype.push(&main[..main.len().min(48)]);
                self.kind = if eq_ci(main, b"text/html") || eq_ci(main, b"application/xhtml+xml") {
                    Kind::Html
                } else if eq_ci(main, b"text/plain") {
                    Kind::Text
                } else {
                    Kind::Other
                };
                // charset=iso-8859-1 or windows-1252: one byte, one letter
                let mut rest = val;
                while let Some(p) = rest.iter().position(|&c| c == b';') {
                    rest = &rest[p + 1..];
                    let part = trim(&rest[..rest.iter().position(|&c| c == b';').unwrap_or(rest.len())]);
                    if part.len() > 8 && eq_ci(&part[..8], b"charset=") {
                        let cs = trim_quotes(&part[8..]);
                        self.latin1 = eq_ci(cs, b"iso-8859-1") || eq_ci(cs, b"windows-1252") || eq_ci(cs, b"latin1");
                    }
                }
            }
        }
        if self.ctype.len() == 0 {
            self.kind = Kind::Html; // no Content-Type: read it as a page, as other browsers do
        }
        if te_seen {
            self.length = None; // RFC 9112 6.3: chunked wins over Content-Length
        }
        self.state = if self.status == 204 || self.status == 304 || self.length == Some(0) {
            State::Done
        } else if self.chunked {
            State::ChunkSize
        } else {
            State::Body
        };
    }
}

fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let e = s.iter().rposition(|c| !c.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(a);
    &s[a..e.max(a)]
}

fn trim_quotes(s: &[u8]) -> &[u8] {
    let s = s.strip_prefix(b"\"").unwrap_or(s);
    s.strip_suffix(b"\"").unwrap_or(s)
}

/// `GET <path> HTTP/1.1` with the few headers Dira sends. None when it does not fit in `out`.
pub fn request(u: &Url, out: &mut [u8]) -> Option<usize> {
    let mut b: Buf<1024> = Buf::new();
    let mut port = [0u8; 6];
    let mut pn = 0;
    if u.port != if u.https { 443 } else { 80 } {
        let mut v = u.port;
        let mut d = [0u8; 5];
        let mut k = 0;
        loop {
            d[4 - k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        port[0] = b':';
        port[1..1 + k].copy_from_slice(&d[5 - k..]);
        pn = 1 + k;
    }
    let ok = b.push(b"GET ")
        && b.push(u.path.get())
        && b.push(b" HTTP/1.1\r\nHost: ")
        && b.push(u.host.get())
        && b.push(&port[..pn])
        && b.push(b"\r\nUser-Agent: Dira/0.1 (AfriSovera)\r\nAccept: text/html, text/plain;q=0.9\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n");
    if !ok || b.len() > out.len() {
        return None;
    }
    out[..b.len()].copy_from_slice(b.get());
    Some(b.len())
}
