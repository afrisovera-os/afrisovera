//! `report <ip>[:port]`: the PC tells the owner's main PC how it is doing, over the home
//! network (docs/design/field-report.md).
//!
//! The owner boots AfriSovera on a spare PC whose screen nobody else can see. Typing
//! `report 192.168.1.20` sends ONE plain-text report to `tools/report-receiver.py` on the
//! main PC (port 7700 unless one is given): commit and build, firmware and screen, cores and
//! memory, the Asili device list, every ls* probe, the boot log, and any fault lines.
//!
//! One-way and owner-driven: nothing is sent unless the owner types the command, the
//! connection goes only to the address typed, nothing it receives is read as data or as a
//! command, and nothing is written to any disk.
//!
//! The console log is kept from power-on in two pieces: the FIRST 16 KiB (the boot lines)
//! and a ring of the LATEST 16 KiB, both fed by the PC's print path (uart.rs).

use core::fmt::Write;
use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, Ordering};

pub const PORT: u16 = 7700;
const HEAD_CAP: usize = 16 * 1024;
const TAIL_CAP: usize = 16 * 1024;
const BODY_CAP: usize = 96 * 1024;

static mut HEAD: [u8; HEAD_CAP] = [0; HEAD_CAP];
static mut HEAD_LEN: usize = 0;
static mut TAIL: [u8; TAIL_CAP] = [0; TAIL_CAP];
static mut TAIL_TOTAL: usize = 0; // bytes ever written to the ring
static mut BODY: [u8; BODY_CAP] = [0; BODY_CAP];
static mut BODY_LEN: usize = 0;
static mut BODY_CUT: bool = false;
/// While set, console output goes into the report body (a probe's answer), not the log.
static CAPTURE: AtomicBool = AtomicBool::new(false);

/// Every console print, from uart.rs (under its print lock, interrupts off): keep it.
pub fn feed(s: &str) {
    // SAFETY: called only under uart.rs's PRINT_LOCK, so one writer at a time.
    unsafe {
        if CAPTURE.load(Ordering::Relaxed) {
            body_put(s.as_bytes());
            return;
        }
        let mut b = s.as_bytes();
        let room = HEAD_CAP - HEAD_LEN;
        if room > 0 {
            let n = room.min(b.len());
            let head = addr_of_mut!(HEAD) as *mut u8;
            core::ptr::copy_nonoverlapping(b.as_ptr(), head.add(HEAD_LEN), n);
            HEAD_LEN += n;
            b = &b[n..];
        }
        let tail = addr_of_mut!(TAIL) as *mut u8;
        for &c in b {
            *tail.add(TAIL_TOTAL % TAIL_CAP) = c;
            TAIL_TOTAL += 1;
        }
    }
}

unsafe fn body_put(b: &[u8]) {
    let room = BODY_CAP - BODY_LEN;
    let n = room.min(b.len());
    let body = addr_of_mut!(BODY) as *mut u8;
    core::ptr::copy_nonoverlapping(b.as_ptr(), body.add(BODY_LEN), n);
    BODY_LEN += n;
    if n < b.len() {
        BODY_CUT = true;
    }
}

/// `write!` into the body (only between captures, so never under the print lock).
struct Body;
impl Write for Body {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // SAFETY: kernel context, outside any capture: the only writer.
        unsafe { body_put(s.as_bytes()) };
        Ok(())
    }
}

/// The commit this kernel was built from, stamped by build.rs ("unknown" without git).
fn commit() -> &'static str {
    option_env!("AFRI_COMMIT").unwrap_or("unknown")
}

fn commit_date() -> &'static str {
    option_env!("AFRI_COMMIT_DATE").unwrap_or("unknown")
}

fn profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// `a.b.c.d` or `a.b.c.d:port`.
fn parse(arg: &str) -> Option<([u8; 4], u16)> {
    let (ip, port) = match arg.find(':') {
        Some(i) => (&arg[..i], arg[i + 1..].parse::<u16>().ok().filter(|&p| p != 0)?),
        None => (arg, PORT),
    };
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in ip.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 {
            return None;
        }
        out[n] = part.parse::<u8>().ok()?;
        n += 1;
    }
    if n != 4 || out == [0; 4] || out == [255; 4] || out[0] == 127 || out[0] >= 224 {
        return None;
    }
    Some((out, port))
}

/// The probes, in the order the report lists them. Each only reads.
const PROBES: &[&str] = &[
    "date", "fwinfo", "screeninfo", "cpuinfo", "acpiinfo", "mem", "uptime", "ifconfig",
    "devices", "lsvirtio", "lsusb", "lsps2", "lsnvme", "lsahci", "lside", "lse1000", "lsrtl", "lssd",
    "lshda", "wifi",
];

/// Lines of the console log that look like trouble.
const TROUBLE: &[&str] = &[
    "PANIC", "EXCEPTION", "panicked", "FAIL", "WRONG", "NOT STOPPED", "stuck", "timed out",
    "did not", "could not", "no reply",
];

/// Copy the log (`HEAD` then the `TAIL` ring in order) through `f`, a line at a time.
unsafe fn log_lines(mut f: impl FnMut(&[u8])) {
    let head = &*addr_of!(HEAD);
    for l in head[..HEAD_LEN].split(|&c| c == b'\n') {
        f(l);
    }
    let tail = &*addr_of!(TAIL);
    let total = TAIL_TOTAL;
    let start = total.saturating_sub(TAIL_CAP);
    let mut line = [0u8; 256];
    let mut n = 0;
    for i in start..total {
        let c = tail[i % TAIL_CAP];
        if c == b'\n' || n == line.len() {
            f(&line[..n]);
            n = 0;
            if c == b'\n' {
                continue;
            }
        }
        line[n] = c;
        n += 1;
    }
    if n > 0 {
        f(&line[..n]);
    }
}

fn contains(hay: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    hay.len() >= n.len() && hay.windows(n.len()).any(|w| w == n)
}

/// Build the whole report into `BODY`. Returns its length.
fn build(ip: [u8; 4], port: u16) -> usize {
    // SAFETY: kernel context; the report statics are touched only here and in `feed`.
    unsafe {
        BODY_LEN = 0;
        BODY_CUT = false;
        let mac = crate::virtio_core::net_mac();
        let me = crate::virtio_core::dhcp_ip();
        let _ = write!(
            Body,
            "AFRISOVERA FIELD REPORT 1\n\
             commit: {} ({})\n\
             build: {} x86_64 UEFI (mansa-pc)\n\
             from: {}.{}.{}.{} mac {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} ({})\n\
             to: {}.{}.{}.{}:{}\n\
             cores: {} (ACPI)   memory: {} MiB\n",
            commit(), commit_date(), profile(),
            me[0], me[1], me[2], me[3], mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
            crate::nic::short(), ip[0], ip[1], ip[2], ip[3], port,
            crate::acpi::cores(), crate::frame::total_frames() * 4 / 1024
        );
        for p in PROBES {
            let _ = write!(Body, "\n== {} ==\n", p);
            CAPTURE.store(true, Ordering::Relaxed);
            crate::shell::exec(p);
            CAPTURE.store(false, Ordering::Relaxed);
        }
        let _ = write!(Body, "\n== trouble lines in the console log ==\n");
        let mut hits = 0;
        log_lines(|l| {
            if hits < 60 && TROUBLE.iter().any(|t| contains(l, t)) {
                body_put(l);
                body_put(b"\n");
                hits += 1;
            }
        });
        if hits == 0 {
            let _ = write!(Body, "(none)\n");
        }
        let _ = write!(
            Body,
            "(a kernel panic stops the PC, so it cannot be sent afterwards: photograph that screen)\n"
        );
        let head_len = HEAD_LEN;
        let total = TAIL_TOTAL;
        let _ = write!(Body, "\n== boot log (the first {} bytes since power-on) ==\n", head_len);
        body_put(&(&*addr_of!(HEAD))[..head_len]);
        if total > 0 {
            let shown = total.min(TAIL_CAP);
            let _ = write!(
                Body,
                "\n== latest console (the last {} of {} bytes after the boot log) ==\n",
                shown, total
            );
            let tail = &*addr_of!(TAIL);
            for i in total - shown..total {
                body_put(&[tail[i % TAIL_CAP]]);
            }
        }
        if BODY_CUT {
            BODY_LEN = BODY_CAP - 64;
            let _ = write!(Body, "\n[report cut at {} KiB]\n", BODY_CAP / 1024);
        }
        let n = BODY_LEN;
        let _ = write!(Body, "\n== end of report ({} bytes before this line) ==\n", n);
        BODY_LEN
    }
}

/// The shell command.
pub fn command(arg: &str) {
    let arg = arg.trim();
    let (ip, port) = match parse(arg) {
        Some(d) if !arg.is_empty() => d,
        _ => {
            crate::kprintln!("report: send this PC's details to your main PC over the home network.");
            crate::kprintln!("  1. on the main PC run tools/report-receiver.py; it shows its address");
            crate::kprintln!("  2. here type   report <that address>   e.g.  report 192.168.1.20");
            crate::kprintln!("     (port {} unless you add one: report 192.168.1.20:7701)", PORT);
            crate::kprintln!("  nothing is sent anywhere else, and nothing comes back in");
            return;
        }
    };
    if !crate::virtio_core::net_present() {
        crate::kprintln!("report: no wired network card was found; lse1000, lsrtl and lsusb show what is there");
        return;
    }
    if crate::virtio_core::report_lease().is_none() {
        crate::kprintln!("report: the router gave this PC no address. Is the network cable plugged in?");
        return;
    }
    crate::kprintln!("[report] collecting this PC's details ...");
    let len = build(ip, port);
    // SAFETY: the body is finished; nothing writes it while it is being sent.
    let body = unsafe { &(&*addr_of!(BODY))[..len] };
    crate::kprintln!("[report] {} bytes ready, sending to {}.{}.{}.{}:{}", len, ip[0], ip[1], ip[2], ip[3], port);
    use crate::virtio_core::ReportErr as E;
    match crate::virtio_core::report_send(ip, port, body) {
        Ok(r) => crate::kprintln!(
            "[report] SENT {} bytes to {}.{}.{}.{}:{} in {} segments ({} resent), {} ms{}",
            len, ip[0], ip[1], ip[2], ip[3], port, r.segments, r.retransmits, r.ms,
            if r.closed { "" } else { " (the close was not answered; the report was received)" }
        ),
        Err(e) => {
            crate::kprint!("report: NOT SENT: ");
            match e {
                E::NoCard => crate::kprintln!("no wired network card"),
                E::NoLease => crate::kprintln!("the router gave this PC no address (is the cable plugged in?)"),
                E::NoRoute => crate::kprintln!("that address is on another network and the router gave no gateway"),
                E::NoArp(h) => crate::kprintln!(
                    "nobody at {}.{}.{}.{} answered. Is that the right address, on the same router?",
                    h[0], h[1], h[2], h[3]
                ),
                E::Refused => crate::kprintln!(
                    "{}.{}.{}.{} is there but nothing listens on port {}; start tools/report-receiver.py first",
                    ip[0], ip[1], ip[2], ip[3], port
                ),
                E::NoAnswer => crate::kprintln!(
                    "{}.{}.{}.{} never answered; its firewall may block Python (allow it on private networks)",
                    ip[0], ip[1], ip[2], ip[3]
                ),
                E::Stalled(n) => crate::kprintln!("the receiver stopped answering after {} of {} bytes", n, len),
                E::Reset(n) => crate::kprintln!("the receiver closed the connection after {} of {} bytes", n, len),
            }
        }
    }
}
