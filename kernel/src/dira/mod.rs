//! Dira, AfriSovera's own web browser, milestone M1 (BROWSER_PLAN.md in the shared notes, architect's gate):
//! `dira <address>` fetches a page over HTTP/1.1 and shows it as readable text in a cell window on the desk.
//!
//!   dira <address>        the page in a desk window (on the console when the desk is not showing)
//!   dira text <address>   the page on the console, wrapped at 80, with its links listed
//!   dira file <name>      an .html or .txt file from the file store, the same way
//!
//! The parsers (url.rs, http.rs, html.rs) are pure, forbid `unsafe` and build on the host for tests and fuzzing
//! (dira-host). This file is the kernel side: the TCP client (`virtio::net_tcp_dial_with`), DNS, and the cell
//! window calls nia.rs also uses (`desk::app_cell_*`). Every buffer is a fixed static below (about 150 KiB);
//! the raw HTML is never kept. Dira writes nothing to disk, keeps no cookies, runs no scripts and sends no forms.

mod html;
mod http;
mod url;

use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};
use html::{Html, Page};
use http::{Kind, Resp};
use url::{Buf, Shown, Url};

use crate::desk::cell_abi as abi;

/// Rows of the wrapped page Dira can show; a longer page shows a line saying so.
const ROWS_MAX: usize = 4096;
const HIST: usize = 8;
const WAIT_MS: u64 = 30_000;
const NAME_MAX: usize = 24;
const COLS_MAX: usize = 120;

/// Where a page came from: the network, or a file in the store.
#[derive(Clone, Copy)]
enum Loc {
    Net(Url),
    File(Buf<NAME_MAX>),
}

struct Bufs {
    page: Page,
    html: Html,
    resp: Resp,
    starts: [u32; ROWS_MAX],
    nrows: usize,
    width: usize,
    status: Buf<100>,
    typed: Buf<{ url::PATH_MAX }>,
}

static mut B: Bufs = Bufs {
    page: Page::new(),
    html: Html::new(),
    resp: Resp::new(),
    starts: [0; ROWS_MAX],
    nrows: 0,
    width: 0,
    status: Buf::new(),
    typed: Buf::new(),
};

/// Where Dira is and where it has been. Kept apart from `B` because an empty `Option<Loc>` is not all zero bytes:
/// in `B` it would move the whole 150 KiB from .bss into .data, into the kernel image (the cloud runner size job, 13 Nov).
struct Nav {
    loc: Option<Loc>,
    hist: [Option<Loc>; HIST],
    nhist: usize,
}

static mut N: Nav = Nav { loc: None, hist: [None; HIST], nhist: 0 };

/// SAFETY: as for `b()` below.
fn nav() -> &'static mut Nav {
    unsafe { &mut *addr_of_mut!(N) }
}

/// SAFETY: Dira runs only inside its shell command, in mainline context, one at a time (the cell window allows one
/// owner, and the shell runs one command); interrupt handlers never touch these buffers.
fn b() -> &'static mut Bufs {
    unsafe { &mut *addr_of_mut!(B) }
}

fn ms_now() -> u64 {
    (crate::timer::cycles() as u128 * 1000 / crate::timer::hz().max(1) as u128) as u64
}

/// An IPv4 address as text, added to `out`.
fn ip_text<const N: usize>(ip: [u8; 4], out: &mut Buf<N>) {
    for (k, part) in ip.iter().enumerate() {
        let mut d: Buf<100> = Buf::new();
        num(&mut d, *part as u64);
        out.push(d.get());
        if k < 3 {
            out.push_byte(b'.');
        }
    }
}

/// A small decimal writer for status lines.
fn num(out: &mut Buf<100>, mut v: u64) {
    let mut d = [0u8; 20];
    let mut n = 0;
    loop {
        d[19 - n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.push(&d[20 - n..]);
}

fn loc_text(l: &Loc, out: &mut Shown) {
    match l {
        Loc::Net(u) => url::show(u, out),
        Loc::File(n) => {
            out.clear();
            out.push(b"file:");
            out.push(n.get());
        }
    }
}

/// Replace the page with a short message (an error, or what a non-page address was).
fn message_page(lines: &[&[u8]]) {
    let s = b();
    s.page.reset();
    s.page.title.push(b"Dira");
    for (k, l) in lines.iter().enumerate() {
        s.page.note(l, if k == 0 { html::R_ERROR } else { html::R_TEXT });
    }
}

fn set_status(parts: &[&[u8]]) {
    let st = &mut b().status;
    st.clear();
    for p in parts {
        let room = 100 - st.len();
        st.push(&p[..p.len().min(room)]);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Stopping (architect's verdict, Q2): kernel code cannot be killed, so Dira stops itself. While it waits on the
// network, `virtio::set_net_abort` asks `stop_asked` every 256 spins and after each run of data; it looks at the desk
// (or the serial line) at most every 10 ms, so a close request ends the wait well inside 100 ms.
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    No,
    Close, // the close box, Alt+F4 or Ctrl+\: Dira ends
    Quit,  // q: Dira ends
    Esc,   // Esc: the load stops, the window stays
    CtrlC, // Ctrl+C on the console (dira text)
}

static mut STOP: Stop = Stop::No;
static mut STOP_AT: u64 = 0;
static mut STOP_LOOK: u64 = 0;
static mut STOP_WINDOW: bool = false;

/// The network wait's question (a plain fn, so virtio.rs holds no closure): did the person ask Dira to stop?
fn stop_asked() -> bool {
    // SAFETY: mainline only, while Dira runs; nothing else reads or writes these.
    unsafe {
        if STOP != Stop::No {
            return true;
        }
        let now = ms_now();
        if now.wrapping_sub(STOP_LOOK) < 10 {
            return false;
        }
        STOP_LOOK = now;
        let mut why = Stop::No;
        if STOP_WINDOW {
            // one desk pass (input, redraws) and whatever arrived: a close ends Dira, q too, Esc stops the load;
            // other keys wait for the page (they are dropped)
            for _ in 0..abi::QUEUE {
                let ev = crate::desk::app_cell_event(0);
                if ev == 0 {
                    break;
                }
                if ev < 0 {
                    why = Stop::Close;
                    break;
                }
                let ev = ev as u32;
                why = match ((ev >> 28) & 7, ev & abi::PAYLOAD) {
                    (abi::EV_CLOSE_REQUEST, _) | (abi::EV_CLOSED, _) => Stop::Close,
                    (abi::EV_KEY, 0x71) => Stop::Quit,
                    (abi::EV_KEY, 27) => Stop::Esc,
                    _ => Stop::No,
                };
                if why != Stop::No {
                    break;
                }
            }
        } else {
            while let Some(c) = crate::uart::getc() {
                if c == 3 {
                    why = Stop::CtrlC;
                }
            }
        }
        if why != Stop::No {
            STOP = why;
            STOP_AT = now;
        }
        why != Stop::No
    }
}

fn stopped() -> Stop {
    // SAFETY: as in `stop_asked`.
    unsafe { STOP }
}

/// Clears the network hook on every way out of a load.
struct HookGuard;
impl Drop for HookGuard {
    fn drop(&mut self) {
        crate::virtio::set_net_abort(None);
    }
}

/// M1b, the PC on a real network: open the route to `ip` (the leased address, the next hop's MAC; DHCP and ARP if
/// needed) before Dira talks to it. A failure becomes a page saying what is missing. Dropping the hold closes it.
#[cfg(feature = "pc")]
fn route(ip: [u8; 4], what: &str, t0: u64) -> Result<crate::virtio::RouteHold, ()> {
    use crate::virtio::RouteErr;
    match crate::virtio::route_open(ip, "[dira]", what) {
        Ok(h) => Ok(h),
        Err(RouteErr::Stopped) => {
            stopped_page(t0);
            Err(())
        }
        Err(_) if stopped() != Stop::No => {
            stopped_page(t0);
            Err(())
        }
        Err(e) => {
            let mut hop = Buf::<16>::new();
            let (line, why): (&[u8], &[u8]) = match e {
                RouteErr::NoCard => (b"This device has no network card Dira can use.", b"no network"),
                RouteErr::NoLease => (b"The router gave this computer no address (no answer to DHCP).", b"no address from the router"),
                RouteErr::NoRoute => (b"This address is not on the local network, and the network has no router.", b"no router"),
                RouteErr::Busy => (b"Another part of the system is using the network right now. Try again.", b"network busy"),
                RouteErr::NoArp(h) => {
                    ip_text(h, &mut hop);
                    (b"The router, or the computer at this address on the local network, did not answer:", b"no answer on the local network")
                }
                RouteErr::Stopped => (b"Stopped.", b"stopped"),
            };
            message_page(&[line, hop.get()]);
            set_status(&[why]);
            Err(())
        }
    }
}

/// ARM: QEMU's fixed plan (10.0.2.15 through the gateway); there is no route to open until an ARM board has a network.
/// (The DNS on ARM stays `net_dns_resolve`, QEMU's fixed path.)
#[cfg(not(feature = "pc"))]
fn route(_ip: [u8; 4], _what: &str, _t0: u64) -> Result<(), ()> {
    Ok(())
}

/// Dira's own words for a status code (the server's reason phrase is never shown).
fn phrase(code: u16) -> &'static [u8] {
    match code {
        200..=299 => b"OK",
        400 => b"Bad Request",
        401 => b"Unauthorized",
        403 => b"Forbidden",
        404 => b"Not Found",
        410 => b"Gone",
        429 => b"Too Many Requests",
        500 => b"Internal Server Error",
        502 => b"Bad Gateway",
        503 => b"Service Unavailable",
        400..=499 => b"refused by the server",
        500..=599 => b"server error",
        _ => b"",
    }
}

/// Fetch and read a page into the buffers, following up to 5 redirects. `window` says where a stop comes from
/// (the desk, or Ctrl+C on the console). The location actually shown (after redirects) becomes `loc`.
fn load(start: Loc, window: bool) {
    let s = b();
    let t0 = ms_now();
    s.status.clear();
    let mut u = match start {
        Loc::File(name) => {
            nav().loc = Some(start);
            let n = core::str::from_utf8(name.get()).unwrap_or("");
            match crate::vfs::read_full(n) {
                None => message_page(&[b"There is no file by that name in the store.", b"The Files window lists the files there."]),
                Some(bytes) => {
                    s.page.reset();
                    s.html.start(n.ends_with(".txt"), false);
                    s.html.feed(&mut s.page, bytes);
                    s.html.finish(&mut s.page);
                    page_notes();
                    let mut st: Buf<100> = Buf::new();
                    st.push(b"read from the store, ");
                    num(&mut st, bytes.len() as u64);
                    st.push(b" bytes");
                    s.status = st;
                }
            }
            return;
        }
        Loc::Net(u) => u,
    };
    // SAFETY: mainline only (see stop_asked).
    unsafe {
        STOP = Stop::No;
        STOP_LOOK = 0;
        STOP_WINDOW = window;
    }
    crate::virtio::set_net_abort(Some(stop_asked));
    let _hook = HookGuard;
    let mut from: Option<[u8; 4]> = None; // the server that sent the last redirect
    let mut hops: Buf<72> = Buf::new(); // every hop's resolved address, shown on the status line (M1b)
    for hop in 0..=5 {
        nav().loc = Some(Loc::Net(u));
        if u.https {
            message_page(&[
                b"Secure pages (https://) are not supported yet.",
                b"They come when Dira runs as its own protected program, outside the system's core.",
                b"If the site also answers on plain http://, type that address with g.",
            ]);
            return set_status(&[b"https is not supported yet"]);
        }
        if !crate::virtio::net_present() {
            message_page(&[b"This device has no network card Dira can use."]);
            return set_status(&[b"no network"]);
        }
        let host = core::str::from_utf8(u.host.get()).unwrap_or("");
        let ip = match url::ipv4(&u) {
            Some(ip) => ip,
            None => match {
                // PC (M1b): the name server the router named, reached through its own route, asked with a random id
                // and port, its answer checked (virtio::dns_resolve_routed). ARM: QEMU's fixed path.
                #[cfg(feature = "pc")]
                let found = {
                    if !crate::virtio::lease_stoppable() {
                        if stopped() != Stop::No {
                            return stopped_page(t0);
                        }
                        message_page(&[b"The router gave this computer no address (no answer to DHCP)."]);
                        return set_status(&[b"no address from the router"]);
                    }
                    let Ok(_g) = route(crate::virtio::dhcp_dns(), "name server", t0) else { return };
                    crate::virtio::dns_resolve_routed(host)
                };
                #[cfg(not(feature = "pc"))]
                let found = crate::virtio::net_dns_resolve(host);
                found
            } {
                Some(ip) => ip,
                None if stopped() != Stop::No => return stopped_page(t0),
                None => {
                    message_page(&[b"The name was not found (no answer from the name server).", u.host.get()]);
                    return set_status(&[b"name not found"]);
                }
            },
        };
        // condition 4: a redirect never takes Dira to a more private place than the server that sent it
        if let Some(f) = from {
            if !url::redirect_ok(f, ip) {
                let mut shown = Shown::new();
                url::show(&u, &mut shown);
                message_page(&[
                    b"The site sent Dira to a private address, and Dira did not go there by itself:",
                    shown.get(),
                    b"If you trust it, follow it as link 1 (type 1, then Enter).",
                ]);
                s.page.add_link(shown.get());
                return set_status(&[b"redirect to a private address not followed"]);
            }
        }
        let mut req = [0u8; crate::virtio::TCP_MAX_PAYLOAD];
        let Some(n) = http::request(&u, &mut req) else {
            message_page(&[b"This address is too long for Dira M1 (one request is at most 512 bytes)."]);
            return set_status(&[b"address too long"]);
        };
        if hops.len() > 0 {
            hops.push(b" -> ");
        }
        ip_text(ip, &mut hops);
        let Ok(_route) = route(ip, "server", t0) else { return };
        s.resp.reset();
        s.page.reset();
        let (resp, html, page) = (&mut s.resp, &mut s.html, &mut s.page);
        let mut started = false;
        let full = core::cell::Cell::new(false); // the page holds all it can: read no more
        let mut sink = |r: &Resp, bytes: &[u8]| {
            if !started {
                html.start(r.kind == Kind::Text, r.latin1);
                started = true;
            }
            html.feed(page, bytes);
            if page.cut {
                full.set(true);
            }
        };
        let mut handler = |input: &[u8], _out: &mut alloc::vec::Vec<u8>| resp.feed(input, &mut sink) && !full.get();
        crate::virtio::net_tcp_dial_with(ip, u.port, &req[..n], WAIT_MS, &mut handler);
        s.resp.closed();
        if stopped() != Stop::No {
            return stopped_page(t0);
        }
        if let Some(e) = s.resp.error {
            if started && s.page.n > 0 {
                // what arrived stays, with the reason it ends here (condition 1)
                s.html.finish(&mut s.page);
                s.page.note(b"[Dira: the page stops here: the rest could not be read]", html::R_DIM);
                s.page.note(e.as_bytes(), html::R_DIM);
            } else {
                message_page(&[b"The server's answer could not be read:", e.as_bytes()]);
            }
            return set_status(&[b"bad answer"]);
        }
        if !s.resp.have_headers() {
            message_page(&[
                b"No answer from the server.",
                b"Nothing may be listening at this address, or the network did not answer in 30 seconds.",
            ]);
            return set_status(&[b"no answer"]);
        }
        if s.resp.redirect() {
            if hop == 5 {
                message_page(&[b"The site sent Dira round more than 5 redirects; Dira stopped."]);
                return set_status(&[b"too many redirects"]);
            }
            match url::resolve(&u, s.resp.location.get()) {
                Ok(Some(next)) if !next.https => {
                    from = Some(ip);
                    u = next;
                    continue;
                }
                _ => {
                    message_page(&[b"The site redirected to an address Dira cannot open:", s.resp.location.get()]);
                    return set_status(&[b"bad redirect"]);
                }
            }
        }
        if s.resp.kind == Kind::Other {
            message_page(&[b"This address is a file, not a page:", s.resp.ctype.get(), b"Dira M1 shows web pages and plain text."]);
            return set_status(&[b"not a page"]);
        }
        if started {
            s.html.finish(&mut s.page);
        }
        page_notes();
        if !s.page.cut && s.resp.cut {
            s.page.note(b"[Dira: the page was cut here; M1 reads at most 4 MiB]", html::R_DIM);
        } else if !s.page.cut && !s.resp.complete() {
            s.page.note(b"[Dira: the page did not arrive in full]", html::R_DIM);
        }
        let mut st: Buf<100> = Buf::new();
        num(&mut st, s.resp.status as u64);
        st.push(b" ");
        st.push(phrase(s.resp.status));
        st.push(b", ");
        num(&mut st, s.resp.body_bytes);
        st.push(b" bytes in ");
        num(&mut st, ms_now().saturating_sub(t0));
        st.push(b" ms, at ");
        let room = 100 - st.len();
        st.push(&hops.get()[..hops.len().min(room)]);
        s.status = st;
        return;
    }
}

/// The closing lines a full page gets.
fn page_notes() {
    let p = &mut b().page;
    if p.cut {
        p.note(b"[Dira: the page was cut here; M1 keeps the first 48,000 letters]", html::R_DIM);
    }
    if p.links_full {
        p.note(b"[Dira: this page has more links than M1 can number; the rest are plain text]", html::R_DIM);
    }
}

/// The person stopped the load: say so, with how long the stop took.
fn stopped_page(t0: u64) {
    let now = ms_now();
    // SAFETY: as in stop_asked.
    let (why, at) = unsafe { (STOP, STOP_AT) };
    let what: &str = match why {
        Stop::Close => "the close box",
        Stop::Quit => "q",
        Stop::Esc => "Esc",
        Stop::CtrlC => "Ctrl+C",
        Stop::No => "?",
    };
    crate::kprintln!(
        "[dira] stopped loading ({}): {} ms after it was asked, {} ms after the load began",
        what,
        now.saturating_sub(at),
        now.saturating_sub(t0)
    );
    message_page(&[b"Stopped. The page was not loaded."]);
    set_status(&[b"stopped"]);
}

/// Heap use now, for the "no heap above one segment" proof (architect's Q3).
fn heap_line(when: &str) {
    crate::kprintln!(
        "dira: heap {}: {} bytes in use, high-water {} bytes",
        when,
        crate::heap::used(),
        crate::heap::high_water()
    );
}

/// A letter as the console shows it (the box becomes '?').
fn console_byte(c: u8) -> u8 {
    if (0x20..0x7f).contains(&c) {
        c
    } else {
        b'?'
    }
}

/// The loaded page on the console: a header line, the text wrapped at 80, then the links.
fn print_page() {
    let s = b();
    let mut shown = Shown::new();
    if let Some(l) = &nav().loc {
        loc_text(l, &mut shown);
    }
    crate::kprintln!(
        "dira: {}: {}",
        core::str::from_utf8(shown.get()).unwrap_or("?"),
        core::str::from_utf8(s.status.get()).unwrap_or("?")
    );
    if s.page.title.len() > 0 {
        crate::kprintln!("dira: title: {}", core::str::from_utf8(s.page.title.get()).unwrap_or("?"));
    }
    const W: usize = 80;
    let rows = html::wrap(&s.page, W, &mut s.starts);
    let mut line = [0u8; W];
    for r in 0..rows {
        let (a, e) = html::row(&s.page, &s.starts, rows, r, W);
        let n = (e - a).min(W);
        for k in 0..n {
            line[k] = console_byte(s.page.at(a + k).0);
        }
        crate::kprintln!("| {}", core::str::from_utf8(&line[..n]).unwrap_or(""));
    }
    if rows == ROWS_MAX {
        crate::kprintln!("dira: (only the first {} rows are shown)", ROWS_MAX);
    }
    for k in 1..=s.page.nlinks {
        let href = s.page.link(k).unwrap_or(b"");
        let mut out = Shown::new();
        let what: &[u8] = match &nav().loc {
            Some(Loc::Net(base)) => match url::resolve(base, href) {
                Ok(Some(u)) => {
                    url::show(&u, &mut out);
                    out.get()
                }
                Ok(None) => b"(a place in this page)",
                Err(e) => e.as_bytes(),
            },
            _ => href,
        };
        crate::kprintln!("dira: link [{}] {}", k, core::str::from_utf8(what).unwrap_or("?"));
    }
    crate::kprintln!("dira: done: {} letters, {} rows, {} links", s.page.n, rows, s.page.nlinks);
}

fn parse_loc(arg: &str) -> Result<Loc, &'static str> {
    let a = arg.trim();
    if let Some(name) = a.strip_prefix("file:") {
        let mut n: Buf<NAME_MAX> = Buf::new();
        if name.is_empty() || !n.push(name.as_bytes()) {
            return Err("a file name is 1 to 24 letters");
        }
        return Ok(Loc::File(n));
    }
    url::parse(a.as_bytes()).map(Loc::Net)
}

const USAGE: &str = "dira: usage: dira <address>   (a page in a desk window, e.g. dira http://10.0.2.2:8000/)\n      dira text <address>   (the page on the console)\n      dira file <name>      (an .html or .txt file from the store)";

/// The shell command.
pub fn run(args: &str) {
    let args = args.trim();
    let (text, rest) = match args.split_once(' ') {
        Some(("text", r)) => (true, r.trim()),
        _ if args == "text" => (true, ""),
        _ => (false, args),
    };
    let target = match rest.split_once(' ') {
        Some(("file", n)) => parse_loc(&alloc::format!("file:{}", n.trim())),
        _ if rest.is_empty() => return crate::kprintln!("{}", USAGE),
        _ if rest == "file" => return crate::kprintln!("{}", USAGE),
        _ => parse_loc(rest),
    };
    let target = match target {
        Ok(t) => t,
        Err(e) => return crate::kprintln!("dira: {}", e),
    };
    if RUNNING.swap(true, Ordering::Acquire) {
        return crate::kprintln!("dira: Dira is already running; one at a time");
    }
    nav().nhist = 0;
    // SAFETY: mainline only, and RUNNING is ours.
    unsafe { STOP = Stop::No };
    if text || !crate::desk::shown() {
        if !text {
            crate::kprintln!("dira: the desk is not showing, so the page comes here (open the desk with: desk)");
        }
        heap_line("before");
        load(target, false);
        print_page();
        heap_line("after");
    } else {
        window(target);
    }
    RUNNING.store(false, Ordering::Release);
}

/// Condition 7: the statics above belong to one Dira at a time.
static RUNNING: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------------------------------------------
// The desk window
// ---------------------------------------------------------------------------------------------------------------

/// Bar colours: the address bar on the selection background, the status row on the cursor-line one.
const A_BAR: u8 = 0x90 | 10;
const A_BAR_DIM: u8 = 0x90 | 11;
const A_FOOT: u8 = 0xC0 | 11;
const A_FOOT_HOT: u8 = 0xC0 | 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Address,
    Number,
}

struct Win {
    cols: usize,
    rows: usize,
    top: usize,
    mode: Mode,
    esc: u8,
}

fn grid() -> (usize, usize) {
    let g = crate::desk::app_cell_size(abi::WIN);
    if g < 0 {
        return (0, 0);
    }
    let g = g as u32;
    (((g >> 14) & 0x3FFF) as usize, (g & 0x3FFF) as usize)
}

/// One row of cells; text past the grid is clipped.
struct Row {
    c: [u8; COLS_MAX * 2],
    n: usize,
    cols: usize,
}

impl Row {
    fn new(cols: usize, attr: u8) -> Row {
        let cols = cols.min(COLS_MAX);
        let mut r = Row { c: [0; COLS_MAX * 2], n: 0, cols };
        for k in 0..cols {
            r.c[2 * k] = b' ';
            r.c[2 * k + 1] = attr;
        }
        r
    }
    fn put(&mut self, s: &[u8], attr: u8) {
        for &ch in s {
            if self.n >= self.cols {
                return;
            }
            self.c[2 * self.n] = if (0x20..0x7f).contains(&ch) || ch == 0x80 { ch } else { 0x80 };
            self.c[2 * self.n + 1] = attr;
            self.n += 1;
        }
    }
    fn show(&self, r: usize) {
        crate::desk::app_cell_put(abi::WIN, 0, r as i32, &self.c[..2 * self.cols]);
    }
}

fn rewrap(w: &mut Win) {
    let s = b();
    let width = w.cols.saturating_sub(2).max(8);
    s.width = width;
    s.nrows = html::wrap(&s.page, width, &mut s.starts);
    let body = w.rows.saturating_sub(2).max(1);
    w.top = w.top.min(s.nrows.saturating_sub(body));
}

fn draw_bar(w: &Win) {
    let s = b();
    let mut r = Row::new(w.cols, A_BAR);
    if w.mode == Mode::Address {
        r.put(b" Go to: ", A_BAR_DIM);
        r.put(s.typed.get(), A_BAR);
        let at = r.n;
        r.show(0);
        crate::desk::app_cell_cursor(abi::WIN, at.min(w.cols.saturating_sub(1)) as i32, 0, 1);
        return;
    }
    let mut shown = Shown::new();
    if let Some(l) = &nav().loc {
        loc_text(l, &mut shown);
    }
    r.put(b" ", A_BAR);
    r.put(shown.get(), A_BAR); // the address Dira built and checked; nothing from the page reaches row 0
    if matches!(nav().loc, Some(Loc::Net(_))) {
        // said plainly, right-aligned: nothing in M1 is encrypted
        let note = b"plain HTTP, not private ";
        if r.n + note.len() + 2 <= w.cols {
            r.n = w.cols - note.len();
            r.put(note, A_BAR_DIM);
        }
    }
    r.show(0);
    crate::desk::app_cell_cursor(abi::WIN, 0, 0, 0);
}

fn draw_foot(w: &Win) {
    let s = b();
    let mut r = Row::new(w.cols, A_FOOT);
    match w.mode {
        Mode::Number => {
            r.put(b" Follow link: ", A_FOOT_HOT);
            r.put(s.typed.get(), A_FOOT_HOT);
            r.put(b"   Enter goes, Esc cancels", A_FOOT);
        }
        Mode::Address => r.put(b" Type an address. Enter goes, Esc cancels", A_FOOT),
        Mode::Browse => {
            r.put(b" ", A_FOOT);
            r.put(s.status.get(), A_FOOT_HOT);
            r.put(b"   Space/b j/k  N Enter link  g go  h back  r reload  q quit", A_FOOT);
        }
    }
    r.show(w.rows.saturating_sub(1));
}

fn draw_body(w: &Win) {
    let s = b();
    let body = w.rows.saturating_sub(2);
    for k in 0..body {
        let mut r = Row::new(w.cols, 0);
        let i = w.top + k;
        if i < s.nrows {
            let (a, e) = html::row(&s.page, &s.starts, s.nrows, i, s.width);
            r.n = 1; // a one-cell margin
            for j in a..e {
                let (c, role) = s.page.at(j);
                r.put(&[c], role & 15);
            }
        } else if i == s.nrows && s.nrows == ROWS_MAX {
            r.n = 1;
            r.put(b"[Dira: only the first 4,096 rows of this page are shown]", html::R_DIM);
        }
        r.show(1 + k);
    }
}

fn draw_all(w: &Win) {
    if w.cols < 10 || w.rows < 3 {
        return;
    }
    draw_bar(w);
    draw_body(w);
    draw_foot(w);
}

/// Load a page in the window: the bar says what is loading, then the page is drawn from its top. False when the
/// person closed Dira while the page loaded.
fn go(w: &mut Win, to: Loc, remember: bool) -> bool {
    let s = b();
    if remember {
        if let Some(cur) = nav().loc {
            if nav().nhist == HIST {
                nav().hist.copy_within(1.., 0);
                nav().nhist -= 1;
            }
            nav().hist[nav().nhist] = Some(cur);
            nav().nhist += 1;
        }
    }
    w.mode = Mode::Browse;
    nav().loc = Some(to);
    s.page.reset();
    s.nrows = 0;
    w.top = 0;
    let mut shown = Shown::new();
    loc_text(&to, &mut shown);
    set_status(&[b"Loading ", shown.get()]);
    draw_all(w);
    load(to, true);
    rewrap(w);
    let mut shown = Shown::new();
    if let Some(l) = &nav().loc {
        loc_text(l, &mut shown);
    }
    crate::kprintln!(
        "[dira] {}: {}; {} letters, {} rows, {} links",
        core::str::from_utf8(shown.get()).unwrap_or("?"),
        core::str::from_utf8(s.status.get()).unwrap_or("?"),
        s.page.n,
        s.nrows,
        s.page.nlinks
    );
    if matches!(stopped(), Stop::Close | Stop::Quit) {
        return false;
    }
    draw_all(w);
    true
}

/// Follow link k; false when Dira was closed while the page loaded.
fn follow(w: &mut Win, k: usize) -> bool {
    let s = b();
    let Some(href) = s.page.link(k) else {
        set_status(&[b"there is no link with that number"]);
        draw_foot(w);
        return true;
    };
    let mut h: Buf<{ html::LINK_MAX }> = Buf::new();
    h.push(href);
    let target = match &nav().loc {
        Some(Loc::Net(base)) => url::resolve(base, h.get()),
        _ => url::parse(h.get()).map(Some).map_err(|_| "a link in a file page needs a full http:// address"),
    };
    match target {
        Ok(Some(u)) => {
            crate::kprintln!("[dira] follow link [{}]", k);
            go(w, Loc::Net(u), true)
        }
        Ok(None) => {
            set_status(&[b"that link points to a place in this page"]);
            draw_foot(w);
            true
        }
        Err(e) => {
            set_status(&[e.as_bytes()]);
            draw_foot(w);
            true
        }
    }
}

/// What a key asks for; false ends Dira.
fn key(w: &mut Win, k: u32) -> bool {
    let s = b();
    // ESC [ A/B/5~/6~ from a serial terminal: up, down, page up, page down
    match (w.esc, k) {
        (1, 0x5B) => {
            w.esc = 2;
            return true;
        }
        (2, 0x41) => return {
            w.esc = 0;
            scroll(w, -1)
        },
        (2, 0x42) => return {
            w.esc = 0;
            scroll(w, 1)
        },
        (2, 0x35) | (2, 0x36) => {
            w.esc = if k == 0x35 { 3 } else { 4 };
            return true;
        }
        (3, 0x7E) | (4, 0x7E) => {
            let page = w.rows.saturating_sub(3).max(1) as isize;
            let d = if w.esc == 3 { -page } else { page };
            w.esc = 0;
            return scroll(w, d);
        }
        (1, _) => {
            // a lone Esc
            w.esc = 0;
            if w.mode != Mode::Browse {
                w.mode = Mode::Browse;
                s.typed.clear();
                draw_bar(w);
                draw_foot(w);
            }
        }
        (2.., _) => {
            w.esc = 0;
            return true;
        }
        _ => {}
    }
    if k == 27 {
        w.esc = 1;
        return true;
    }
    let c = k as u8;
    let enter = k == 13 || k == 10;
    let back = k == 8 || k == 127;
    match w.mode {
        Mode::Address => {
            if enter {
                let mut t: Buf<{ url::PATH_MAX }> = Buf::new();
                t.push(s.typed.get());
                s.typed.clear();
                w.mode = Mode::Browse;
                match parse_loc(core::str::from_utf8(t.get()).unwrap_or("")) {
                    Ok(l) => return go(w, l, true),
                    Err(e) => {
                        set_status(&[e.as_bytes()]);
                        draw_bar(w);
                        draw_foot(w);
                    }
                }
            } else if back {
                s.typed.pop();
                draw_bar(w);
            } else if k < 0x80 && (0x20..0x7f).contains(&c) {
                s.typed.push_byte(c);
                draw_bar(w);
            }
        }
        Mode::Number => {
            if enter {
                let n = s.typed.get().iter().fold(0usize, |a, &d| a * 10 + (d - b'0') as usize);
                s.typed.clear();
                w.mode = Mode::Browse;
                return follow(w, n);
            } else if back {
                s.typed.pop();
                if s.typed.len() == 0 {
                    w.mode = Mode::Browse;
                }
                draw_foot(w);
            } else if c.is_ascii_digit() && k < 0x80 {
                if s.typed.len() < 3 {
                    s.typed.push_byte(c);
                }
                draw_foot(w);
            }
        }
        Mode::Browse => {
            let page = w.rows.saturating_sub(3).max(1) as isize;
            match k {
                0x20 | 0x66 => return scroll(w, page),          // Space, f
                0x62 => return scroll(w, -page),                // b
                0x6A => return scroll(w, 1),                    // j
                0x6B => return scroll(w, -1),                   // k
                0x47 => return scroll(w, isize::MAX / 2),       // G: the end
                0x74 => return scroll(w, -(isize::MAX / 2)),    // t: the top
                0x71 => return false,                           // q
                0x67 => {
                    // g
                    w.mode = Mode::Address;
                    s.typed.clear();
                    draw_bar(w);
                    draw_foot(w);
                }
                0x72 => {
                    // r
                    if let Some(l) = nav().loc {
                        return go(w, l, false);
                    }
                }
                0x68 | 8 | 127 => {
                    // h, Backspace: back
                    if nav().nhist == 0 {
                        set_status(&[b"nothing to go back to"]);
                        draw_foot(w);
                    } else {
                        nav().nhist -= 1;
                        if let Some(l) = nav().hist[nav().nhist].take() {
                            crate::kprintln!("[dira] back");
                            return go(w, l, false);
                        }
                    }
                }
                0x30..=0x39 => {
                    w.mode = Mode::Number;
                    s.typed.clear();
                    s.typed.push_byte(c);
                    draw_foot(w);
                }
                _ => {}
            }
        }
    }
    true
}

fn scroll(w: &mut Win, d: isize) -> bool {
    let s = b();
    let body = w.rows.saturating_sub(2).max(1);
    let max = s.nrows.saturating_sub(body);
    let top = if d < 0 { w.top.saturating_sub(d.unsigned_abs()) } else { w.top.saturating_add(d as usize).min(max) };
    if top != w.top {
        w.top = top;
        draw_body(w);
    }
    true
}

fn window(target: Loc) {
    let r = crate::desk::app_cell_open(b"Dira (built-in)", true, 84, 30);
    // The title says built-in in words (architect's Q2). No Nia app can show it: their names are file names ending
    // .wasm, with no spaces or brackets.
    if r < 0 {
        let why = match r {
            abi::ERR_ALREADY_OPEN => "another app has the cell window open",
            abi::ERR_NO_DESK => "the desk is not showing",
            _ => "the desk refused the window",
        };
        return crate::kprintln!("dira: cannot open a window: {}", why);
    }
    let (cols, rows) = grid();
    let mut w = Win { cols: cols.min(COLS_MAX), rows, top: 0, mode: Mode::Browse, esc: 0 };
    crate::kprintln!("[dira] window {}x{}", w.cols, w.rows);
    let mut open = go(&mut w, target, false);
    while open {
        let ev = crate::desk::app_cell_event(-1);
        if ev < 0 {
            break;
        }
        let ev = ev as u32;
        let (kind, payload) = ((ev >> 28) & 7, ev & abi::PAYLOAD);
        match kind {
            abi::EV_KEY => open = key(&mut w, payload),
            abi::EV_RESIZE => {
                let (c, r) = (((payload >> 14) & 0x3FFF) as usize, (payload & 0x3FFF) as usize);
                w.cols = c.min(COLS_MAX);
                w.rows = r;
                rewrap(&mut w);
                draw_all(&w);
            }
            abi::EV_CLOSE_REQUEST | abi::EV_CLOSED => {
                // SAFETY: mainline only.
                unsafe { STOP = Stop::Close };
                open = false;
            }
            _ => {}
        }
    }
    crate::kprintln!("[dira] closed ({})", if stopped() == Stop::Close { "the close box" } else { "q" });
    crate::desk::cell_app_ended();
}
