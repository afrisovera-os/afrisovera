//! Dira's addresses (BROWSER_PLAN.md section 4): `http://host[:port]/path?query`, typed or found in a page.
//!
//! Pure code: no kernel calls, no `unsafe`, no allocation, so the same file builds in the kernel and in the host
//! tests and fuzzing (tools/dira-host). Every buffer is fixed; anything longer is refused with a reason, never cut.
#![forbid(unsafe_code)]

pub const HOST_MAX: usize = 253;
pub const PATH_MAX: usize = 400;

/// A small fixed buffer of bytes.
#[derive(Clone, Copy)]
pub struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    pub const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    pub fn get(&self) -> &[u8] {
        &self.b[..self.n]
    }
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn clear(&mut self) {
        self.n = 0;
    }
    /// Append; false (and nothing appended) when it does not fit.
    pub fn push(&mut self, s: &[u8]) -> bool {
        if s.len() > N - self.n {
            return false;
        }
        self.b[self.n..self.n + s.len()].copy_from_slice(s);
        self.n += s.len();
        true
    }
    pub fn push_byte(&mut self, c: u8) -> bool {
        self.push(&[c])
    }
    pub fn pop(&mut self) -> Option<u8> {
        if self.n == 0 {
            return None;
        }
        self.n -= 1;
        Some(self.b[self.n])
    }
    pub fn truncate(&mut self, n: usize) {
        if n < self.n {
            self.n = n;
        }
    }
}

#[derive(Clone, Copy)]
pub struct Url {
    pub https: bool,
    pub host: Buf<HOST_MAX>,
    pub port: u16,
    /// The path and query, always starting with '/'; the fragment is dropped.
    pub path: Buf<PATH_MAX>,
}

fn lower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

fn starts_with_ci(s: &[u8], p: &[u8]) -> bool {
    s.len() >= p.len() && s[..p.len()].iter().zip(p).all(|(a, b)| lower(*a) == *b)
}

fn trim(s: &[u8]) -> &[u8] {
    let a = s.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(s.len());
    let e = s.iter().rposition(|c| !c.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(a);
    &s[a..e.max(a)]
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Append path bytes, percent-encoding what may not travel raw (spaces, controls, bytes past ASCII).
fn push_path(out: &mut Buf<PATH_MAX>, s: &[u8]) -> bool {
    for &c in s {
        let ok = if c > 0x20 && c < 0x7f && c != b'"' && c != b'<' && c != b'>' && c != b'\\' && c != b'^' && c != b'`' && c != b'{' && c != b'|' && c != b'}' {
            out.push_byte(c)
        } else {
            out.push(&[b'%', HEX[(c >> 4) as usize], HEX[(c & 15) as usize]])
        };
        if !ok {
            return false;
        }
    }
    true
}

/// The scheme of an absolute address (`letters:` before any '/', '?' or '#'), lower case, or None.
fn scheme_of(s: &[u8]) -> Option<&[u8]> {
    let colon = s.iter().position(|&c| c == b':')?;
    let sch = &s[..colon];
    if sch.is_empty() || sch.len() > 16 || !sch[0].is_ascii_alphabetic() {
        return None;
    }
    if !sch.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'+' || *c == b'-' || *c == b'.') {
        return None;
    }
    Some(sch)
}

/// Parse an address as a person types it. No scheme means `http://`.
pub fn parse(input: &[u8]) -> Result<Url, &'static str> {
    let s = trim(input);
    if s.is_empty() {
        return Err("the address is empty");
    }
    let (https, rest) = if starts_with_ci(s, b"http://") {
        (false, &s[7..])
    } else if starts_with_ci(s, b"https://") {
        (true, &s[8..])
    } else if let Some(sc) = scheme_of(s) {
        // `localhost:8080/x` is a host and a port, `ftp://x` or `mailto:x` a scheme Dira does not open
        if s[sc.len() + 1..].first().map_or(false, |c| c.is_ascii_digit()) {
            (false, s)
        } else {
            return Err("Dira opens http:// and https:// addresses only");
        }
    } else {
        (false, s)
    };
    let end = rest.iter().position(|&c| c == b'/' || c == b'?' || c == b'#').unwrap_or(rest.len());
    let auth = &rest[..end];
    if auth.contains(&b'@') {
        return Err("addresses with a user name in them are refused");
    }
    if auth.first() == Some(&b'[') {
        return Err("IPv6 addresses are not supported yet");
    }
    let (host_s, port) = match auth.iter().position(|&c| c == b':') {
        Some(i) => {
            let p = &auth[i + 1..];
            if p.is_empty() || p.len() > 5 || !p.iter().all(|c| c.is_ascii_digit()) {
                return Err("the port is not a number from 1 to 65535");
            }
            let v = p.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32);
            if v == 0 || v > 65535 {
                return Err("the port is not a number from 1 to 65535");
            }
            (&auth[..i], v as u16)
        }
        None => (auth, if https { 443 } else { 80 }),
    };
    let host_s = host_s.strip_suffix(b".").unwrap_or(host_s);
    if host_s.is_empty() {
        return Err("the address has no host name");
    }
    if host_s.len() > HOST_MAX {
        return Err("the host name is longer than 253 letters");
    }
    if !host_s.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-' || *c == b'.') || host_s.windows(2).any(|w| w == b"..") || host_s[0] == b'.' || host_s.ends_with(b".") {
        return Err("the host name has letters a host name cannot have");
    }
    let mut url = Url { https, host: Buf::new(), port, path: Buf::new() };
    for &c in host_s {
        url.host.push_byte(lower(c));
    }
    let tail = &rest[end..];
    let tail = &tail[..tail.iter().position(|&c| c == b'#').unwrap_or(tail.len())];
    if tail.first() != Some(&b'/') && !url.path.push_byte(b'/') {
        return Err("the address is too long");
    }
    if !push_path(&mut url.path, tail) {
        return Err("the address is too long");
    }
    normalize(&mut url.path);
    Ok(url)
}

/// Remove `.` and `..` segments from the path part (before any '?'), as RFC 3986 section 5.2.4 does.
fn normalize(p: &mut Buf<PATH_MAX>) {
    let all = *p;
    let s = all.get();
    let q = s.iter().position(|&c| c == b'?').unwrap_or(s.len());
    let (path, query) = (&s[..q], &s[q..]);
    if !path.split(|&c| c == b'/').any(|seg| seg == b"." || seg == b"..") {
        return;
    }
    // segment starts in the output, to drop the last one on ".."
    let mut out: Buf<PATH_MAX> = Buf::new();
    let mut starts = [0usize; 128];
    let mut depth = 0usize;
    let segs = path.split(|&c| c == b'/').skip(1);
    let nseg = path.split(|&c| c == b'/').skip(1).count();
    for (i, seg) in segs.enumerate() {
        let last = i + 1 == nseg;
        if seg == b"." {
            if last {
                out.push_byte(b'/');
            }
            continue;
        }
        if seg == b".." {
            if depth > 0 {
                depth -= 1;
                out.truncate(starts[depth]);
            }
            if last {
                out.push_byte(b'/');
            }
            continue;
        }
        if depth < starts.len() {
            starts[depth] = out.len();
            depth += 1;
        }
        out.push_byte(b'/');
        out.push(seg);
    }
    if out.len() == 0 {
        out.push_byte(b'/');
    }
    out.push(query);
    *p = out;
}

/// Resolve a link found in a page against the page's own address (RFC 3986 section 5.2, the common cases).
/// `Ok(None)` means a link to a place in the same page (`#x`), which Dira does not move to in M1.
pub fn resolve(base: &Url, href: &[u8]) -> Result<Option<Url>, &'static str> {
    let h = trim(href);
    if h.is_empty() || h[0] == b'#' {
        return Ok(None);
    }
    if starts_with_ci(h, b"http://") || starts_with_ci(h, b"https://") {
        return parse(h).map(Some);
    }
    if h.starts_with(b"//") {
        let mut full: Buf<{ PATH_MAX + HOST_MAX + 16 }> = Buf::new();
        full.push(if base.https { b"https:" } else { b"http:" });
        if !full.push(h) {
            return Err("the link is too long");
        }
        return parse(full.get()).map(Some);
    }
    if let Some(sch) = scheme_of(h) {
        if starts_with_ci(sch, b"javascript") {
            return Err("this link runs a script, and Dira runs no scripts");
        }
        if starts_with_ci(sch, b"mailto") {
            return Err("this link writes an email; Dira has no mail");
        }
        return Err("Dira opens http:// and https:// links only");
    }
    let mut u = *base;
    let h = &h[..h.iter().position(|&c| c == b'#').unwrap_or(h.len())];
    let bp = base.path.get();
    let bq = bp.iter().position(|&c| c == b'?').unwrap_or(bp.len());
    u.path.clear();
    let ok = if h[0] == b'/' {
        push_path(&mut u.path, h)
    } else if h[0] == b'?' {
        u.path.push(&bp[..bq]) && push_path(&mut u.path, h)
    } else {
        let dir = bp[..bq].iter().rposition(|&c| c == b'/').map(|i| i + 1).unwrap_or(0);
        u.path.push(&bp[..dir]) && push_path(&mut u.path, h)
    };
    if !ok {
        return Err("the link is too long");
    }
    if u.path.get().first() != Some(&b'/') {
        let mut p: Buf<PATH_MAX> = Buf::new();
        p.push_byte(b'/');
        if !p.push(u.path.get()) {
            return Err("the link is too long");
        }
        u.path = p;
    }
    normalize(&mut u.path);
    Ok(Some(u))
}

/// The address as text: `http://host[:port]/path`.
pub fn show(u: &Url, out: &mut Buf<{ PATH_MAX + HOST_MAX + 16 }>) {
    out.clear();
    out.push(if u.https { b"https://" } else { b"http://" });
    out.push(u.host.get());
    if u.port != if u.https { 443 } else { 80 } {
        let mut d = [0u8; 6];
        let mut n = 0;
        let mut v = u.port;
        loop {
            d[5 - n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        out.push_byte(b':');
        out.push(&d[6 - n..]);
    }
    out.push(u.path.get());
}

pub type Shown = Buf<{ PATH_MAX + HOST_MAX + 16 }>;

/// The host as an IPv4 address, when it is written as one.
pub fn ipv4(u: &Url) -> Option<[u8; 4]> {
    let h = u.host.get();
    let mut ip = [0u8; 4];
    let mut n = 0;
    for part in h.split(|&c| c == b'.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.iter().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let v = part.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32);
        if v > 255 {
            return None;
        }
        ip[n] = v as u8;
        n += 1;
    }
    if n == 4 {
        Some(ip)
    } else {
        None
    }
}

/// How private an IPv4 address is: 0 public, 1 a private network (10/8, 172.16/12, 192.168/16, and 100.64/10 for
/// carrier NAT and VPN overlays), 2 link-local (169.254/16, where cloud machines answer questions about themselves),
/// 3 this machine (127/8), 4 no single machine (0/8, and 224/3: multicast, reserved, broadcast).
pub fn ip_class(ip: [u8; 4]) -> u8 {
    match ip {
        [0, ..] => 4,             // "this network": no single machine, never a redirect's target
        [a, ..] if a >= 224 => 4, // multicast, reserved and broadcast: never a redirect's target
        [127, ..] => 3,
        [169, 254, ..] => 2,
        [10, ..] | [192, 168, ..] => 1,
        [172, b, ..] if (16..32).contains(&b) => 1,
        [100, b, ..] if (64..128).contains(&b) => 1, // carrier NAT and private VPN overlays (100.64/10)
        _ => 0,
    }
}

/// Condition 4 (architect's verdict): a redirect is followed only to a public address or to one exactly as private
/// as the server that sent it, and never to class 4. A public site cannot steer Dira at the router, a site on the
/// local network cannot steer it at this machine or at link-local addresses, and nothing reaches 0/8 or 224/3.
pub fn redirect_ok(from: [u8; 4], to: [u8; 4]) -> bool {
    let t = ip_class(to);
    t != 4 && (t == 0 || t == ip_class(from))
}
