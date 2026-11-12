//! `appget <ip> <port> <name> <sha256>`: install a Nia app from the network into the VFS (item 2,
//! claude/appget, architect's verdict and amendment of 11 Nov 2026 in ARCHITECT_REVIEWS.md).
//!
//! AFR-0003 v0 sideloading: a bare `.wasm`, unsigned. The owner must give the SHA-256 the publisher
//! published by another route (a web page, a printed card, a message), because plain HTTP lets anyone on the
//! path swap the bytes. Order: name and argument checks, then HTTP/1.0 GET through the existing TCP client
//! (`virtio::net_tcp_dial_with`), then the HTTP checks (status 200 only, Content-Length 8..=16384 required,
//! no Transfer-Encoding, headers at most 1 KiB, a short body refused, bytes past Content-Length ignored), then
//! the hash (a mismatch writes nothing and cannot be overridden), then the module check (`nia::check_module`,
//! which parses and never runs), then the owner's typed `y`, then `vfs::write_full` and a sync.
//!
//! An installed app stays "unverified" (it is not added to the integrity manifest), so `nia::run_file` runs it
//! without file, net, AI and voice. Nothing is written before the `y`; every refusal is one line.

use core::ptr::addr_of_mut;

const HDR_MAX: usize = 1024;
const BODY_MIN: usize = 8;
const BODY_MAX: usize = crate::vfs::MAX_BIG_FILE_BYTES;
const NAME_MAX: usize = 24;
const WAIT_MS: u64 = 10_000;

static mut HDR: [u8; HDR_MAX] = [0; HDR_MAX];
static mut BODY: [u8; BODY_MAX] = [0; BODY_MAX];

/// What the TCP handler has seen so far.
struct Fetch {
    hdr_len: usize,
    in_body: bool,
    want: usize, // Content-Length
    got: usize,
    error: Option<&'static str>,
    status: u32,
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut ip = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: u32 = part.parse().ok()?;
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

fn parse_port(s: &str) -> Option<u16> {
    if s.is_empty() || s.len() > 5 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: u32 = s.parse().ok()?;
    if (1..=65535).contains(&v) {
        Some(v as u16)
    } else {
        None
    }
}

fn name_ok(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= NAME_MAX
        && b[0] != b'.'
        && s.ends_with(".wasm")
        && b.len() > 5
        && b.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-')
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn parse_sha(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = (hex_val(b[2 * i])? << 4) | hex_val(b[2 * i + 1])?;
    }
    Some(out)
}

fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Read the status line and headers once the blank line has arrived.
fn parse_headers(h: &[u8], f: &mut Fetch) {
    let mut lines = h.split(|&c| c == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    let status = lines.next().unwrap_or(b"");
    // "HTTP/1.x NNN reason"
    let mut parts = status.split(|&c| c == b' ').filter(|p| !p.is_empty());
    let proto = parts.next().unwrap_or(b"");
    let code = parts.next().unwrap_or(b"");
    if !proto.starts_with(b"HTTP/1.") || code.len() != 3 || !code.iter().all(|c| c.is_ascii_digit()) {
        f.error = Some("the server's answer is not HTTP/1.x");
        return;
    }
    f.status = code.iter().fold(0u32, |a, &c| a * 10 + (c - b'0') as u32);
    if f.status != 200 {
        f.error = Some("the server did not answer 200 OK (no redirects are followed)");
        return;
    }
    let mut length: Option<usize> = None;
    for l in lines {
        let colon = match l.iter().position(|&c| c == b':') {
            Some(i) => i,
            None => continue,
        };
        let (key, val) = (&l[..colon], &l[colon + 1..]);
        let val: &[u8] = {
            let s = val.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(val.len());
            let e = val.iter().rposition(|c| !c.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(s);
            &val[s..e.max(s)]
        };
        if eq_ignore_case(key, b"transfer-encoding") {
            f.error = Some("the server used Transfer-Encoding (refused: a plain Content-Length body is required)");
            return;
        }
        if eq_ignore_case(key, b"content-length") {
            if val.is_empty() || val.len() > 6 || !val.iter().all(|c| c.is_ascii_digit()) || length.is_some() {
                f.error = Some("the server's Content-Length is not a single number");
                return;
            }
            length = Some(val.iter().fold(0usize, |a, &c| a * 10 + (c - b'0') as usize));
        }
    }
    match length {
        None => f.error = Some("the server sent no Content-Length (required)"),
        Some(n) if n < BODY_MIN => f.error = Some("the app is shorter than 8 bytes"),
        Some(n) if n > BODY_MAX => f.error = Some("the app is bigger than 16,384 bytes (the most a file holds)"),
        Some(n) => {
            f.want = n;
            f.in_body = true;
        }
    }
}

/// The TCP handler: headers into HDR (at most 1 KiB), then exactly Content-Length bytes into BODY.
fn feed(f: &mut Fetch, input: &[u8]) -> bool {
    // SAFETY: single-threaded shell context; HDR/BODY are only touched here and in `run`, never past their size.
    unsafe {
        let hdr = &mut *addr_of_mut!(HDR);
        let body = &mut *addr_of_mut!(BODY);
        let mut i = 0;
        while i < input.len() && f.error.is_none() {
            if !f.in_body {
                if f.hdr_len == HDR_MAX {
                    f.error = Some("the server's headers are longer than 1 KiB");
                    break;
                }
                hdr[f.hdr_len] = input[i];
                f.hdr_len += 1;
                i += 1;
                if f.hdr_len >= 4 && &hdr[f.hdr_len - 4..f.hdr_len] == b"\r\n\r\n" {
                    let h = &hdr[..f.hdr_len - 4];
                    parse_headers(h, f);
                }
            } else {
                let take = (f.want - f.got).min(input.len() - i);
                body[f.got..f.got + take].copy_from_slice(&input[i..i + take]);
                f.got += take;
                break; // bytes past Content-Length are ignored
            }
        }
    }
    f.error.is_none() && !(f.in_body && f.got == f.want)
}

fn hex16(d: &[u8; 32]) -> [u8; 16] {
    let mut full = [0u8; 64];
    crate::sha256::to_hex(d, &mut full);
    let mut out = [0u8; 16];
    out.copy_from_slice(&full[..16]);
    out
}

/// `appget <ip> <port> <name> <sha256>`. `getkey` is the shell's own key reader (for the `y`).
pub fn run(args: &str, getkey: fn() -> u8) {
    const USAGE: &str = "appget: usage: appget <ip> <port> <name.wasm> <sha256>  (the SHA-256 the publisher gave you, 64 hex)";
    let mut it = args.split_whitespace();
    let (ip_s, port_s, name, sha_s) = match (it.next(), it.next(), it.next(), it.next(), it.next()) {
        (Some(a), Some(b), Some(c), Some(d), None) => (a, b, c, d),
        _ => {
            crate::kprintln!("{}", USAGE);
            return;
        }
    };
    let ip = match parse_ip(ip_s) {
        Some(ip) => ip,
        None => return crate::kprintln!("appget: '{}' is not an IP address like 10.0.2.2; nothing written", ip_s),
    };
    let port = match parse_port(port_s) {
        Some(p) => p,
        None => return crate::kprintln!("appget: '{}' is not a port from 1 to 65535; nothing written", port_s),
    };
    if !name_ok(name) {
        return crate::kprintln!(
            "appget: '{}' is not an app name (1-24 characters of A-Z a-z 0-9 . _ -, ending .wasm); nothing written",
            name
        );
    }
    if crate::nia::BUNDLED.iter().any(|(n, _)| *n == name) || crate::integrity::expected(name).is_some() {
        return crate::kprintln!("appget: {} is a bundled app and cannot be replaced; nothing written", name);
    }
    let want_hash = match parse_sha(sha_s) {
        Some(h) => h,
        None => {
            return crate::kprintln!(
                "appget: the SHA-256 must be 64 hex characters (got {} characters); nothing written",
                sha_s.len()
            )
        }
    };

    // GET /name HTTP/1.0
    let mut req = [0u8; 64];
    let mut n = 0;
    for part in [b"GET /".as_slice(), name.as_bytes(), b" HTTP/1.0\r\n\r\n".as_slice()] {
        req[n..n + part.len()].copy_from_slice(part);
        n += part.len();
    }
    crate::kprintln!(
        "appget: fetching http://{}.{}.{}.{}:{}/{} ...",
        ip[0], ip[1], ip[2], ip[3], port, name
    );
    let mut f = Fetch { hdr_len: 0, in_body: false, want: 0, got: 0, error: None, status: 0 };
    let mut handler = |input: &[u8], _out: &mut alloc::vec::Vec<u8>| feed(&mut f, input);
    crate::virtio::net_tcp_dial_with(ip, port, &req[..n], WAIT_MS, &mut handler);
    if let Some(e) = f.error {
        return crate::kprintln!("appget: {}; nothing written", e);
    }
    if !f.in_body {
        return crate::kprintln!("appget: no complete answer from the server; nothing written");
    }
    if f.got < f.want {
        return crate::kprintln!(
            "appget: the server sent {} of the {} bytes it announced; nothing written",
            f.got, f.want
        );
    }
    // SAFETY: BODY holds f.got <= BODY_MAX bytes from this fetch.
    let bytes: &[u8] = unsafe { &(&*addr_of_mut!(BODY))[..f.got] };

    // The hash the owner gave, before anything else looks at the bytes.
    let got_hash = crate::sha256::digest(bytes);
    if got_hash != want_hash {
        let (e, g) = (hex16(&want_hash), hex16(&got_hash));
        return crate::kprintln!(
            "appget: hash does not match: expected {}..., got {}...; nothing written",
            core::str::from_utf8(&e).unwrap_or("?"),
            core::str::from_utf8(&g).unwrap_or("?")
        );
    }
    let needs = match crate::nia::check_module(bytes) {
        Ok(caps) => caps,
        Err(e) => return crate::kprintln!("appget: {} is not a Nia app ({}); nothing written", name, e),
    };

    // The owner's choice (AFR-0003 5.4).
    let mut full = [0u8; 64];
    crate::sha256::to_hex(&got_hash, &mut full);
    crate::kprintln!("appget: {}, {} bytes", name, bytes.len());
    crate::kprintln!("  SHA-256 matches the one you gave: {}", core::str::from_utf8(&full).unwrap_or("?"));
    crate::kprintln!("  unsigned (no publisher), downloaded over plain HTTP (not encrypted)");
    crate::nia::print_caps("  it uses: ", needs, "  (none)");
    crate::nia::print_caps("  it will NOT get: ", needs & !crate::nia::CAP_SIDELOAD, "  (nothing withheld)");
    match crate::vfs::read_full(name) {
        Some(old) => crate::kprintln!("  replaces the existing {} ({} bytes)", name, old.len()),
        None => crate::kprintln!("  a new file"),
    }
    crate::kprintln!("appget: install {}? type y", name);
    let k = getkey();
    if k != b'y' {
        return crate::kprintln!("appget: cancelled; nothing written");
    }
    if !crate::vfs::write_full(name, bytes) {
        return crate::kprintln!("appget: the file table is full (24 files); nothing written");
    }
    if crate::virtio::blk_present() {
        crate::vfs::sync_to_disk();
    } else {
        crate::kprintln!("appget: no disk attached: {} lasts until the next boot", name);
    }
    crate::kprintln!("appget: installed {} ({} bytes); run it with: nia {}", name, bytes.len(), name);
}
