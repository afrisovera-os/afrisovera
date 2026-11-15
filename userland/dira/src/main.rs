//! Dira, AfriSovera's own web browser, as a ring-3 process on the PC (Dira M3 S4; AFR-0001 7.6, the Dira subset).
//!
//! The parsers are the kernel's files compiled unchanged (url.rs, http.rs, html.rs, through `#[path]`, as dira-host
//! does), so the code that runs is the code that is fuzzed. This file is M1's kernel side (kernel/src/dira/mod.rs)
//! moved out of the kernel: every kernel call is now one of the v0.5 calls, and nothing else.
//!   - the window: cell_open, cell_put, cell_cursor, cell_size, cell_event (64 to 68), on the CELL handle;
//!   - the network: tcp_dial, tcp_send, tcp_recv, tcp_close, dns_a (72 to 76), on the NET handle;
//!   - memory: one memory object of 1 MiB (vmo_create, vmo_map), which holds every buffer and the stack;
//!   - the console: putc (11), for `dira text` and the lines the suites read.
//! The kernel starts Dira with a block at the start of its data page (pc/dira_proc.rs): the mode, which handle is
//! which, the address as typed, the TSC frequency, and, for `dira file`, a store file mapped read-only. A crash here
//! ends Dira, not the system.
#![no_std]
#![no_main]
#![allow(clippy::needless_range_loop)]

#[path = "../../../kernel/src/dira/html.rs"]
mod html;
#[path = "../../../kernel/src/dira/http.rs"]
mod http;
#[path = "../../../kernel/src/dira/url.rs"]
mod url;

use core::arch::{asm, global_asm};
use core::panic::PanicInfo;
use html::{Html, Page};
use http::{Kind, Resp};
use url::{Buf, Shown, Url};

// ------------------------------------------------------------------------------------------- the kernel calls --

const SYS_EXIT: u64 = 0;
const SYS_PUTC: u64 = 11;
const SYS_VMO_CREATE: u64 = 32;
const SYS_VMO_MAP: u64 = 33;
const SYS_CELL_OPEN: u64 = 64;
const SYS_CELL_PUT: u64 = 65;
const SYS_CELL_CURSOR: u64 = 66;
const SYS_CELL_SIZE: u64 = 67;
const SYS_CELL_EVENT: u64 = 68;
const SYS_TCP_DIAL: u64 = 72;
const SYS_TCP_SEND: u64 = 73;
const SYS_TCP_RECV: u64 = 74;
const SYS_TCP_CLOSE: u64 = 75;
const SYS_DNS_A: u64 = 76;

const E_PERM: i64 = -1;
const E_WOULDBLOCK: i64 = -7;
const E_PEERCLOSED: i64 = -8;
const E_TOOBIG: i64 = -9;
const E_CANCELED: i64 = -10;
const E_NOROUTE: i64 = -11;

/// The cell window's events (desk cell_abi): the kind in bits 28 to 30, the payload below.
const EV_KEY: u32 = 1;
const EV_RESIZE: u32 = 2;
const EV_CLOSE_REQUEST: u32 = 3;
const EV_CLOSED: u32 = 4;
const PAYLOAD: u32 = (1 << 28) - 1;

/// One request is at most one segment (virtio TCP_MAX_PAYLOAD).
const REQ_MAX: usize = 512;
/// One receive is at most one segment (MSS 1460).
const SEG: usize = 1460;

#[inline(always)]
fn sys(n: u64, a: u64, b: u64, c: u64, d: u64, e: u64) -> i64 {
    let r: u64;
    // SAFETY: `int 0x80` is the PC kernel's call gate (pc/proc.rs): it reads rax and rdi, rsi, rdx, rcx, r8, and
    // writes rax only; every other register comes back as it was.
    unsafe {
        asm!("int 0x80", inlateout("rax") n => r, in("rdi") a, in("rsi") b, in("rdx") c, in("rcx") d, in("r8") e,
             options(nostack));
    }
    r as i64
}

fn exit(code: u64) -> ! {
    sys(SYS_EXIT, code, 0, 0, 0, 0);
    loop {
        core::hint::spin_loop();
    }
}

fn putc(c: u8) {
    sys(SYS_PUTC, c as u64, 0, 0, 0, 0);
}

/// One console line made of `parts` (the kernel's `kprintln!` for a process).
fn out(parts: &[&[u8]]) {
    for p in parts {
        for &c in *p {
            putc(c);
        }
    }
    putc(b'\n');
}

/// A number as decimal text.
struct Num {
    d: [u8; 20],
    n: usize,
}

impl Num {
    fn new(mut v: u64) -> Num {
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
        Num { d, n }
    }
    fn get(&self) -> &[u8] {
        &self.d[20 - self.n..]
    }
}

// --------------------------------------------------------------------------------------- the kernel's block --

const USER_BASE: u64 = 0x100_0000_0000;
const DATA_VA: u64 = USER_BASE + 0x2_0000;
const FILE_VA: u64 = USER_BASE + 0x2_4000;
const BLOCK_MAGIC: u32 = 0x4E50_5344; // "DSPN"
const MODE_TEXT: u8 = 1;
/// The kernel was started by the test rig (fw_cfg opt/afrisovera/test-grants): the two test addresses below work.
const MODE_RIG: u8 = 2;
const NONE_H: u8 = 0xFF;
const NO_FILE: u32 = u32::MAX;
const STOP_CLOSE: u32 = 1;
const STOP_CTRLC: u32 = 2;

/// What the kernel wrote at the start of the data page before Dira's first instruction (pc/dira_proc.rs, `Block`).
#[repr(C)]
struct Block {
    magic: u32,
    mode: u8,
    cell_h: u8,
    net_h: u8,
    _r0: u8,
    arg_len: u32,
    file_len: u32,
    tsc_hz: u64,
    /// written by the kernel when it ends a wait with E_CANCELED: the TSC when it saw the stop, and which stop
    stop_tsc: u64,
    stop_kind: u32,
    _r1: [u8; 28],
    arg: [u8; 512],
}

fn block() -> &'static Block {
    // SAFETY: the kernel maps the data page at DATA_VA before the first instruction and writes the block there; it
    // only ever changes stop_tsc and stop_kind, inside a call of ours.
    unsafe { &*(DATA_VA as *const Block) }
}

fn stop_seen() -> (u64, u32) {
    // SAFETY: as block(); read volatile because the kernel writes them during our calls.
    unsafe {
        let b = DATA_VA as *const Block;
        (
            core::ptr::read_volatile(core::ptr::addr_of!((*b).stop_tsc)),
            core::ptr::read_volatile(core::ptr::addr_of!((*b).stop_kind)),
        )
    }
}

fn tsc() -> u64 {
    // SAFETY: rdtsc is allowed in ring 3 (CR4.TSD clear); it reads the counter only.
    unsafe { core::arch::x86_64::_rdtsc() }
}

fn ms_of(t: u64) -> u64 {
    (t as u128 * 1000 / block().tsc_hz.max(1) as u128) as u64
}

fn ms_now() -> u64 {
    ms_of(tsc())
}

// ------------------------------------------------------------------------------------------------ the memory --

/// The one memory object: the state at its start, the stack at its end.
const MEM: u64 = 1 << 20;
/// The map area's first address: the first object mapped lands there (first fit, AFR-0001 7.6).
const MAP_LO: u64 = USER_BASE + 0x4000_0000;
const STATE_MAX: usize = 640 * 1024;

/// Rows of the wrapped page Dira can show; a longer page shows a line saying so.
const ROWS_MAX: usize = 4096;
const HIST: usize = 8;
const WAIT_MS: u64 = 30_000;
const NAME_MAX: usize = 24;
const COLS_MAX: usize = 120;

#[derive(Clone, Copy)]
enum Loc {
    Net(Url),
    File(Buf<NAME_MAX>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    No,
    Close,
    Quit,
    Esc,
    CtrlC,
}

struct State {
    page: Page,
    html: Html,
    resp: Resp,
    starts: [u32; ROWS_MAX],
    nrows: usize,
    width: usize,
    status: Buf<100>,
    typed: Buf<{ url::PATH_MAX }>,
    loc: Option<Loc>,
    hist: [Option<Loc>; HIST],
    nhist: usize,
    stop: Stop,
    stop_at: u64,
    window: bool,
}

const _: () = assert!(core::mem::size_of::<State>() <= STATE_MAX);

/// The state, at the start of the memory object (checked at start). Dira is one thread; nothing else touches it.
fn b() -> &'static mut State {
    // SAFETY: `dira_start` maps the object at MAP_LO and writes every field before anything calls this.
    unsafe { &mut *(MAP_LO as *mut State) }
}

global_asm!(
    r#"
.section .text.entry
.global _start
_start:
    jmp dira_start
"#
);

#[no_mangle]
extern "C" fn dira_start() -> ! {
    if block().magic != BLOCK_MAGIC {
        out(&[b"dira: started without the kernel's block"]);
        exit(2);
    }
    let h = sys(SYS_VMO_CREATE, MEM, 0, 0, 0, 0);
    let at = if h >= 0 { sys(SYS_VMO_MAP, h as u64, 0, MEM, 3, 0) } else { h };
    if at != MAP_LO as i64 {
        out(&[b"dira: no memory for Dira's buffers"]);
        exit(1);
    }
    // SAFETY: the object is mapped read-write at MAP_LO for MEM bytes and is ours alone; the stack moves to its end
    // (16-byte aligned) and never comes back, as dira_main does not return.
    unsafe {
        asm!("mov rsp, {top}", "call {f}", "ud2", top = in(reg) MAP_LO + MEM, f = in(reg) dira_main as extern "C" fn() -> !,
             options(noreturn));
    }
}

extern "C" fn dira_main() -> ! {
    // SAFETY: every field written in place, so no 150 KiB value is built on the stack or kept in the binary.
    unsafe {
        let s = MAP_LO as *mut State;
        core::ptr::addr_of_mut!((*s).page).write(Page::new());
        core::ptr::addr_of_mut!((*s).html).write(Html::new());
        core::ptr::addr_of_mut!((*s).resp).write(Resp::new());
        core::ptr::addr_of_mut!((*s).starts).write([0; ROWS_MAX]);
        core::ptr::addr_of_mut!((*s).nrows).write(0);
        core::ptr::addr_of_mut!((*s).width).write(0);
        core::ptr::addr_of_mut!((*s).status).write(Buf::new());
        core::ptr::addr_of_mut!((*s).typed).write(Buf::new());
        core::ptr::addr_of_mut!((*s).loc).write(None);
        core::ptr::addr_of_mut!((*s).hist).write([None; HIST]);
        core::ptr::addr_of_mut!((*s).nhist).write(0);
        core::ptr::addr_of_mut!((*s).stop).write(Stop::No);
        core::ptr::addr_of_mut!((*s).stop_at).write(0);
        core::ptr::addr_of_mut!((*s).window).write(false);
    }
    run();
    exit(0);
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    out(&[b"dira: Dira stopped on an internal error (a check in its code failed)"]);
    exit(70);
}

// ------------------------------------------------------------------------------------------------- the pages --

fn ip_text<const N: usize>(ip: [u8; 4], o: &mut Buf<N>) {
    for (k, part) in ip.iter().enumerate() {
        o.push(Num::new(*part as u64).get());
        if k < 3 {
            o.push_byte(b'.');
        }
    }
}

fn loc_text(l: &Loc, o: &mut Shown) {
    match l {
        Loc::Net(u) => url::show(u, o),
        Loc::File(n) => {
            o.clear();
            o.push(b"file:");
            o.push(n.get());
        }
    }
}

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

fn page_notes() {
    let p = &mut b().page;
    if p.cut {
        p.note(b"[Dira: the page was cut here; M1 keeps the first 48,000 letters]", html::R_DIM);
    }
    if p.links_full {
        p.note(b"[Dira: this page has more links than M1 can number; the rest are plain text]", html::R_DIM);
    }
}

/// The person stopped the load: say so, with how long the stop took. For a stop the kernel saw (a close request, or
/// Ctrl+C on the console) the time runs from when the kernel saw it; for Esc and q, from when Dira read the key.
fn stopped_page(t0: u64) {
    let s = b();
    let now = ms_now();
    let what: &[u8] = match s.stop {
        Stop::Close => b"the close box",
        Stop::Quit => b"q",
        Stop::Esc => b"Esc",
        Stop::CtrlC => b"Ctrl+C",
        Stop::No => b"?",
    };
    out(&[
        b"[dira] stopped loading (",
        what,
        b"): ",
        Num::new(now.saturating_sub(s.stop_at)).get(),
        b" ms after it was asked, ",
        Num::new(now.saturating_sub(t0)).get(),
        b" ms after the load began",
    ]);
    message_page(&[b"Stopped. The page was not loaded."]);
    set_status(&[b"stopped"]);
}

/// A wait ended with E_CANCELED: the kernel saw a close request (or Ctrl+C on the console) and says when.
fn canceled() {
    let s = b();
    let (t, kind) = stop_seen();
    s.stop = if kind == STOP_CTRLC { Stop::CtrlC } else { Stop::Close };
    s.stop_at = if t != 0 { ms_of(t) } else { ms_now() };
}

/// Between two waits of a load in the window: read what the person typed. A close ends Dira, q too, Esc stops the
/// load; other keys wait for the page (they are dropped, as in M1). True when the load must stop.
fn keys_say_stop() -> bool {
    let s = b();
    if !s.window || s.stop != Stop::No {
        return s.stop != Stop::No;
    }
    let cell = block().cell_h as u64;
    for _ in 0..64 {
        let ev = sys(SYS_CELL_EVENT, cell, 0, 0, 0, 0);
        if ev == 0 {
            break;
        }
        let why = if ev < 0 {
            Stop::Close
        } else {
            let ev = ev as u32;
            match ((ev >> 28) & 7, ev & PAYLOAD) {
                (EV_CLOSE_REQUEST, _) | (EV_CLOSED, _) => Stop::Close,
                (EV_KEY, 0x71) => Stop::Quit,
                (EV_KEY, 27) => Stop::Esc,
                _ => Stop::No,
            }
        };
        if why != Stop::No {
            s.stop = why;
            s.stop_at = ms_now();
            return true;
        }
    }
    false
}

/// The network's answer to one of Dira's waits that failed: a page that says what is missing.
fn net_failed(e: i64, t0: u64, what: &[u8]) {
    if e == E_CANCELED {
        canceled();
        return stopped_page(t0);
    }
    if b().stop != Stop::No {
        return stopped_page(t0);
    }
    match e {
        E_NOROUTE => {
            message_page(&[
                b"The network did not answer: the router gave this computer no address (no answer to DHCP),",
                b"or the router, or the computer at this address on the local network, did not answer.",
            ]);
            set_status(&[b"no route on the local network"]);
        }
        E_PERM => {
            message_page(&[b"Dira does not open this address: it is not one a web page may reach.", what]);
            set_status(&[b"address refused"]);
        }
        E_WOULDBLOCK => {
            message_page(&[b"Another part of the system is using the network right now. Try again."]);
            set_status(&[b"network busy"]);
        }
        _ => {
            message_page(&[
                b"No answer from the server.",
                b"Nothing may be listening at this address, or the network did not answer in 30 seconds.",
            ]);
            set_status(&[b"no answer"]);
        }
    }
}

/// A store file the kernel mapped for `dira file`: its bytes, if the kernel found it under this name.
fn mapped_file(name: &[u8]) -> Option<&'static [u8]> {
    let blk = block();
    if blk.file_len == NO_FILE {
        return None;
    }
    let arg = &blk.arg[..(blk.arg_len as usize).min(512)];
    let named = arg.strip_prefix(b"file:").map(|n| n.trim_ascii());
    if named != Some(name) {
        return None;
    }
    // SAFETY: the kernel mapped file_len bytes (at most 16 KiB) read-only at FILE_VA before Dira started.
    Some(unsafe { core::slice::from_raw_parts(FILE_VA as *const u8, blk.file_len as usize) })
}

/// Fetch and read a page into the buffers, following up to 5 redirects. The location shown becomes `loc`.
fn load(start: Loc) {
    let s = b();
    let t0 = ms_now();
    s.status.clear();
    s.stop = Stop::No;
    let mut u = match start {
        Loc::File(name) => {
            s.loc = Some(start);
            let n = name.get();
            match mapped_file(n) {
                None => message_page(&[
                    b"There is no file by that name in the store.",
                    b"The Files window lists the files there; dira file <name> opens one of them.",
                ]),
                Some(bytes) => {
                    s.page.reset();
                    s.html.start(n.ends_with(b".txt"), false);
                    s.html.feed(&mut s.page, bytes);
                    s.html.finish(&mut s.page);
                    page_notes();
                    let mut st: Buf<100> = Buf::new();
                    st.push(b"read from the store, ");
                    st.push(Num::new(bytes.len() as u64).get());
                    st.push(b" bytes");
                    s.status = st;
                }
            }
            return;
        }
        Loc::Net(u) => u,
    };
    let net = block().net_h as u64;
    let mut from: Option<[u8; 4]> = None;
    let mut hops: Buf<72> = Buf::new();
    for hop in 0..=5 {
        s.loc = Some(Loc::Net(u));
        if u.https {
            message_page(&[
                b"Secure pages (https://) are not supported yet.",
                b"They come when Dira runs as its own protected program, outside the system's core.",
                b"If the site also answers on plain http://, type that address with g.",
            ]);
            return set_status(&[b"https is not supported yet"]);
        }
        if block().net_h == NONE_H {
            message_page(&[b"This device has no network card Dira can use."]);
            return set_status(&[b"no network"]);
        }
        let host = u.host.get();
        let ip = match url::ipv4(&u) {
            Some(ip) => ip,
            None => {
                let r = sys(SYS_DNS_A, net, host.as_ptr() as u64, host.len() as u64, 10_000, 0);
                if r >= 0 {
                    (r as u32).to_be_bytes()
                } else if r == E_PEERCLOSED {
                    message_page(&[b"The name was not found (no answer from the name server).", host]);
                    return set_status(&[b"name not found"]);
                } else {
                    return net_failed(r, t0, host);
                }
            }
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
        let mut req = [0u8; REQ_MAX];
        let Some(n) = http::request(&u, &mut req) else {
            message_page(&[b"This address is too long for Dira M1 (one request is at most 512 bytes)."]);
            return set_status(&[b"address too long"]);
        };
        if hops.len() > 0 {
            hops.push(b" -> ");
        }
        ip_text(ip, &mut hops);
        let ipn = u32::from_be_bytes(ip) as u64;
        let c = sys(SYS_TCP_DIAL, net, ipn, u.port as u64, 10_000, 0); // a real router's DHCP can take seconds
        if c < 0 {
            let mut where_: Buf<16> = Buf::new();
            ip_text(ip, &mut where_);
            return net_failed(c, t0, where_.get());
        }
        let conn = c as u64;
        s.resp.reset();
        s.page.reset();
        let sent = sys(SYS_TCP_SEND, conn, req.as_ptr() as u64, n as u64, 0, 0);
        let mut started = false;
        let mut err: i64 = if sent < 0 { sent } else { 0 };
        let mut seg = [0u8; SEG];
        let until = t0 + WAIT_MS;
        while err == 0 {
            let r = sys(SYS_TCP_RECV, conn, seg.as_mut_ptr() as u64, SEG as u64, 50, 0);
            if r == 0 {
                break; // the server closed: the page is all here
            }
            if r == E_WOULDBLOCK {
                if keys_say_stop() {
                    break;
                }
                if ms_now() >= until {
                    break;
                }
                continue;
            }
            if r < 0 {
                err = r;
                break;
            }
            let (resp, html, page) = (&mut s.resp, &mut s.html, &mut s.page);
            let mut full = false;
            let mut sink = |rr: &Resp, bytes: &[u8]| {
                if !started {
                    html.start(rr.kind == Kind::Text, rr.latin1);
                    started = true;
                }
                html.feed(page, bytes);
                if page.cut {
                    full = true;
                }
            };
            let more = resp.feed(&seg[..r as usize], &mut sink);
            if !more || full {
                break; // the answer is complete, or the page holds all it can: read no more
            }
        }
        // close it at once: a page Dira stopped reading must not hold the shell (DIRA_OPEN_SLOW_CLOSE.md); a load the
        // person stopped is reset at once (how = 1), not left waiting 200 ms on a server that says nothing
        let how = if s.stop != Stop::No { 1 } else { 0 };
        sys(SYS_TCP_CLOSE, conn, how, 0, 0, 0);
        s.resp.closed();
        if err == E_CANCELED {
            canceled();
        }
        if s.stop != Stop::No {
            return stopped_page(t0);
        }
        if let Some(e) = s.resp.error {
            if started && s.page.n > 0 {
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
        if !s.page.cut && (s.resp.cut || err == E_TOOBIG) {
            s.page.note(b"[Dira: the page was cut here; M1 reads at most 4 MiB]", html::R_DIM);
        } else if !s.page.cut && !s.resp.complete() {
            s.page.note(b"[Dira: the page did not arrive in full]", html::R_DIM);
        }
        let mut st: Buf<100> = Buf::new();
        st.push(Num::new(s.resp.status as u64).get());
        st.push(b" ");
        st.push(phrase(s.resp.status));
        st.push(b", ");
        st.push(Num::new(s.resp.body_bytes).get());
        st.push(b" bytes in ");
        st.push(Num::new(ms_now().saturating_sub(t0)).get());
        st.push(b" ms, at ");
        let room = 100 - st.len();
        st.push(&hops.get()[..hops.len().min(room)]);
        s.status = st;
        return;
    }
}

// ------------------------------------------------------------------------------------------- the text mode --

fn console_byte(c: u8) -> u8 {
    if (0x20..0x7f).contains(&c) {
        c
    } else {
        b'?'
    }
}

fn print_page() {
    let s = b();
    let mut shown = Shown::new();
    if let Some(l) = &s.loc {
        loc_text(l, &mut shown);
    }
    out(&[b"dira: ", shown.get(), b": ", s.status.get()]);
    if s.page.title.len() > 0 {
        out(&[b"dira: title: ", s.page.title.get()]);
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
        out(&[b"| ", &line[..n]]);
    }
    if rows == ROWS_MAX {
        out(&[b"dira: (only the first ", Num::new(ROWS_MAX as u64).get(), b" rows are shown)"]);
    }
    for k in 1..=s.page.nlinks {
        let href = s.page.link(k).unwrap_or(b"");
        let mut o = Shown::new();
        let what: &[u8] = match &s.loc {
            Some(Loc::Net(base)) => match url::resolve(base, href) {
                Ok(Some(u)) => {
                    url::show(&u, &mut o);
                    o.get()
                }
                Ok(None) => b"(a place in this page)",
                Err(e) => e.as_bytes(),
            },
            _ => href,
        };
        out(&[b"dira: link [", Num::new(k as u64).get(), b"] ", what]);
    }
    out(&[
        b"dira: done: ",
        Num::new(s.page.n as u64).get(),
        b" letters, ",
        Num::new(rows as u64).get(),
        b" rows, ",
        Num::new(s.page.nlinks as u64).get(),
        b" links",
    ]);
}

fn parse_loc(arg: &[u8]) -> Result<Loc, &'static str> {
    let a = arg.trim_ascii();
    if let Some(name) = a.strip_prefix(b"file:") {
        let name = name.trim_ascii();
        let mut n: Buf<NAME_MAX> = Buf::new();
        if name.is_empty() || !n.push(name) {
            return Err("a file name is 1 to 24 letters");
        }
        return Ok(Loc::File(n));
    }
    url::parse(a).map(Loc::Net)
}

/// The test rig's two addresses (suite pc-dira-proc), refused as addresses anywhere else: `dira:crash` runs an invalid
/// instruction (a crash in Dira ends Dira, not the system), `dira:spin` loops with no calls (Close anyway stops it).
/// In the window mode the window opens first.
fn rig_test(arg: &[u8]) {
    let t = arg.trim_ascii();
    if block().mode & MODE_RIG == 0 || (t != b"dira:crash" && t != b"dira:spin") {
        return;
    }
    if block().mode & MODE_TEXT == 0 && block().cell_h != NONE_H {
        sys(SYS_CELL_OPEN, cell(), 84, 30, 0, 0);
        out(&[b"[dira] test: the window is open"]);
    }
    if t == b"dira:crash" {
        out(&[b"[dira] test: an invalid instruction now"]);
        // SAFETY: ud2 raises #UD in ring 3 on purpose; the kernel ends this process.
        unsafe { asm!("ud2", options(noreturn)) };
    }
    out(&[b"[dira] test: spinning with no calls"]);
    loop {
        core::hint::spin_loop();
    }
}

fn run() {
    let blk = block();
    let arg = &blk.arg[..(blk.arg_len as usize).min(512)];
    rig_test(arg);
    let target = match parse_loc(arg) {
        Ok(t) => t,
        Err(e) => return out(&[b"dira: ", e.as_bytes()]),
    };
    if blk.mode & MODE_TEXT != 0 || blk.cell_h == NONE_H {
        b().window = false;
        load(target);
        print_page();
    } else {
        b().window = true;
        window(target);
    }
}

// ----------------------------------------------------------------------------------------------- the window --

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

fn cell() -> u64 {
    block().cell_h as u64
}

fn grid() -> (usize, usize) {
    let g = sys(SYS_CELL_SIZE, cell(), 0, 0, 0, 0);
    if g < 0 {
        return (0, 0);
    }
    let g = g as u32;
    (((g >> 14) & 0x3FFF) as usize, (g & 0x3FFF) as usize)
}

fn cursor(col: usize, row: usize, shape: u64) {
    sys(SYS_CELL_CURSOR, cell(), col as u64, row as u64, shape, 0);
}

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
    fn put(&mut self, t: &[u8], attr: u8) {
        for &ch in t {
            if self.n >= self.cols {
                return;
            }
            self.c[2 * self.n] = if (0x20..0x7f).contains(&ch) || ch == 0x80 { ch } else { 0x80 };
            self.c[2 * self.n + 1] = attr;
            self.n += 1;
        }
    }
    fn show(&self, r: usize) {
        let bytes = &self.c[..2 * self.cols];
        sys(SYS_CELL_PUT, cell(), 0, r as u64, bytes.as_ptr() as u64, bytes.len() as u64);
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
        cursor(at.min(w.cols.saturating_sub(1)), 0, 1);
        return;
    }
    let mut shown = Shown::new();
    if let Some(l) = &s.loc {
        loc_text(l, &mut shown);
    }
    r.put(b" ", A_BAR);
    r.put(shown.get(), A_BAR); // the address Dira built and checked; nothing from the page reaches row 0
    if matches!(s.loc, Some(Loc::Net(_))) {
        let note = b"plain HTTP, not private ";
        if r.n + note.len() + 2 <= w.cols {
            r.n = w.cols - note.len();
            r.put(note, A_BAR_DIM);
        }
    }
    r.show(0);
    cursor(0, 0, 0);
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
            r.n = 1;
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

fn go(w: &mut Win, to: Loc, remember: bool) -> bool {
    let s = b();
    if remember {
        if let Some(cur) = s.loc {
            if s.nhist == HIST {
                s.hist.copy_within(1.., 0);
                s.nhist -= 1;
            }
            s.hist[s.nhist] = Some(cur);
            s.nhist += 1;
        }
    }
    w.mode = Mode::Browse;
    s.loc = Some(to);
    s.page.reset();
    s.nrows = 0;
    w.top = 0;
    let mut shown = Shown::new();
    loc_text(&to, &mut shown);
    set_status(&[b"Loading ", shown.get()]);
    draw_all(w);
    load(to);
    rewrap(w);
    let mut shown = Shown::new();
    if let Some(l) = &s.loc {
        loc_text(l, &mut shown);
    }
    out(&[
        b"[dira] ",
        shown.get(),
        b": ",
        s.status.get(),
        b"; ",
        Num::new(s.page.n as u64).get(),
        b" letters, ",
        Num::new(s.nrows as u64).get(),
        b" rows, ",
        Num::new(s.page.nlinks as u64).get(),
        b" links",
    ]);
    if matches!(s.stop, Stop::Close | Stop::Quit) {
        return false;
    }
    draw_all(w);
    true
}

fn follow(w: &mut Win, k: usize) -> bool {
    let s = b();
    let Some(href) = s.page.link(k) else {
        set_status(&[b"there is no link with that number"]);
        draw_foot(w);
        return true;
    };
    let mut h: Buf<{ html::LINK_MAX }> = Buf::new();
    h.push(href);
    let target = match &s.loc {
        Some(Loc::Net(base)) => url::resolve(base, h.get()),
        _ => url::parse(h.get()).map(Some).map_err(|_| "a link in a file page needs a full http:// address"),
    };
    match target {
        Ok(Some(u)) => {
            out(&[b"[dira] follow link [", Num::new(k as u64).get(), b"]"]);
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

fn key(w: &mut Win, k: u32) -> bool {
    let s = b();
    match (w.esc, k) {
        (1, 0x5B) => {
            w.esc = 2;
            return true;
        }
        (2, 0x41) => {
            w.esc = 0;
            return scroll(w, -1);
        }
        (2, 0x42) => {
            w.esc = 0;
            return scroll(w, 1);
        }
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
                match parse_loc(t.get()) {
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
                let n = s.typed.get().iter().fold(0usize, |a, &d| a.saturating_mul(10).saturating_add((d - b'0') as usize));
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
                0x20 | 0x66 => return scroll(w, page),
                0x62 => return scroll(w, -page),
                0x6A => return scroll(w, 1),
                0x6B => return scroll(w, -1),
                0x47 => return scroll(w, isize::MAX / 2),
                0x74 => return scroll(w, -(isize::MAX / 2)),
                0x71 => {
                    s.stop = Stop::Quit;
                    return false;
                }
                0x67 => {
                    w.mode = Mode::Address;
                    s.typed.clear();
                    draw_bar(w);
                    draw_foot(w);
                }
                0x72 => {
                    if let Some(l) = s.loc {
                        return go(w, l, false);
                    }
                }
                0x68 | 8 | 127 => {
                    if s.nhist == 0 {
                        set_status(&[b"nothing to go back to"]);
                        draw_foot(w);
                    } else {
                        s.nhist -= 1;
                        if let Some(l) = s.hist[s.nhist].take() {
                            out(&[b"[dira] back"]);
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

fn window(target: Loc) {
    let r = sys(SYS_CELL_OPEN, cell(), 84, 30, 0, 0);
    if r < 0 {
        let why: &[u8] = if r == E_WOULDBLOCK { b"another app has the cell window open" } else { b"the desk refused the window" };
        return out(&[b"dira: cannot open a window: ", why]);
    }
    let (cols, rows) = grid();
    let mut w = Win { cols: cols.min(COLS_MAX), rows, top: 0, mode: Mode::Browse, esc: 0 };
    out(&[b"[dira] window ", Num::new(w.cols as u64).get(), b"x", Num::new(w.rows as u64).get()]);
    let mut open = go(&mut w, target, false);
    while open {
        let ev = sys(SYS_CELL_EVENT, cell(), u64::MAX, 0, 0, 0); // no limit
        if ev < 0 {
            b().stop = Stop::Close;
            break;
        }
        let ev = ev as u32;
        let (kind, payload) = ((ev >> 28) & 7, ev & PAYLOAD);
        match kind {
            EV_KEY => open = key(&mut w, payload),
            EV_RESIZE => {
                let (c, r) = (((payload >> 14) & 0x3FFF) as usize, (payload & 0x3FFF) as usize);
                w.cols = c.min(COLS_MAX);
                w.rows = r;
                rewrap(&mut w);
                draw_all(&w);
            }
            EV_CLOSE_REQUEST | EV_CLOSED => {
                b().stop = Stop::Close;
                open = false;
            }
            _ => {}
        }
    }
    out(&[b"[dira] closed (", if b().stop == Stop::Close { &b"the close box"[..] } else { &b"q"[..] }, b")"]);
}
