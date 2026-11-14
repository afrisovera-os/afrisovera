//! AfriSh — a tiny interactive kernel shell.
//!
//! Reads a line from the console UART, echoes it (so it also appears on the
//! framebuffer console via the `kprintln!` tee), and runs a built-in command.
//! This is the first *interactive* surface of AfriSovera — proof the OS can take
//! input and respond, the seed of a real shell/launcher.
//!
//! It runs at EL1 in the boot context after the Phase 0 demo, with interrupts
//! enabled (so the timer keeps ticking and `uptime` advances).

use crate::{asili, frame, heap, sched, vfs};
include!("shell_arch.rs");
include!("shell_exec_cmds.rs");
const MAX_LINE: usize = 128;

/// Run the shell forever.
pub fn run() -> ! {
    crate::kprintln!();
    crate::kprintln!("AfriSh ready. Type 'help' for commands.");
    let mut buf = [0u8; MAX_LINE];
    loop {
        crate::kprint!("afrisovera> ");
        let line = read_line(&mut buf);
        exec(line);
    }
}

/// The shell waits at its own prompt with nothing typed (afriui.md section 32). Only then may the Circle start an app,
/// and only this prompt's read takes its line: a command's own key wait (`cpu_getc`, the pager, appget's y) never
/// does, so the desk never answers a question the person did not see (the architect, 14 Nov).
static AT_PROMPT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The shell is idle at its prompt, with nothing typed.
pub fn at_prompt() -> bool {
    AT_PROMPT.load(core::sync::atomic::Ordering::Relaxed)
}

/// The prompt's key wait: a key, or the line a Circle tile asked for (only while nothing is typed). A key typed after
/// a tile was clicked wins, and the tile's line is dropped.
fn prompt_getc(len: usize) -> Result<u8, &'static str> {
    use core::sync::atomic::Ordering::Relaxed;
    end_catch();
    AT_PROMPT.store(len == 0, Relaxed);
    loop {
        if let Some(c) = crate::uart::getc() {
            AT_PROMPT.store(false, Relaxed);
            crate::desk::launch_drop();
            return Ok(c);
        }
        crate::desk::idle(); // the desktop's input events and redraws (desk.rs): a tile is clicked here
        if len == 0 {
            if let Some(line) = crate::desk::launch_take() {
                AT_PROMPT.store(false, Relaxed);
                return Err(line);
            }
        }
        crate::uart::note_idle_wfi();
        crate::cpu::wfi();
    }
}

/// Read a line, echoing characters. Handles backspace and CR/LF.
fn read_line(buf: &mut [u8; MAX_LINE]) -> &str {
    let mut len = 0;
    loop {
        let c = match prompt_getc(len) {
            Ok(c) => c,
            Err(line) => {
                // a Circle tile's line, shown as typed so the person sees what ran
                let n = line.len().min(MAX_LINE - 1);
                buf[..n].copy_from_slice(&line.as_bytes()[..n]);
                crate::kprintln!("{}", &line[..n]);
                return core::str::from_utf8(&buf[..n]).unwrap_or("");
            }
        };
        match c {
            b'\r' | b'\n' => {
                crate::kprintln!();
                break;
            }
            0x08 | 0x7f => {
                // backspace / delete
                if len > 0 {
                    len -= 1;
                    crate::kprint!("\u{8} \u{8}"); // erase last glyph on screen
                }
            }
            c if c >= 0x20 && c < 0x7f => {
                if len < MAX_LINE - 1 {
                    buf[len] = c;
                    len += 1;
                    // Echo the character.
                    let s = [c];
                    crate::kprint!("{}", core::str::from_utf8(&s).unwrap_or("?"));
                }
            }
            _ => {}
        }
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

/// Block until a byte arrives. When no key is ready we sleep on `wfi` rather than
/// spin — the console runs at EL1 with IRQs enabled, so the PL011 RX interrupt
/// (2.87) wakes us the instant a byte arrives and the 100 Hz timer IRQ guarantees
/// a wake within 10 ms regardless (so `uptime` keeps advancing and we never sleep
/// forever). The RX ISR drains the hardware FIFO into the ring, so idling on `wfi`
/// no longer risks dropping bursty input.
// ---------------------------------------------------------------------------------------------------------------
// A pager and output to a file (afriui.md section 28): `more <cmd>`, `<cmd> | more`, `<cmd> > name`. The command
// runs as usual; what it prints is caught at uart::_print (both kernels) into CAP, then paged or written.
// ---------------------------------------------------------------------------------------------------------------
const CAP_LEN: usize = 32 * 1024;
static mut CAP: [u8; CAP_LEN] = [0; CAP_LEN];
static mut CAP_N: usize = 0;
static mut CAP_CUT: bool = false;
static CAPTURING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The pager is showing a page: its lines go straight to the screen, past `help`'s own page filter on the PC (which
/// holds a line until it ends, and so held the "More:" prompt back).
static PAGING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn paging() -> bool {
    PAGING.load(core::sync::atomic::Ordering::Relaxed)
}

/// Commands that wait for a key (a prompt, an app's input, "any key to return"): they cannot run inside a catch,
/// since the person would never see the question (the architect, 14 Nov). Add each new one here as it lands; the
/// shell's key wait ends a catch anyway (`end_catch`), so a missed name shows its output instead of hanging.
const ASKS_KEY: [&str; 9] = ["appget", "store", "edit", "web", "dira", "nia", "wm", "rustapp", "niavm"];

/// A catch ended early by a key wait: the command's output already went to the screen, so it is not paged or saved.
static CAP_BROKEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// A command is waiting for a key: end an active catch first, showing what was caught, so the person sees the
/// question (`cpu_getc`, store.rs `getkey`). Nothing when no catch is on.
pub fn end_catch() {
    if !capturing() {
        return;
    }
    CAPTURING.store(false, core::sync::atomic::Ordering::Relaxed);
    CAP_BROKEN.store(true, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: the catch is off; nothing else writes CAP.
    let out = unsafe { &(&*core::ptr::addr_of!(CAP))[..CAP_N] };
    crate::kprint!("{}", core::str::from_utf8(out).unwrap_or(""));
    crate::kprintln!("[more] this command asks for a key, so its output goes to the screen");
}

/// Output is being caught (uart::_print asks, with its print lock held).
pub fn capturing() -> bool {
    CAPTURING.load(core::sync::atomic::Ordering::Relaxed)
}

struct Catch;
impl core::fmt::Write for Catch {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // SAFETY: only uart::_print calls this, with its print lock held.
        unsafe {
            let buf = &mut *core::ptr::addr_of_mut!(CAP);
            for &b in s.as_bytes() {
                if CAP_N < CAP_LEN {
                    buf[CAP_N] = b;
                    CAP_N += 1;
                } else {
                    CAP_CUT = true;
                }
            }
        }
        Ok(())
    }
}

/// Catch one print (uart::_print, print lock held).
pub fn capture(args: core::fmt::Arguments) {
    let _ = core::fmt::Write::write_fmt(&mut Catch, args);
}

/// Run `cmd` with its output caught; returns the bytes caught (valid until the next catch) and whether any were cut.
fn caught(cmd: &str) -> (&'static [u8], bool) {
    // SAFETY: mainline; the catch is off until `CAPTURING` is set below.
    unsafe {
        CAP_N = 0;
        CAP_CUT = false;
    }
    CAP_BROKEN.store(false, core::sync::atomic::Ordering::Relaxed);
    CAPTURING.store(true, core::sync::atomic::Ordering::Relaxed);
    exec(cmd);
    CAPTURING.store(false, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: the catch is off; nothing else writes CAP.
    unsafe { (&(&*core::ptr::addr_of!(CAP))[..CAP_N], CAP_CUT) }
}

/// The three forms; true if `line` was one of them (and has run).
fn paged(line: &str) -> bool {
    let t = line.trim();
    if capturing() {
        return false; // a command inside a caught one runs plainly
    }
    let (cmd, to_file) = if let Some(rest) = t.strip_prefix("more ") {
        (rest.trim(), None)
    } else if let Some(rest) = t.strip_suffix("| more") {
        (rest.trim(), None)
    } else if let Some(i) = t.rfind(" > ") {
        let name = t[i + 3..].trim();
        if name.is_empty() || name.contains(' ') {
            return false;
        }
        (t[..i].trim(), Some(name))
    } else {
        return false;
    };
    if cmd.is_empty() {
        crate::kprintln!("more: more <command>, <command> | more, or <command> > <file>");
        return true;
    }
    if cmd.starts_with("more ") || cmd == "more" || cmd.ends_with("| more") || cmd.contains(" > ") {
        crate::kprintln!("more: one more or > at a time");
        return true;
    }
    let word = cmd.split_whitespace().next().unwrap_or("");
    if ASKS_KEY.contains(&word) {
        crate::kprintln!("more: '{}' asks for a key, so it cannot run inside more or >", word);
        return true;
    }
    if let Some(name) = to_file {
        // never the system's files: the config, a bundled app, anything the integrity manifest or appget owns
        if name == "afrisovera.conf" || crate::nia::BUNDLED.iter().any(|(n, _)| *n == name) || crate::integrity::expected(name).is_some() {
            crate::kprintln!("[shell] {} is the system's file; not replaced", name);
            return true;
        }
        // and never a file that is there already (an explicit overwrite can come later; the safe default first)
        if crate::vfs::read_raw(name).is_some() {
            crate::kprintln!("[shell] {} exists; not replaced (choose another name)", name);
            return true;
        }
    }
    let (out, cut) = caught(cmd);
    if CAP_BROKEN.load(core::sync::atomic::Ordering::Relaxed) {
        crate::kprintln!("[more] nothing {}: the command asked for a key", if to_file.is_some() { "saved" } else { "paged" });
        return true;
    }
    match to_file {
        Some(name) => {
            // a cut file ends with a line that says so (the architect, 14 Nov)
            const CUT_NOTE: &[u8] = b"\n[cut: the output was longer than this file can hold]\n";
            let max = crate::vfs::MAX_BIG_FILE_BYTES;
            let longer = cut || out.len() > max;
            let saved = if longer {
                let n = max - CUT_NOTE.len();
                // SAFETY: mainline; CAP is not written again until the next catch, and n + CUT_NOTE fits in CAP.
                unsafe {
                    let buf = &mut *core::ptr::addr_of_mut!(CAP);
                    buf[n..n + CUT_NOTE.len()].copy_from_slice(CUT_NOTE);
                    &buf[..n + CUT_NOTE.len()]
                }
            } else {
                out
            };
            if crate::vfs::write_full(name, saved) {
                crate::kprintln!("[shell] {} bytes of '{}' saved to {}{}", saved.len(), cmd, name, if longer { " (cut: the output was longer)" } else { "" });
            } else {
                crate::kprintln!("[shell] could not save to {} (no room for another file?)", name);
            }
        }
        None => {
            PAGING.store(true, core::sync::atomic::Ordering::Relaxed);
            page(out, cut);
            PAGING.store(false, core::sync::atomic::Ordering::Relaxed);
        }
    }
    true
}

/// Show caught output a screenful at a time: the Terminal's rows with the desk showing, else the console's.
fn page(out: &[u8], cut: bool) {
    let rows = if crate::desk::shown() { crate::desk::term_rows() } else { 46 }.max(3);
    let per = rows - 1; // the last row holds the prompt
    const PROMPT: &str = "More: Space next page, Enter next line, q quit";
    let mut lines = out.split(|&b| b == b'\n').peekable();
    let mut left = per;
    let mut shown = 0usize;
    while let Some(l) = lines.next() {
        if lines.peek().is_none() && l.is_empty() {
            break; // the newline that ends the output
        }
        crate::kprintln!("{}", core::str::from_utf8(l).unwrap_or("?"));
        shown += 1;
        left -= 1;
        if left == 0 && lines.peek().is_some() {
            crate::kprint!("{}", PROMPT);
            let k = cpu_getc();
            // take the prompt off again (both the console and the Terminal take a backspace)
            for _ in 0..PROMPT.len() {
                crate::kprint!("\x08 \x08");
            }
            match k {
                b'q' | b'Q' | 0x03 | 0x1B => {
                    crate::kprintln!("[more] stopped after {} line(s)", shown);
                    return;
                }
                b'\r' | b'\n' => left = 1,
                _ => left = per,
            }
        }
    }
    if cut {
        crate::kprintln!("[more] (the output was longer than {} KiB; the rest was not kept)", CAP_LEN / 1024);
    }
}

fn cpu_getc() -> u8 {
    end_catch(); // a command waiting for a key inside more or > shows its question instead of hanging
    loop {
        if let Some(c) = crate::uart::getc() {
            return c;
        }
        crate::desk::idle(); // the desktop's input events and redraws (desk.rs)
        crate::uart::note_idle_wfi();
        crate::cpu::wfi();
    }
}

/// Convert Unix epoch seconds to civil UTC (year, month, day, hour, min, sec).
/// Pure integer math via Howard Hinnant's days->civil algorithm -- correct leap
/// years and month lengths, no float, no timezone/DST (UTC only).
fn epoch_to_utc(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let hh = (rem / 3600) as u32;
    let mm = ((rem % 3600) / 60) as u32;
    let ss = (rem % 60) as u32;

    let z = days + 719468; // shift epoch to 0000-03-01
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // day-of-era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d, hh, mm, ss)
}

/// Standard CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320). A fast
/// error-detection checksum -- complements the cryptographic SHA/Ed25519 for the
/// non-adversarial integrity case. `crc32(b"123456789") == 0xCBF43926`.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// HMAC-SHA256 (RFC 2104), keyed message authentication over the kernel's SHA-256.
/// Block size 64 bytes; the message is capped at 512 bytes (the VFS file cap).
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let h = crate::sha256::digest(key);
        k[..32].copy_from_slice(&h);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    // inner = SHA256((k ^ ipad) || msg)
    let mut inner_in = [0u8; 64 + 512];
    for i in 0..64 {
        inner_in[i] = 0x36 ^ k[i];
    }
    let m = msg.len().min(512);
    inner_in[64..64 + m].copy_from_slice(&msg[..m]);
    let inner = crate::sha256::digest(&inner_in[..64 + m]);
    // outer = SHA256((k ^ opad) || inner)
    let mut outer_in = [0u8; 64 + 32];
    for i in 0..64 {
        outer_in[i] = 0x5c ^ k[i];
    }
    outer_in[64..96].copy_from_slice(&inner);
    crate::sha256::digest(&outer_in)
}

/// A dotted quad, or `None`. Strict: four decimal fields, each 0..=255, nothing
/// else — `10.0.2.` , `10.0.2.2.2` and `10.0.2.256` are all rejected, because a
/// destination this parser guesses at is a segment that silently leaves for the
/// wrong host.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0usize;
    for field in s.split('.') {
        if n == 4 || field.is_empty() || field.len() > 3 {
            return None;
        }
        let mut v: u32 = 0;
        for c in field.bytes() {
            if !c.is_ascii_digit() {
                return None;
            }
            v = v * 10 + (c - b'0') as u32;
        }
        if v > 255 {
            return None;
        }
        out[n] = v as u8;
        n += 1;
    }
    if n == 4 { Some(out) } else { None }
}

/// Position 1 of `net tcp send`: either `<port>` or `<host>:<port>`.
///
/// An ADDED grammar, not a new verb, and the two halves are DISJOINT: a string
/// that parses as a `u16` can never contain a `:`, so every input the old form
/// accepted takes the identical path with the gateway as its destination, and
/// every input the new form accepts is one the old form answered with a usage
/// line. That invariance is why 2.767 could extend an argument where 2.762
/// needed a new verb, and green suites `tcp`/`tcp-data` re-running unchanged IS
/// the assertion (design-2.767 §2(b)).
///
/// `<host>` is a dotted quad OR a name. The name arm is step (c)+(d) of this
/// milestone: it is carried out of the parser as the name itself rather than
/// resolved here, because parsing is a pure function of the token and a DNS
/// query is a round trip that can time out — one of those belongs in a `match`
/// arm and the other does not.
enum Dest<'a> {
    /// `dst_ip`, `dst_port`.
    Addr([u8; 4], u16),
    /// A `<host>:<port>` whose host is not a dotted quad: resolve, then dial.
    Name(&'a str, u16),
    Bad,
}

fn parse_dest(tok: &str) -> Dest<'_> {
    // `rfind`, not `find`: the port is always after the LAST colon, which is the
    // rule that still holds the day a bracketed IPv6 literal shows up here.
    match tok.rfind(':') {
        None => match tok.parse::<u16>() {
            Ok(p) if p != 0 => Dest::Addr(crate::virtio::GW_IP, p),
            _ => Dest::Bad,
        },
        Some(i) => {
            let port = match tok[i + 1..].parse::<u16>() {
                Ok(p) if p != 0 => p,
                _ => return Dest::Bad,
            };
            match parse_ipv4(&tok[..i]) {
                Some(ip) => Dest::Addr(ip, port),
                None => Dest::Name(&tok[..i], port),
            }
        }
    }
}

pub(crate) fn exec(line: &str) { if paged(line) { return; } pc_intercept!(line);
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    // Split into command and the rest (argument tail).
    let (cmd, rest) = match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => cmd_help(line, cmd, rest),
        "splash" => {
            // The premium AfriSovera brand splash (gold monument on black).
            crate::framebuffer::brand_splash();
            crate::kprintln!("[splash] AfriSovera brand splash rendered; type 'clear' to return");
        }
        "gui" => {
            // Switch off console mirroring so shell echo won't scribble on the
            // desktop, then composite the Tonga desktop onto the framebuffer.
            crate::framebuffer::set_console(false);
            crate::desk::show(); // the AfriSovera desktop (desk.rs, docs/design/desktop.md)
            crate::kprintln!("[gui] Tonga desktop rendered; type 'clear' to return to console");
        }
        "desk" => {
            if !crate::desk::shown() && !rest.is_empty() && rest != "list" {
                crate::framebuffer::set_console(false);
                crate::desk::show();
            }
            crate::desk::command(rest);
        }
        "wm" => {
            // App-owned surfaces: spawn EL0 apps that each create + draw a window
            // via the compositor. Console suspended so it won't overdraw them.
            crate::framebuffer::set_console(false);
            crate::process::run_gui_demo();
            crate::kprintln!("[wm] 3 EL0 apps drew their own windows; type 'clear' to return");
        }
        "faultdemo" => {
            // Spawn a crashing EL0 process; the kernel isolates + survives it.
            crate::kprintln!("[faultdemo] spawning an EL0 process that will deliberately fault...");
            crate::process::run_fault_demo();
            crate::kprintln!("[faultdemo] the offending process was killed; the OS is still running.");
        }
        "el0-ptr" => {
            // Gap G2 probe: an EL0 program passes kernel addresses to SYS_AI and SYS_SPEAK (scripts/el0-ptr-test.sh).
            crate::kprintln!("[el0-ptr] an EL0 program hands the kernel its own addresses...");
            crate::process::run_el0_ptr_probe();
        }
        "sgi-el0" => {
            // 2.991 falsify-first probe: is an SGI taken at EL0 delivered, or
            // dropped by `rust_irq_from_el0`? Prints SGI-EL1-SEEN (control) and
            // SGI-EL0-SEEN (subject) on this boot. Debug-only; no suite calls it.
            crate::kprintln!("[sgi-el0] probing the EL0 SGI entry point...");
            crate::process::run_sgi_el0_probe();
            crate::kprintln!("[sgi-el0] probe done");
        }
        "syssleep" => {
            // Spawn an EL0 app that blocks via the SYS_SLEEP syscall (~300 ms),
            // proving the kernel can suspend a userspace process for a bounded,
            // measured duration. Returns to the shell when the app exits.
            crate::kprintln!("[syssleep] running EL0 app that sleeps via a syscall...");
            crate::process::run_sleep_demo();
            crate::kprintln!("[syssleep] EL0 sleep syscall returned; app exited");
        }
        "edit" => {
            // Interactive EL0 app: keystrokes are routed to its window. It runs
            // until you press '~', then control returns to the shell.
            crate::framebuffer::set_console(false);
            crate::process::run_edit_app();
            crate::kprintln!("[edit] editor app exited; type 'clear' to return to console");
        }
        "sync" => {
            // Persist the in-RAM VFS to the virtio-blk disk.
            crate::vfs::sync_to_disk();
        }
        "config" if rest == "test" => {
            // The allocation-free render's refusal paths against synthetic buffers
            // (2.736). The live kernel renders ~90 B into 512 and never comes near the
            // bound, so these are otherwise code that has never run.
            crate::config::selftest();
        }
        "config" => {
            // Persistent system configuration (survives reboot via the disk).
            if rest == "save" {
                crate::config::save();
            } else {
                crate::config::show();
            }
        }
        "disk" => {
            // virtio-blk status + a write/read-back self-test.
            if !crate::virtio::blk_present() {
                crate::kprintln!("[disk] no virtio-blk device (boot with -drive file=disk.img,if=none,id=d0 -device virtio-blk-device,drive=d0)");
            } else {
                let cap = crate::virtio::blk_capacity();
                crate::kprintln!("[disk] virtio-blk: {} sectors ({} KiB)", cap, cap * 512 / 1024);
                crate::virtio::blk_selftest();
            }
        }
        "net" => cmd_net(line, cmd, rest),
        "ping" => {
            // Convenience alias for `net ping` (ICMP echo to the gateway).
            if crate::virtio::net_present() {
                crate::virtio::net_ping();
            } else {
                crate::kprintln!("[ping] no virtio-net device");
            }
        }
        "audio" => {
            // Report the virtio-sound device; `audio tone` streams a test tone.
            if !crate::virtio::sound_present() {
                crate::kprintln!("[audio] no virtio-sound device (boot QEMU with -device virtio-sound-device,audiodev=snd0)");
            } else if rest == "tone" {
                crate::kprintln!("[audio] streaming a test tone through virtio-sound...");
                if crate::virtio::sound_play_tone() {
                    crate::kprintln!("[audio] done — captured by the wav audiodev if attached");
                }
            } else {
                crate::kprintln!(
                    "[audio] virtio-sound online @ {:#010x} (tx queue max {}); 'audio tone' to play",
                    crate::virtio::sound_base(),
                    crate::virtio::sound_txq_max()
                );
            }
        }
        "ifconfig" => cmd_ifconfig(line, cmd, rest),
        "board" => {
            // No argument: the active board's peripheral (MMIO) map. With one:
            // inspect any board the kernel has a map for (`board list` to see them).
            // `board check` diffs the compiled-in map against the device tree the
            // kernel parsed — no board is named `check`, so it cannot shadow one.
            // `board resolved` shows what the drivers actually use after a live tree
            // has had its say, with a source per row; `board resolved test` runs the
            // override rules against synthetic trees (the live branch is unreachable
            // on QEMU's -kernel path, which passes no DTB). `board machine` asks the
            // question underneath all three — whether the tree describes THIS board
            // at all — by matching its root compatible against the map's.
            match rest {
                "" => crate::board::describe(),
                "list" => crate::board::list(),
                "check" => crate::board::check(),
                "machine" => crate::board::describe_machine(),
                "machine test" => crate::board::machine_selftest(),
                "resolved" => crate::board::describe_resolved(),
                "resolved test" => crate::board::resolve_selftest(),
                name => crate::board::describe_named(name),
            }
        }
        "frame" => {
            // The physical frame allocator's range, and — the point of 2.731 — the
            // bounds that shrink it. `board resolved` answers "how much memory does
            // this machine have"; this answers "how much of it can the kernel hand
            // out", which after the identity-map and bitmap limits is a different
            // number. `frame test` runs the bounds against synthetic ranges, since
            // this machine boots with one range that both bounds happen to accept.
            match rest {
                "" => frame::describe(),
                "test" => frame::selftest(),
                other => crate::kprintln!("frame: unknown argument '{}' (try: frame, frame test)", other),
            }
        }
        "dtb" => cmd_dtb(line, cmd, rest),
        "verify" => cmd_verify(line, cmd, rest),
        "verify-sig" => cmd_verify_sig(line, cmd, rest),
        "daraja" => {
            // Android bridge: translate an .apkmeta's permissions to capabilities.
            let name = if rest.is_empty() { "sample.apkmeta" } else { rest };
            match vfs::read_raw(name) {
                Some(src) => crate::daraja::inspect(src),
                None => crate::kprintln!("daraja: no such file: {}", name),
            }
        }
        "web" => {
            // Nia Web runtime: load a .webapp (HTML subset) and render it.
            let name = if rest.is_empty() { "sample.webapp" } else { rest };
            match vfs::read_raw(name) {
                Some(src) => {
                    crate::kprintln!("[web] rendering {} via the Nia Web runtime...", name);
                    crate::web::render(src);
                    // wait for a key, then return to the console
                    loop {
                        if crate::uart::getc().is_some() {
                            break;
                        }
                    }
                    crate::framebuffer::console_reset();
                }
                None => crate::kprintln!("web: no such file: {}", name),
            }
        }
        "ask" => cmd_ask(line, cmd, rest),
        "say" => {
            // Asili Voice: synthesize the text to real audio on-device.
            if rest.is_empty() {
                crate::kprintln!("usage: say <text>");
            } else if !crate::virtio::sound_present() {
                crate::kprintln!("[say] no audio device (boot with -device virtio-sound-device,audiodev=snd0)");
            } else {
                crate::kprintln!("[say] speaking: \"{}\"", rest);
                crate::voice::say(rest.as_bytes());
            }
        }
        "ps" => {
            // Read-only snapshot of the kernel process table (state per slot).
            crate::process::dump_table();
        }
        "uuid" => cmd_uuid(line, cmd, rest),
        "factor" => cmd_factor(line, cmd, rest),
        "primes" => cmd_primes(line, cmd, rest),
        "sort" => cmd_sort(line, cmd, rest),
        "sortdesc" => cmd_sortdesc(line, cmd, rest),
        "sortbylen" => cmd_sortbylen(line, cmd, rest),
        "cpuinfo" => cmd_cpuinfo(line, cmd, rest),
        "divmod" => cmd_divmod(line, cmd, rest),
        "nextmultiple" => cmd_nextmultiple(line, cmd, rest),
        "roundto" => cmd_roundto(line, cmd, rest),
        "prevmultiple" => {
            // The largest multiple of m that is <= n (flooring to a multiple of m), computed as
            // (n/m)*m. "prevmultiple 17 5 -> 15", "prevmultiple 20 5 -> 20" (n already a multiple),
            // "prevmultiple 1 10 -> 0", "prevmultiple 100 7 -> 98", "prevmultiple 0 5 -> 0". m = 0
            // is reported, not a crash. The floor mirror of nextmultiple (which rounds UP),
            // completing the nextmultiple/roundto/prevmultiple trio; distinct from divmod.
            let mut it = rest.split_whitespace();
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            let m = it.next().and_then(|s| s.parse::<u64>().ok());
            match (n, m) {
                (Some(_), Some(0)) => crate::kprintln!("prevmultiple: m must be nonzero"),
                (Some(n), Some(m)) => {
                    let down = (n / m) * m;
                    crate::kprintln!("prevmultiple: {} -> {} (multiple of {})", n, down, m);
                }
                _ => crate::kprintln!("usage: prevmultiple <n> <m>"),
            }
        }
        "gcd" => cmd_gcd(line, cmd, rest),
        "gcd3" => cmd_gcd3(line, cmd, rest),
        "lcm3" => cmd_lcm3(line, cmd, rest),
        "iscoprime" => cmd_iscoprime(line, cmd, rest),
        "modinv" => cmd_modinv(line, cmd, rest),
        "crt2" => cmd_crt2(line, cmd, rest),
        "jacobi" => cmd_jacobi(line, cmd, rest),
        "order" => cmd_order(line, cmd, rest),
        "repunit" => {
            // The nth base-10 REPUNIT R(n): the integer written as n ones
            // (R(1)=1, R(2)=11, R(3)=111, ...). Built by repeated x = x*10 + 1;
            // R(0) is the empty repunit = 0. Distinct from isrepdigit, which
            // CLASSIFIES whether a given number is a repdigit -- repunit COMPUTES
            // the nth all-ones number. u128 + checked_mul so a large n reports
            // overflow honestly (R(39) is the largest that fits u128; R(40) over-
            // flows) instead of panicking.
            match rest.trim().parse::<u32>() {
                Ok(n) => {
                    let mut x: u128 = 0;
                    let mut overflow = false;
                    for _ in 0..n {
                        match x.checked_mul(10).and_then(|v| v.checked_add(1)) {
                            Some(v) => x = v,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("repunit({}) overflows u128", n);
                    } else {
                        crate::kprintln!("repunit({}) = {}", n, x);
                    }
                }
                Err(_) => crate::kprintln!("usage: repunit <n>"),
            }
        }
        "thabit" => {
            // The nth Thabit number T(n) = 3 * 2^n - 1 (also called "321 numbers";
            // T(0)=2, T(1)=5, T(2)=11, T(3)=23, ...). Named for the 9th-century
            // mathematician Thabit ibn Qurra, whose rule uses Thabit primes to
            // generate amicable pairs. u128 with a guarded shift + checked_mul so a
            // large n reports overflow honestly instead of panicking (T(126) is the
            // largest that fits u128; T(127) overflows).
            match rest.trim().parse::<u32>() {
                Ok(n) => {
                    // 1u128 << n is only defined for n < 128; guard it explicitly.
                    let pow = if n < 128 { Some(1u128 << n) } else { None };
                    // 3*2^n - 1 >= 2 for all n, so the final -1 never underflows.
                    match pow.and_then(|p| p.checked_mul(3)).map(|v| v - 1) {
                        Some(t) => crate::kprintln!("thabit({}) = {}", n, t),
                        None => crate::kprintln!("thabit({}) overflows u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: thabit <n>"),
            }
        }
        "cullen" => {
            // The nth Cullen number C(n) = n * 2^n + 1 (C(0)=1, C(1)=3, C(2)=9,
            // C(3)=25, C(4)=65, ...). The companion of the Woodall numbers n*2^n-1,
            // and distinct mathematics from Thabit's 3*2^n-1. u128 with a guarded
            // shift + checked_mul/checked_add so a large n reports overflow honestly
            // instead of panicking.
            match rest.trim().parse::<u32>() {
                Ok(n) => {
                    // 1u128 << n is only defined for n < 128; guard it explicitly.
                    let pow = if n < 128 { Some(1u128 << n) } else { None };
                    match pow
                        .and_then(|p| p.checked_mul(n as u128))
                        .and_then(|v| v.checked_add(1))
                    {
                        Some(c) => crate::kprintln!("cullen({}) = {}", n, c),
                        None => crate::kprintln!("cullen({}) overflows u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: cullen <n>"),
            }
        }
        "woodall" => {
            // The nth Woodall number W(n) = n * 2^n - 1 (W(1)=1, W(2)=7, W(3)=23,
            // W(4)=63, W(5)=159, ...; OEIS A003261). The companion of the Cullen numbers
            // n*2^n+1 -- W(n) = cullen(n) - 2 -- and distinct from Thabit's 3*2^n-1.
            // W(0) = 0*2^0 - 1 = -1 is undefined in u128, so the domain starts at n = 1.
            // u128 with a guarded shift + checked_mul so a large n reports overflow
            // honestly instead of panicking (W(121) fits, W(122) overflows).
            match rest.trim().parse::<u32>() {
                Ok(0) => crate::kprintln!("woodall: W(0) is undefined (0*2^0 - 1 = -1)"),
                Ok(n) => {
                    // 1u128 << n is only defined for n < 128; guard it explicitly.
                    let pow = if n < 128 { Some(1u128 << n) } else { None };
                    match pow.and_then(|p| p.checked_mul(n as u128)) {
                        Some(v) => crate::kprintln!("woodall({}) = {}", n, v - 1),
                        None => crate::kprintln!("woodall({}) overflows u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: woodall <n>"),
            }
        }
        "leyland" => cmd_leyland(line, cmd, rest),
        "wagstaff" => {
            // The nth Wagstaff number W(n) = (2^n + 1) / 3, defined for ODD n only (for
            // even n, 2^n + 1 is not divisible by 3): W(1)=1, W(3)=3, W(5)=11, W(7)=43,
            // W(9)=171, W(11)=683. A Mersenne cousin -- distinct from mersenne (2^p-1),
            // thabit (3*2^n-1), cullen/woodall (n*2^n +- 1), and fermat (2^(2^n)+1).
            // Guarded u128 shift: W(127) fits, W(129) overflows.
            match rest.trim().parse::<u32>() {
                Ok(n) if n % 2 == 0 => {
                    crate::kprintln!("wagstaff: W({}) is undefined (n must be odd)", n)
                }
                Ok(n) if n < 128 => {
                    // 2^n + 1 fits u128 for n <= 127; exact /3 because n is odd
                    let v = (1u128 << n) + 1;
                    crate::kprintln!("wagstaff({}) = {}", n, v / 3);
                }
                Ok(n) => crate::kprintln!("wagstaff({}) overflows u128", n),
                Err(_) => crate::kprintln!("usage: wagstaff <n>"),
            }
        }
        "fermat" => {
            // The nth Fermat number F(n) = 2^(2^n) + 1 (F(0)=3, F(1)=5, F(2)=17,
            // F(3)=257, F(4)=65537, F(5)=4294967297, F(6)=18446744073709551617).
            // The exponent 2^n must be < 128 for `1u128 << exp` to be defined, so
            // F(6) (exp 64) is the largest that fits u128 and F(7) (exp 128) reports
            // overflow. Distinct from mersenne (2^p - 1) and pow.
            match rest.trim().parse::<u32>() {
                Ok(n) => {
                    if n <= 6 {
                        let exp = 1u32 << n; // 2^n, in {1,2,4,8,16,32,64}
                        let val = (1u128 << exp) + 1;
                        crate::kprintln!("fermat({}) = {}", n, val);
                    } else {
                        crate::kprintln!(
                            "fermat({}) overflows u128 (F(7) = 2^128 + 1)",
                            n
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: fermat <n>"),
            }
        }
        "magicconstant" => cmd_magicconstant(line, cmd, rest),
        "timerinfo" => {
            // ARM generic timer: frequency (CNTFRQ), live count (CNTPCT), and a
            // measured 10ms busywait to prove the counter monotonically advances.
            let hz = crate::timer::hz();
            let c1 = crate::timer::cycles();
            crate::timer::busywait_ms(10);
            let c2 = crate::timer::cycles();
            let elapsed = c2.wrapping_sub(c1);
            let elapsed_us = if hz > 0 {
                ((elapsed as u128) * 1_000_000 / (hz as u128)) as u64
            } else {
                0
            };
            crate::kprintln!("timerinfo: CNTFRQ = {} Hz", hz);
            crate::kprintln!("timerinfo: CNTPCT = {}", c2);
            crate::kprintln!(
                "timerinfo: 10ms busywait measured {} us ({} ticks)",
                elapsed_us,
                elapsed
            );
        }
        "rev" => cmd_rev(line, cmd, rest),
        "hex" => cmd_hex(line, cmd, rest),
        "unhex" => cmd_unhex(line, cmd, rest),
        "pow" => cmd_pow(line, cmd, rest),
        "ipow" => cmd_ipow(line, cmd, rest),
        "caesar" => cmd_caesar(line, cmd, rest),
        "caesarf" => cmd_caesarf(line, cmd, rest),
        "rot13" => cmd_rot13(line, cmd, rest),
        "rot13f" => cmd_rot13f(line, cmd, rest),
        "rot5" => cmd_rot5(line, cmd, rest),
        "rot18" => cmd_rot18(line, cmd, rest),
        "rot18f" => cmd_rot18f(line, cmd, rest),
        "frombase" => cmd_frombase(line, cmd, rest),
        "base" => {
            // Parse an integer in dec/0x/0b/0o and print all four representations.
            let t = rest.trim();
            if t.is_empty() {
                crate::kprintln!("usage: base <number>  (dec, 0x.., 0b.., 0o..)");
            } else {
                let parsed = if t.len() > 2 && (t.starts_with("0x") || t.starts_with("0X")) {
                    u64::from_str_radix(&t[2..], 16)
                } else if t.len() > 2 && (t.starts_with("0b") || t.starts_with("0B")) {
                    u64::from_str_radix(&t[2..], 2)
                } else if t.len() > 2 && (t.starts_with("0o") || t.starts_with("0O")) {
                    u64::from_str_radix(&t[2..], 8)
                } else {
                    t.parse::<u64>()
                };
                match parsed {
                    Ok(n) => crate::kprintln!(
                        "base: dec {}  hex 0x{:x}  oct 0o{:o}  bin 0b{:b}",
                        n, n, n, n
                    ),
                    Err(_) => crate::kprintln!("base: invalid number"),
                }
            }
        }
        "smpinfo" => {
            // Report the live SMP topology. The secondary's Aff0 was recorded by
            // the secondary core itself, so printing it proves core 1 executed code.
            let mpidr: u64;
            // SAFETY: MPIDR_EL1 is readable at EL1. This runs on the primary core.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr); #[cfg(not(target_arch = "aarch64"))] { mpidr = crate::shell_pc::sysreg("mpidr_el1"); }
            }
            let this_aff0 = (mpidr & 0xff) as u32;
            crate::kprintln!("smpinfo: cores online = {}", crate::cores_online());
            crate::kprintln!("smpinfo: primary core Aff0 = {}", this_aff0);
            match crate::secondary_aff0() {
                Some(a) => crate::kprintln!(
                    "smpinfo: secondary core Aff0 = {} (checked in via PSCI CPU_ON)",
                    a
                ),
                None => crate::kprintln!("smpinfo: secondary core: not online (boot with -smp 2)"),
            }
        }
        "crc16" => {
            // CRC-16/CCITT-FALSE: poly 0x1021, init 0xFFFF, non-reflected, xorout 0.
            if rest.is_empty() {
                crate::kprintln!("usage: crc16 <text>");
            } else {
                let bytes = rest.as_bytes();
                let mut crc: u16 = 0xFFFF;
                for &b in bytes {
                    crc ^= (b as u16) << 8;
                    for _ in 0..8 {
                        if crc & 0x8000 != 0 {
                            crc = (crc << 1) ^ 0x1021;
                        } else {
                            crc <<= 1;
                        }
                    }
                }
                crate::kprintln!("crc16: 0x{:04x} ({} bytes, CCITT-FALSE)", crc, bytes.len());
            }
        }
        "adler32" => {
            // Adler-32 checksum (zlib): a running modular sum, distinct from a CRC.
            if rest.is_empty() {
                crate::kprintln!("usage: adler32 <text>");
            } else {
                const MOD: u32 = 65521;
                let bytes = rest.as_bytes();
                let mut a: u32 = 1;
                let mut b: u32 = 0;
                for &byte in bytes {
                    a = (a + byte as u32) % MOD;
                    b = (b + a) % MOD;
                }
                let sum = (b << 16) | a;
                crate::kprintln!("adler32: 0x{:08x} ({} bytes)", sum, bytes.len());
            }
        }
        "swapcase" => cmd_swapcase(line, cmd, rest),
        "swapcasef" => cmd_swapcasef(line, cmd, rest),
        "shoutf" => cmd_shoutf(line, cmd, rest),
        "shout" => cmd_shout(line, cmd, rest),
        "whisper" => cmd_whisper(line, cmd, rest),
        "whisperf" => cmd_whisperf(line, cmd, rest),
        "isqrt" => {
            // Integer square root (floor) via Newton's method -- exact, no floats.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let x = if n < 2 {
                        n
                    } else {
                        let mut x = n;
                        let mut y = (x + 1) / 2;
                        while y < x {
                            x = y;
                            y = (x + n / x) / 2;
                        }
                        x
                    };
                    // x = floor(sqrt(n)) <= 2^32, so x*x fits in u64.
                    crate::kprintln!("isqrt: isqrt({}) = {} ({}^2 = {})", n, x, x, x * x);
                }
                Err(_) => crate::kprintln!("usage: isqrt <n>"),
            }
        }
        "isqrtrem" => {
            // Integer square root WITH remainder: floor root r = isqrt(n) and the remainder
            // rem = n - r*r (so n = r^2 + rem, 0 <= rem <= 2r). "isqrtrem 10 -> root 3 rem 1"
            // (3^2=9), "isqrtrem 16 -> root 4 rem 0" (perfect square), "isqrtrem 99 -> root 9 rem
            // 18". A number-theory primitive (cf. GMP isqrtrem) distinct from isqrt (floor root
            // only) and icbrt/iroot. Same Newton's-method root as isqrt, exact, no floats.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let r = if n < 2 {
                        n
                    } else {
                        let mut x = n;
                        let mut y = (x + 1) / 2;
                        while y < x {
                            x = y;
                            y = (x + n / x) / 2;
                        }
                        x
                    };
                    let rem = n - r * r; // r*r <= n by construction, so no underflow
                    crate::kprintln!("isqrtrem: isqrtrem({}) = root {} rem {}", n, r, rem);
                }
                Err(_) => crate::kprintln!("usage: isqrtrem <n>"),
            }
        }
        "isqrtceil" => {
            // CEILING integer square root: the smallest k with k*k >= n. It is isqrt(n) when n is
            // a perfect square, otherwise isqrt(n)+1. "isqrtceil 10 -> 4" (4^2=16>=10), "isqrtceil
            // 16 -> 4" (perfect square), "isqrtceil 17 -> 5", "isqrtceil 1 -> 1", "isqrtceil 0 ->
            // 0". The ceil complement of isqrt (floor); distinct from isqrtrem (root + remainder).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // Same exact Newton's-method floor root as isqrt, then round up if not exact.
                    let r = if n < 2 {
                        n
                    } else {
                        let mut x = n;
                        let mut y = (x + 1) / 2;
                        while y < x {
                            x = y;
                            y = (x + n / x) / 2;
                        }
                        x
                    };
                    let ceil = if r * r == n { r } else { r + 1 };
                    crate::kprintln!("isqrtceil: isqrtceil({}) = {}", n, ceil);
                }
                Err(_) => crate::kprintln!("usage: isqrtceil <n>"),
            }
        }
        "mmuinfo" => cmd_mmuinfo(line, cmd, rest),
        "dczinfo" => {
            // Decode DCZID_EL0 (Data Cache Zero ID): the DC ZVA block size + prohibit bit.
            let dczid: u64;
            // SAFETY: DCZID_EL0 is a standard AArch64 register readable at EL0/EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, dczid_el0", out(reg) dczid); #[cfg(not(target_arch = "aarch64"))] { dczid = crate::shell_pc::sysreg("dczid_el0"); }
            }
            let raw = (dczid & 0xffff_ffff) as u32;
            let bs = (raw & 0xf) as u32; // log2(block size / 4-byte words)
            let dzp = raw & (1 << 4) != 0; // DC ZVA prohibited?
            let block_bytes = 4u32 << bs;
            crate::kprintln!("dczinfo: DCZID_EL0 = 0x{:08x}", raw);
            crate::kprintln!("dczinfo: DC ZVA block size = {} bytes (BS={})", block_bytes, bs);
            crate::kprintln!(
                "dczinfo: DC ZVA {}",
                if dzp { "prohibited" } else { "permitted" }
            );
        }
        "cacheinfo" => cmd_cacheinfo(line, cmd, rest),
        "clidr" => cmd_clidr(line, cmd, rest),
        "ordinal" => {
            // Convert a non-negative integer to its English ordinal string.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // 11/12/13 always take "th"; otherwise the last digit decides.
                    let suffix = if (11..=13).contains(&(n % 100)) {
                        "th"
                    } else {
                        match n % 10 {
                            1 => "st",
                            2 => "nd",
                            3 => "rd",
                            _ => "th",
                        }
                    };
                    crate::kprintln!("ordinal: {}{}", n, suffix);
                }
                Err(_) => crate::kprintln!("usage: ordinal <n>"),
            }
        }
        "commaify" => {
            // Insert thousands separators (commas) into a non-negative integer for readable
            // grouping: 1234567 -> 1,234,567; 1000 -> 1,000; 100 -> 100; 0 -> 0. Groups the
            // decimal digits in threes from the RIGHT. A pure number-FORMATTING util, distinct
            // from cardinal (spells the number in English words) and numberwords (indexes words).
            match rest.trim().parse::<u64>() {
                Err(_) => crate::kprintln!("usage: commaify <number>"),
                Ok(n) => {
                    // render digits least-significant-first
                    let mut digs = [0u8; 20]; // u64 max is 20 decimal digits
                    let mut nd = 0usize;
                    let mut v = n;
                    if v == 0 {
                        digs[0] = b'0';
                        nd = 1;
                    } else {
                        while v > 0 {
                            digs[nd] = b'0' + (v % 10) as u8;
                            nd += 1;
                            v /= 10;
                        }
                    }
                    // emit most-significant-first, comma after each group of 3 remaining
                    let mut buf = [0u8; 27]; // 20 digits + up to 6 commas + slack
                    let mut len = 0usize;
                    let mut i = nd;
                    while i > 0 {
                        i -= 1;
                        buf[len] = digs[i];
                        len += 1;
                        if i > 0 && i % 3 == 0 {
                            buf[len] = b',';
                            len += 1;
                        }
                    }
                    crate::kprintln!(
                        "commaify: {}",
                        core::str::from_utf8(&buf[..len]).unwrap_or("?")
                    );
                }
            }
        }
        "spelldigits" => cmd_spelldigits(line, cmd, rest),
        "cardinal" => cmd_cardinal(line, cmd, rest),
        "perfect" => {
            // Classify n by the sum of its PROPER divisors (perfect/deficient/abundant).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    // 1 is a proper divisor of any n>1; for n==1 the proper-divisor sum is 0.
                    let mut s: u64 = if n > 1 { 1 } else { 0 };
                    let mut i = 2u64;
                    while i * i <= n {
                        if n % i == 0 {
                            s += i;
                            let j = n / i;
                            if j != i && j != n {
                                s += j;
                            }
                        }
                        i += 1;
                    }
                    let kind = if s == n {
                        "perfect"
                    } else if s < n {
                        "deficient"
                    } else {
                        "abundant"
                    };
                    crate::kprintln!("perfect: {} -- proper divisor sum {} -- {}", n, s, kind);
                }
                _ => crate::kprintln!("usage: perfect <n>"),
            }
        }
        "isarithmetic" => cmd_isarithmetic(line, cmd, rest),
        "isharmonic" => cmd_isharmonic(line, cmd, rest),
        "squarefree" => {
            // Classify whether n (n>=1) is SQUAREFREE: no prime factor appears
            // more than once, equivalently no perfect square > 1 divides n
            // (1,2,3,5,6,7,10,... yes; 4,8,9,12,18,... no). Checks i*i | n for
            // every 2<=i with i*i<=n (a composite i with i*i|n implies a prime
            // square divides n too, so scanning all i is correct). u128 squaring
            // avoids overflow. Distinct from factor (lists the factorization),
            // divisors, and issquare (tests n itself is a square).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut sf = true;
                    let mut i: u128 = 2;
                    while i * i <= nn {
                        if nn % (i * i) == 0 {
                            sf = false;
                            break;
                        }
                        i += 1;
                    }
                    if sf {
                        crate::kprintln!("squarefree: {} IS squarefree", n);
                    } else {
                        crate::kprintln!("squarefree: {} is NOT squarefree", n);
                    }
                }
                _ => crate::kprintln!("usage: squarefree <n>"),
            }
        }
        "iscarmichael" => cmd_iscarmichael(line, cmd, rest),
        "ispowerful" => {
            // Classify whether n (n>=1) is a POWERFUL number: every prime factor
            // appears with exponent >= 2 (equivalently, for each prime p | n, also
            // p^2 | n). 1, 4, 8, 9, 16, 25, 27, 36, 72, 144 IS; 2, 3, 12 (=2^2*3)
            // NOT. This is the natural OPPOSITE of squarefree (every exponent == 1).
            // Trial-divides, counting each prime's exponent; a leftover prime after
            // the sqrt loop has exponent 1 and disqualifies n. 1 is vacuously
            // powerful (no prime factors). Distinct from squarefree, semiprime, and
            // radical (which lists the distinct prime product).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut powerful = true;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            let mut e = 0u32;
                            while x % d == 0 {
                                x /= d;
                                e += 1;
                            }
                            if e < 2 {
                                powerful = false;
                                break;
                            }
                        }
                        d += 1;
                    }
                    // A prime factor > sqrt(original) remains with exponent 1.
                    if x > 1 {
                        powerful = false;
                    }
                    if powerful {
                        crate::kprintln!(
                            "ispowerful: {} IS powerful (every prime factor has exponent >= 2)",
                            n
                        );
                    } else {
                        crate::kprintln!(
                            "ispowerful: {} is NOT powerful (a prime factor has exponent 1)",
                            n
                        );
                    }
                }
                _ => crate::kprintln!("usage: ispowerful <n>"),
            }
        }
        "isachilles" => cmd_isachilles(line, cmd, rest),
        "semiprime" => {
            // Classify whether n (n>=2) is a SEMIPRIME: a product of exactly two
            // primes, counted WITH multiplicity (4=2*2, 6=2*3, 9=3*3, 10, 14,
            // 15, 21, 25, 49 yes; 30=2*3*5, primes, and 1 no). Trial-divides,
            // summing prime-factor multiplicities, and tests the total == 2.
            // Distinct from squarefree (4 IS semiprime but NOT squarefree),
            // factor (lists the factorization), and isprime.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut cnt = 0u32;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            x /= d;
                            cnt += 1;
                        }
                        d += 1;
                    }
                    if x > 1 {
                        cnt += 1;
                    }
                    if cnt == 2 {
                        crate::kprintln!("semiprime: {} IS semiprime", n);
                    } else {
                        crate::kprintln!("semiprime: {} is NOT semiprime", n);
                    }
                }
                _ => crate::kprintln!("usage: semiprime <n>"),
            }
        }
        "sphenic" => {
            // Classify whether n (n>=2) is a SPHENIC number: a product of
            // exactly THREE DISTINCT primes, each to the first power (30=2*3*5,
            // 42, 66, 70, 78, 105=3*5*7, 110 yes; NOT 8=2^3, 12=2^2*3,
            // 60=2^2*3*5 [3 distinct but 2 repeats], primes, semiprimes).
            // Trial-divides, counting distinct primes and flagging any repeated
            // factor. Extends the squarefree/semiprime family (a sphenic number
            // is squarefree with exactly 3 prime factors).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut distinct = 0u32;
                    let mut repeated = false;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            distinct += 1;
                            x /= d;
                            if x % d == 0 {
                                repeated = true;
                                while x % d == 0 {
                                    x /= d;
                                }
                            }
                        }
                        d += 1;
                    }
                    if x > 1 {
                        distinct += 1;
                    }
                    if distinct == 3 && !repeated {
                        crate::kprintln!("sphenic: {} IS sphenic", n);
                    } else {
                        crate::kprintln!("sphenic: {} is NOT sphenic", n);
                    }
                }
                _ => crate::kprintln!("usage: sphenic <n>"),
            }
        }
        "bigomega" => {
            // Count the prime factors of n WITH multiplicity (Omega(n), "big
            // omega"): 12=2^2*3 -> 3, 30=2*3*5 -> 3, 8=2^3 -> 3, primes -> 1,
            // 1 -> 0 (empty product). Generalizes semiprime (which tests
            // Omega==2). Distinct from factor (lists the factorization),
            // divisors (all divisors), and totient. Trial-division; u128 guard.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut c = 0u32;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            x /= d;
                            c += 1;
                        }
                        d += 1;
                    }
                    if x > 1 {
                        c += 1;
                    }
                    crate::kprintln!("bigomega: {} has {} prime factors (with multiplicity)", n, c);
                }
                _ => crate::kprintln!("usage: bigomega <n>"),
            }
        }
        "liouville" => {
            // Liouville function lambda(n) = (-1)^Omega(n), giving +1 or -1, where
            // Omega(n) counts prime factors WITH multiplicity. lambda(1)=1, lambda(2)=-1,
            // lambda(4)=+1 (2^2, Omega=2 even), lambda(6)=+1, lambda(8)=-1, lambda(12)=-1.
            // The completely-multiplicative sibling of mobius: unlike mobius (which is 0
            // on non-squarefree n), liouville counts WITH multiplicity, so lambda(4)=+1
            // while mu(4)=0. Distinct from bigomega (which prints the COUNT Omega itself).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut c = 0u32;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            x /= d;
                            c += 1;
                        }
                        d += 1;
                    }
                    if x > 1 {
                        c += 1;
                    }
                    let lambda: i32 = if c % 2 == 0 { 1 } else { -1 };
                    crate::kprintln!("liouville: lambda({}) = {}", n, lambda);
                }
                _ => crate::kprintln!("usage: liouville <n>"),
            }
        }
        "omega" => {
            // Count the DISTINCT prime factors of n (little-omega, omega(n)):
            // 12=2^2*3 -> 2, 30=2*3*5 -> 3, 8=2^3 -> 1, 64=2^6 -> 1, primes ->
            // 1, 1 -> 0. The COMPLEMENT of bigomega (which counts WITH
            // multiplicity, so 8 -> 3 there but 1 here). Distinct from factor
            // (lists them) and divisors. Trial-division; u128 guard.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut distinct = 0u32;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            distinct += 1;
                            while x % d == 0 {
                                x /= d;
                            }
                        }
                        d += 1;
                    }
                    if x > 1 {
                        distinct += 1;
                    }
                    crate::kprintln!("omega: {} has {} distinct prime factors", n, distinct);
                }
                _ => crate::kprintln!("usage: omega <n>"),
            }
        }
        "mobius" => {
            // Mobius function mu(n) in {-1, 0, +1}: mu(1)=1; mu(n)=0 when a squared prime
            // divides n (n not squarefree); otherwise mu(n)=(-1)^k for k distinct primes.
            // mu(1)=1, mu(2)=-1, mu(6)=+1, mu(30)=-1, mu(4)=0, mu(12)=0. Combines the
            // squarefree test with omega (distinct-prime count) into a single classic
            // value; distinct from squarefree (yes/no), omega (a count), and radical.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut distinct = 0u32;
                    let mut squarefree = true;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            distinct += 1;
                            let mut e = 0u32;
                            while x % d == 0 {
                                x /= d;
                                e += 1;
                            }
                            if e >= 2 {
                                squarefree = false;
                                break;
                            }
                        }
                        d += 1;
                    }
                    if x > 1 {
                        distinct += 1;
                    }
                    let mu: i32 = if !squarefree {
                        0
                    } else if distinct % 2 == 0 {
                        1
                    } else {
                        -1
                    };
                    crate::kprintln!("mobius: mu({}) = {}", n, mu);
                }
                _ => crate::kprintln!("usage: mobius <n>"),
            }
        }
        "mertens" => {
            // Mertens function M(n) = sum of the Mobius function mu(k) for k = 1..=n.
            // M(1)=1, M(2)=0, M(3)=-1, M(4)=-1, M(5)=-2, M(10)=-1, M(100)=1. The running
            // partial sum of mobius; distinct from mobius (a single mu(n) value). Each
            // mu(k) is computed by trial-division factorization, so this is O(n sqrt n).
            fn mu(n: u64) -> i32 {
                let mut x = n;
                let mut distinct = 0u32;
                let mut d = 2u64;
                while (d as u128) * (d as u128) <= x as u128 {
                    if x % d == 0 {
                        distinct += 1;
                        let mut e = 0u32;
                        while x % d == 0 {
                            x /= d;
                            e += 1;
                        }
                        if e >= 2 {
                            return 0; // a squared prime divides n -> mu = 0
                        }
                    }
                    d += 1;
                }
                if x > 1 {
                    distinct += 1;
                }
                if distinct % 2 == 0 { 1 } else { -1 }
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut m: i64 = 0;
                    let mut k = 1u64;
                    while k <= n {
                        m += mu(k) as i64;
                        k += 1;
                    }
                    crate::kprintln!("mertens: M({}) = {}", n, m);
                }
                _ => crate::kprintln!("usage: mertens <n>"),
            }
        }
        "radical" => {
            // The RADICAL rad(n): the product of the DISTINCT primes dividing n
            // (rad(12)=2*3=6, rad(8)=2, rad(30)=30, rad(360)=2*3*5=30, rad(64)=2,
            // rad(100)=10, rad(1)=1, rad(prime p)=p). Where omega COUNTS the
            // distinct primes, radical MULTIPLIES them. rad(n)<=n so it never
            // overflows u64, but the product is guarded with checked_mul anyway.
            // Distinct from factor (lists them), divisors, and squarefree.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut r = 1u64;
                    let mut d = 2u64;
                    let mut overflow = false;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            match r.checked_mul(d) {
                                Some(v) => r = v,
                                None => {
                                    overflow = true;
                                    break;
                                }
                            }
                            while x % d == 0 {
                                x /= d;
                            }
                        }
                        d += 1;
                    }
                    if !overflow && x > 1 {
                        match r.checked_mul(x) {
                            Some(v) => r = v,
                            None => overflow = true,
                        }
                    }
                    if overflow {
                        crate::kprintln!("radical: rad({}) overflows u64", n);
                    } else {
                        crate::kprintln!("radical: rad({}) = {}", n, r);
                    }
                }
                _ => crate::kprintln!("usage: radical <n>"),
            }
        }
        "maxprimefactor" => {
            // The largest PRIME factor of n (OEIS A006530, the "greatest prime factor" gpf), by
            // trial division. "maxprimefactor 60 -> 5" (60 = 2^2*3*5), "maxprimefactor 17 -> 17"
            // (a prime is its own gpf), "maxprimefactor 100 -> 5", "maxprimefactor 91 -> 13"
            // (7*13), "maxprimefactor 2 -> 2". n <= 1 has no prime factor and is reported.
            // Distinct from factor (lists ALL), radical (PRODUCT of distinct primes), and isprime.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut largest = 1u64;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            largest = d;
                            x /= d;
                        }
                        d += 1;
                    }
                    if x > 1 {
                        largest = x; // remaining x is a prime factor larger than sqrt
                    }
                    crate::kprintln!("maxprimefactor: {} -> {}", n, largest);
                }
                Ok(_) => crate::kprintln!("maxprimefactor: n has no prime factor (n must be >= 2)"),
                Err(_) => crate::kprintln!("usage: maxprimefactor <n>"),
            }
        }
        "minprimefactor" => {
            // The smallest PRIME factor of n (OEIS A020639, the "least prime factor" lpf), by
            // trial division: the first d >= 2 that divides n, or n itself when n is prime.
            // "minprimefactor 15 -> 3", "minprimefactor 100 -> 2", "minprimefactor 91 -> 7"
            // (7*13), "minprimefactor 17 -> 17" (prime -> itself), "minprimefactor 2 -> 2".
            // n <= 1 has no prime factor and is reported. The complement of maxprimefactor.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut smallest = n; // prime fallback: n is its own least prime factor
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= n as u128 {
                        if n % d == 0 {
                            smallest = d;
                            break;
                        }
                        d += 1;
                    }
                    crate::kprintln!("minprimefactor: {} -> {}", n, smallest);
                }
                Ok(_) => crate::kprintln!("minprimefactor: n has no prime factor (n must be >= 2)"),
                Err(_) => crate::kprintln!("usage: minprimefactor <n>"),
            }
        }
        "sumprimefactors" => {
            // Sum of the prime factors of n counted WITH MULTIPLICITY (OEIS A001414, sopfr, the
            // "integer logarithm"). "sumprimefactors 12 -> 7" (2+2+3), "sumprimefactors 60 -> 12"
            // (2+2+3+5), "sumprimefactors 100 -> 14" (2+2+5+5), "sumprimefactors 17 -> 17" (prime
            // -> itself), "sumprimefactors 2 -> 2". n <= 1 is reported. Distinct from radical
            // (PRODUCT of DISTINCT primes), bigomega/omega (COUNTS), and max/minprimefactor.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut sum = 0u64;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            sum += d;
                            x /= d;
                        }
                        d += 1;
                    }
                    if x > 1 {
                        sum += x; // last prime factor above sqrt
                    }
                    crate::kprintln!("sumprimefactors: {} -> {}", n, sum);
                }
                Ok(_) => crate::kprintln!("sumprimefactors: n has no prime factor (n must be >= 2)"),
                Err(_) => crate::kprintln!("usage: sumprimefactors <n>"),
            }
        }
        "sumdistinctprimefactors" => {
            // Sum of the DISTINCT prime factors of n (OEIS A008472, sopf) -- each prime counted
            // ONCE regardless of multiplicity. "sumdistinctprimefactors 12 -> 5" (2+3),
            // "sumdistinctprimefactors 60 -> 10" (2+3+5), "sumdistinctprimefactors 100 -> 7" (2+5),
            // "sumdistinctprimefactors 17 -> 17" (prime), "sumdistinctprimefactors 2 -> 2". n <= 1
            // is reported. The distinct-primes complement of sumprimefactors (WITH multiplicity);
            // distinct from radical (PRODUCT of distinct) and omega (COUNT of distinct).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut sum = 0u64;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        if x % d == 0 {
                            sum += d; // count this prime once
                            while x % d == 0 {
                                x /= d;
                            }
                        }
                        d += 1;
                    }
                    if x > 1 {
                        sum += x; // last distinct prime above sqrt
                    }
                    crate::kprintln!("sumdistinctprimefactors: {} -> {}", n, sum);
                }
                Ok(_) => {
                    crate::kprintln!("sumdistinctprimefactors: n has no prime factor (n must be >= 2)")
                }
                Err(_) => crate::kprintln!("usage: sumdistinctprimefactors <n>"),
            }
        }
        "smith" => {
            // Classify whether n is a SMITH number: a COMPOSITE whose decimal
            // digit sum equals the combined digit sum of its prime factors,
            // taken WITH multiplicity (4=2*2: ds(4)=4 vs 2+2=4 YES; 22=2*11:
            // 2+2=4 vs 2+(1+1)=4 YES; 27=3^3: 9 vs 3+3+3=9 YES; 58,85,94,121,
            // 166,378 yes). Primes are excluded by definition (their single
            // prime factor is themselves, so the sums trivially match). Uses the
            // trial-division factor loop plus a decimal digit-sum helper.
            fn dsum(mut x: u64) -> u64 {
                let mut s = 0u64;
                while x > 0 {
                    s += x % 10;
                    x /= 10;
                }
                s
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 2 => {
                    let mut x = n;
                    let mut factor_count = 0u32;
                    let mut fsum = 0u64;
                    let mut d = 2u64;
                    while (d as u128) * (d as u128) <= x as u128 {
                        while x % d == 0 {
                            x /= d;
                            factor_count += 1;
                            fsum += dsum(d);
                        }
                        d += 1;
                    }
                    if x > 1 {
                        factor_count += 1;
                        fsum += dsum(x);
                    }
                    // A prime has exactly one prime factor (itself); Smith
                    // numbers must be composite (>= 2 prime factors w/ mult).
                    let composite = factor_count >= 2;
                    if composite && dsum(n) == fsum {
                        crate::kprintln!("smith: {} IS a Smith number", n);
                    } else {
                        crate::kprintln!("smith: {} is NOT a Smith number", n);
                    }
                }
                _ => crate::kprintln!("usage: smith <n>"),
            }
        }
        "amicable" => {
            // Classify whether n is part of an AMICABLE pair: two DISTINCT numbers
            // where each is the sum of the OTHER's proper divisors -- s(a) = b and
            // s(b) = a, with a != b. The classical pair is 220/284: s(220) = 284
            // (1+2+4+5+10+11+20+22+44+55+110), s(284) = 220 (1+2+4+71+142). Reuses
            // the proper-divisor-sum trial-division loop from `perfect`. PERFECT
            // numbers (s(n) == n) are excluded by the a != b check, since a perfect
            // number is its own fixed point under s(), not a distinct partner.
            fn proper_divisor_sum(n: u64) -> u64 {
                let mut s: u64 = if n > 1 { 1 } else { 0 };
                let mut i = 2u64;
                while i * i <= n {
                    if n % i == 0 {
                        s += i;
                        let j = n / i;
                        if j != i {
                            s += j;
                        }
                    }
                    i += 1;
                }
                s
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let partner = proper_divisor_sum(n);
                    let back = proper_divisor_sum(partner);
                    if partner != n && back == n {
                        crate::kprintln!(
                            "amicable: {} IS amicable -- paired with {}",
                            n,
                            partner
                        );
                    } else {
                        crate::kprintln!("amicable: {} is NOT amicable", n);
                    }
                }
                _ => crate::kprintln!("usage: amicable <n>"),
            }
        }
        "issemiperfect" => cmd_issemiperfect(line, cmd, rest),
        "isweird" => cmd_isweird(line, cmd, rest),
        "kaprekar" => cmd_kaprekar(line, cmd, rest),
        "isself" => {
            // Classify whether n is a SELF (Colombian) number: one for which there is NO m with
            // m + digitsum(m) = n. Self numbers: 1,3,5,7,9,20,31,42,53,64,75,86,97,108,... Because
            // digitsum(m) <= 9*20 = 180 for any u64 m, only m in [n-200, n) could possibly generate
            // n, so a short scan suffices. isself 20 = YES, 21 = NO (15+6), 31 = YES, 10 = NO (5+5).
            // Distinct from happy/ugly/perfect and the digitsum command (a plain sum).
            fn dsum(mut x: u64) -> u64 {
                let mut s = 0u64;
                while x > 0 {
                    s += x % 10;
                    x /= 10;
                }
                s
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let lo = n.saturating_sub(200);
                    let mut is_self = true;
                    let mut m = lo;
                    while m < n {
                        if m + dsum(m) == n {
                            is_self = false;
                            break;
                        }
                        m += 1;
                    }
                    if is_self {
                        crate::kprintln!(
                            "isself: {} IS a self number (no m with m + digitsum(m) = n)",
                            n
                        );
                    } else {
                        crate::kprintln!("isself: {} is NOT a self number", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: isself <n>"),
            }
        }
        "isugly" => {
            // Classify whether n (n>=1) is an UGLY number (5-smooth): its only
            // prime factors are 2, 3, and/or 5 (1,2,3,4,5,6,8,9,10,12,15,16,30,
            // 60 yes; 7,11,14,21,77 no). Divides out every factor of 2, 3, and 5
            // and checks the remainder is 1 (1 is ugly -- the empty product).
            // Distinct from factor (lists factors), squarefree, and semiprime.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    for p in [2u64, 3, 5] {
                        while x % p == 0 {
                            x /= p;
                        }
                    }
                    if x == 1 {
                        crate::kprintln!("isugly: {} IS an ugly number", n);
                    } else {
                        crate::kprintln!("isugly: {} is NOT an ugly number", n);
                    }
                }
                _ => crate::kprintln!("usage: isugly <n>"),
            }
        }
        "factorial" => {
            // Compute n! in u128 with an explicit overflow guard (35! exceeds u128).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut f: u128 = 1;
                    let mut overflow = false;
                    let mut i: u128 = 2;
                    while i <= n as u128 {
                        match f.checked_mul(i) {
                            Some(x) => f = x,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                        i += 1;
                    }
                    if overflow {
                        crate::kprintln!("factorial: {}! overflows u128", n);
                    } else {
                        crate::kprintln!("factorial: {}! = {}", n, f);
                    }
                }
                Err(_) => crate::kprintln!("usage: factorial <n>"),
            }
        }
        "doublefact" => {
            // The double factorial n!! = n*(n-2)*(n-4)*... down to 2 (even n) or 1
            // (odd n); 0!!=1!!=1. Distinct from factorial: n! multiplies EVERY
            // integer down to 1, whereas n!! multiplies EVERY OTHER integer (same
            // parity as n) -> 1,1,2,3,8,15,48,105,384,945,3840,... Honest u64
            // overflow guard (33!! fits, 34!! overflows).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut result: u64 = 1;
                    let mut k = n;
                    let mut overflow = false;
                    while k >= 2 {
                        match result.checked_mul(k) {
                            Some(v) => result = v,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                        k -= 2;
                    }
                    if overflow {
                        crate::kprintln!("doublefact: {}!! overflows u64", n);
                    } else {
                        crate::kprintln!("doublefact: {}!! = {}", n, result);
                    }
                }
                Err(_) => crate::kprintln!("usage: doublefact <n>"),
            }
        }
        "compositorial" => {
            // Compositorial C(n): the product of all COMPOSITE numbers <= n (composites are the
            // integers > 1 that are NOT prime: 4, 6, 8, 9, 10, ...). C(3)=1 (empty product),
            // C(4)=4, C(6)=24, C(8)=192, C(10)=17280 (OEIS A036691). The composite analogue of
            // primorial (product of PRIMES). u128 with checked_mul so overflow is reported.
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k % 2 == 0 {
                    return k == 2;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut p: u128 = 1;
                    let mut overflow = false;
                    let mut k = 4u64; // smallest composite
                    while k <= n {
                        if !is_prime(k) {
                            match p.checked_mul(k as u128) {
                                Some(v) => p = v,
                                None => {
                                    overflow = true;
                                    break;
                                }
                            }
                        }
                        k += 1;
                    }
                    if overflow {
                        crate::kprintln!("compositorial: C({}) overflowed u128", n);
                    } else {
                        crate::kprintln!("compositorial: C({}) = {}", n, p);
                    }
                }
                Err(_) => crate::kprintln!("usage: compositorial <n>"),
            }
        }
        "primorial" => {
            // The product of the FIRST n primes: 2*3*5*7*11*... primorial(0)=1
            // (empty product), 2, 6, 30, 210, 2310, 30030, ... Distinct from
            // factorial (product of the first n INTEGERS) and from primepi/nthprime
            // (which count primes / return the nth prime value, not a product).
            // Honest u64 overflow guard (primorial(15) fits, primorial(16) overflows).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut result: u64 = 1;
                    let mut found: u64 = 0;
                    let mut cand: u64 = 2;
                    let mut overflow = false;
                    while found < n {
                        // Trial-division primality (candidates are tiny, <= ~53).
                        let mut is_p = true;
                        let mut d = 2u64;
                        while d * d <= cand {
                            if cand % d == 0 {
                                is_p = false;
                                break;
                            }
                            d += 1;
                        }
                        if is_p {
                            match result.checked_mul(cand) {
                                Some(v) => {
                                    result = v;
                                    found += 1;
                                }
                                None => {
                                    overflow = true;
                                    break;
                                }
                            }
                        }
                        cand += 1;
                    }
                    if overflow {
                        crate::kprintln!("primorial: primorial({}) overflows u64", n);
                    } else {
                        crate::kprintln!("primorial: primorial({}) = {}", n, result);
                    }
                }
                Err(_) => crate::kprintln!("usage: primorial <n>"),
            }
        }
        "superfact" => cmd_superfact(line, cmd, rest),
        "hyperfactorial" => cmd_hyperfactorial(line, cmd, rest),
        "alternatingfactorial" => cmd_alternatingfactorial(line, cmd, rest),
        "roman" => cmd_roman(line, cmd, rest),
        "nl" => cmd_nl(line, cmd, rest),
        "lsvirtio" => {
            // Read-only enumeration of the virtio-mmio transport slots.
            crate::virtio::lsvirtio();
        }
        #[cfg(not(feature = "board-raspi"))]
        "fwcfg" => {
            // Read the QEMU fw_cfg control device: signature, id, file count.
            // Layout (ARM virt): DATA @ +0x0 (byte reads auto-advance), 16-bit
            // SELECTOR @ +0x8 (big-endian write), DMA @ +0x10.
            let base = crate::board::FWCFG_BASE;
            // Select a key (big-endian) then read `n` bytes from the data port.
            let read_key = |key: u16, out: &mut [u8]| {
                // SAFETY: fixed, valid fw_cfg MMIO on the virt board.
                unsafe {
                    core::ptr::write_volatile((base + 0x8) as *mut u16, key.to_be());
                    for b in out.iter_mut() {
                        *b = core::ptr::read_volatile(base as *const u8);
                    }
                }
            };
            let mut sig = [0u8; 4];
            read_key(0x0000, &mut sig); // FW_CFG_SIGNATURE -> "QEMU"
            let mut id = [0u8; 4];
            read_key(0x0001, &mut id); // FW_CFG_ID (little-endian feature bitmap)
            let id_val = u32::from_le_bytes(id);
            let mut nb = [0u8; 4];
            read_key(0x0019, &mut nb); // FW_CFG_NB_FILES (big-endian count)
            let file_count = u32::from_be_bytes(nb);
            crate::kprintln!(
                "fwcfg: signature = {}",
                core::str::from_utf8(&sig).unwrap_or("?")
            );
            crate::kprintln!("fwcfg: id = 0x{:08x}", id_val);
            crate::kprintln!("fwcfg: file count = {}", file_count);
        }
        #[cfg(feature = "board-raspi")]
        "fwcfg" => {
            crate::kprintln!("fwcfg: no fw_cfg device on this board");
        }
        "cmp" => cmd_cmp(line, cmd, rest),
        "strings" => cmd_strings(line, cmd, rest),
        "dfr" => cmd_dfr(line, cmd, rest),
        "pfr" => cmd_pfr(line, cmd, rest),
        "mmfr" => cmd_mmfr(line, cmd, rest),
        "isar" => cmd_isar(line, cmd, rest),
        "isar1" => cmd_isar1(line, cmd, rest),
        "mmfr1" => cmd_mmfr1(line, cmd, rest),
        "pmcr" => {
            // Decode PMCR_EL0 (Performance Monitors Control Register): the number of
            // event counters + implementer/IDCODE + cycle-counter width. Distinct
            // from `dfr` (which reports the PMU architecture VERSION).
            let pmcr: u64;
            // SAFETY: PMCR_EL0 is a standard AArch64 PMU register readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, pmcr_el0", out(reg) pmcr); #[cfg(not(target_arch = "aarch64"))] { pmcr = crate::shell_pc::sysreg("pmcr_el0"); }
            }
            let raw = (pmcr & 0xffff_ffff) as u32;
            let imp = (raw >> 24) & 0xff;
            let idcode = (raw >> 16) & 0xff;
            let n = (raw >> 11) & 0x1f; // number of event counters
            let lc = (raw >> 6) & 1; // long (64-bit) cycle counter
            let e = raw & 1; // all-counters enable
            crate::kprintln!("pmcr: PMCR_EL0 = 0x{:08x}", raw);
            crate::kprintln!("pmcr: event counters (N) = {}", n);
            crate::kprintln!("pmcr: implementer = 0x{:02x}  IDCODE = 0x{:02x}", imp, idcode);
            crate::kprintln!(
                "pmcr: cycle counter = {}  all-enable = {}",
                if lc != 0 { "64-bit" } else { "32-bit" },
                if e != 0 { "on" } else { "off" }
            );
        }
        "ccsidr" => {
            // Select L1 data cache (CSSELR_EL1 = 0) and read its geometry from
            // CCSIDR_EL1: sets, ways (associativity), line size -> total size.
            // Complements cacheinfo (CTR line sizes) and clidr (cache topology).
            let ccsidr: u64;
            // SAFETY: CSSELR_EL1/CCSIDR_EL1 are standard AArch64 cache registers at EL1.
            // Writing CSSELR only selects which cache CCSIDR reports; it does not alter
            // cache operation. The isb ensures the selection takes effect before the read.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!(
                    "msr csselr_el1, xzr",
                    "isb",
                    "mrs {v}, ccsidr_el1",
                    v = out(reg) ccsidr,
                ); #[cfg(not(target_arch = "aarch64"))] { ccsidr = crate::shell_pc::sysreg("ccsidr_el1"); }
            }
            let raw = (ccsidr & 0xffff_ffff) as u32;
            let linesize = raw & 0x7; // log2(line words) - 2
            let assoc = ((raw >> 3) & 0x3ff) + 1; // Associativity + 1 = ways
            let sets = ((raw >> 13) & 0x7fff) + 1; // NumSets + 1
            let line_bytes = 1u32 << (linesize + 4); // words*4 = 1<<(LineSize+2+2)
            let size = sets * assoc * line_bytes;
            crate::kprintln!("ccsidr: CCSIDR_EL1 (L1D) = 0x{:08x}", raw);
            crate::kprintln!(
                "ccsidr: sets = {}  ways = {}  line = {} bytes",
                sets, assoc, line_bytes
            );
            crate::kprintln!(
                "ccsidr: L1 data cache size = {} bytes ({} KiB)",
                size,
                size / 1024
            );
        }
        "pangram" => {
            // Report whether the text contains every letter A-Z (case-insensitive).
            if rest.trim().is_empty() {
                crate::kprintln!("usage: pangram <text>");
            } else {
                let mut mask: u32 = 0;
                for &b in rest.as_bytes() {
                    let c = b | 0x20; // lowercase ASCII letters; leaves non-letters alone
                    if (b'a'..=b'z').contains(&c) {
                        mask |= 1 << (c - b'a');
                    }
                }
                if mask == 0x03ff_ffff {
                    crate::kprintln!("pangram: YES (all 26 letters present)");
                } else {
                    let mut buf = [0u8; 26];
                    let mut len = 0usize;
                    for i in 0..26u8 {
                        if mask & (1 << i) == 0 {
                            buf[len] = b'a' + i;
                            len += 1;
                        }
                    }
                    crate::kprintln!(
                        "pangram: NO (missing: {})",
                        core::str::from_utf8(&buf[..len]).unwrap_or("?")
                    );
                }
            }
        }
        "isogram" => {
            // Report whether the text is an ISOGRAM: every alphabetic character is
            // DISTINCT (no letter repeats), case-insensitive, ignoring non-letters.
            // "black" and "subdermatoglyphic" ARE; "hello" (l twice) and "AfriSovera"
            // (r repeats) are NOT. Uses a 26-bit seen-mask (like pangram, but pangram
            // tests all-present rather than all-distinct); distinct from uniqchars
            // (which REMOVES duplicate chars) and anagram.
            if rest.trim().is_empty() {
                crate::kprintln!("usage: isogram <text>");
            } else {
                let mut mask: u32 = 0;
                let mut repeated: u8 = 0; // 0 = none yet; else the first repeated letter
                for &b in rest.as_bytes() {
                    let c = b | 0x20; // fold ASCII letters to lowercase
                    if (b'a'..=b'z').contains(&c) {
                        let bit = 1u32 << (c - b'a');
                        if mask & bit != 0 {
                            if repeated == 0 {
                                repeated = c;
                            }
                        } else {
                            mask |= bit;
                        }
                    }
                }
                if repeated == 0 {
                    crate::kprintln!("isogram: YES (all letters distinct)");
                } else {
                    crate::kprintln!("isogram: NO (letter '{}' repeats)", repeated as char);
                }
            }
        }
        "palindrome" => {
            // Does the text read the same forwards and backwards, ignoring case
            // and non-alphanumeric characters? (Distinct from revnum, which is numeric.)
            fn norm(c: u8) -> Option<u8> {
                let l = c | 0x20; // lowercase ASCII letters
                if (b'a'..=b'z').contains(&l) {
                    Some(l)
                } else if (b'0'..=b'9').contains(&c) {
                    Some(c)
                } else {
                    None
                }
            }
            let t = rest.trim();
            if t.is_empty() {
                crate::kprintln!("usage: palindrome <text>");
            } else {
                let b = t.as_bytes();
                let mut i = 0usize;
                let mut j = b.len();
                let mut pal = true;
                while i < j {
                    // Advance each end to the next alphanumeric byte.
                    while i < j && norm(b[i]).is_none() {
                        i += 1;
                    }
                    while j > i && norm(b[j - 1]).is_none() {
                        j -= 1;
                    }
                    if i < j {
                        if norm(b[i]) != norm(b[j - 1]) {
                            pal = false;
                            break;
                        }
                        i += 1;
                        j -= 1;
                    }
                }
                crate::kprintln!("palindrome: {}", if pal { "YES" } else { "NO" });
            }
        }
        "anagram" => cmd_anagram(line, cmd, rest),
        "interleave" => cmd_interleave(line, cmd, rest),
        "join" => cmd_join(line, cmd, rest),
        "slug" => cmd_slug(line, cmd, rest),
        "snake" => cmd_snake(line, cmd, rest),
        "constcase" => cmd_constcase(line, cmd, rest),
        "kebab" => cmd_kebab(line, cmd, rest),
        "camel" => cmd_camel(line, cmd, rest),
        "uncamel" => cmd_uncamel(line, cmd, rest),
        "pascal" => cmd_pascal(line, cmd, rest),
        "dotcase" => cmd_dotcase(line, cmd, rest),
        "traincase" => cmd_traincase(line, cmd, rest),
        "piglatin" => cmd_piglatin(line, cmd, rest),
        "tac" => cmd_tac(line, cmd, rest),
        "hamming" => cmd_hamming(line, cmd, rest),
        "hammingdist" => {
            // Bit Hamming distance between two integers: the number of bit positions where a and
            // b differ = popcount(a XOR b). "hammingdist 0 255" -> 8, "hammingdist 5 5" -> 0,
            // "hammingdist 1 2" -> 2, "hammingdist 255 256" -> 9. Distinct from hamming (which
            // compares two equal-length STRINGS position by position) and popcount (set bits of
            // ONE number). A coding-theory primitive.
            let mut it = rest.split_whitespace();
            let a = it.next().and_then(|s| s.parse::<u64>().ok());
            let b = it.next().and_then(|s| s.parse::<u64>().ok());
            match (a, b) {
                (Some(a), Some(b)) => {
                    let d = (a ^ b).count_ones();
                    crate::kprintln!("hammingdist: {} vs {} = {} bits", a, b, d);
                }
                _ => crate::kprintln!("usage: hammingdist <a> <b>"),
            }
        }
        "bitops" => cmd_bitops(line, cmd, rest),
        "rle" => cmd_rle(line, cmd, rest),
        "unrle" => cmd_unrle(line, cmd, rest),
        "vigenere" => cmd_vigenere(line, cmd, rest),
        "unvigenere" => cmd_unvigenere(line, cmd, rest),
        "replace" => cmd_replace(line, cmd, rest),
        "replacefirst" => cmd_replacefirst(line, cmd, rest),
        "replaceword" => cmd_replaceword(line, cmd, rest),
        "removeword" => cmd_removeword(line, cmd, rest),
        "replacelast" => cmd_replacelast(line, cmd, rest),
        "startswith" => {
            // Report whether <text> begins with <prefix> (yes/no). "startswith un unhappy" ->
            // yes, "startswith un happy" -> no. A boolean predicate, distinct from stripprefix
            // (which REMOVES the prefix if present) and commonprefix (which finds the SHARED
            // prefix of two strings). The prefix is the first token, the haystack the remainder.
            let (prefix, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if prefix.is_empty() || text.is_empty() {
                crate::kprintln!("usage: startswith <prefix> <text>");
            } else {
                crate::kprintln!(
                    "startswith: {}",
                    if text.starts_with(prefix) { "yes" } else { "no" }
                );
            }
        }
        "endswith" => {
            // Report whether <text> ends with <suffix> (yes/no) -- the mirror of startswith.
            // "endswith ing running" -> yes, "endswith ing happy" -> no, "endswith .txt about.txt"
            // -> yes. A boolean predicate, distinct from stripsuffix (which REMOVES the suffix)
            // and commonsuffix (which finds the SHARED suffix of two strings).
            let (suffix, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if suffix.is_empty() || text.is_empty() {
                crate::kprintln!("usage: endswith <suffix> <text>");
            } else {
                crate::kprintln!(
                    "endswith: {}",
                    if text.ends_with(suffix) { "yes" } else { "no" }
                );
            }
        }
        "commonprefix" => cmd_commonprefix(line, cmd, rest),
        "commonsuffix" => cmd_commonsuffix(line, cmd, rest),
        "stripprefix" => {
            // Remove a leading <prefix> from <text> if (and only if) the text starts with it;
            // otherwise the text is printed unchanged. "stripprefix un unhappy" -> "happy",
            // "stripprefix un happy" -> "happy" (no change). The prefix is the first token and
            // the haystack is the remainder. A one-shot anchored trim, distinct from replace/tr
            // (which act anywhere/per-char) and strippunct/despace (which remove char classes).
            let (prefix, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if prefix.is_empty() || text.is_empty() {
                crate::kprintln!("usage: stripprefix <prefix> <text>");
            } else {
                let out = if text.starts_with(prefix) {
                    &text[prefix.len()..]
                } else {
                    text
                };
                crate::kprintln!("stripprefix: {}", out);
            }
        }
        "stripsuffix" => {
            // Remove a trailing <suffix> from <text> if (and only if) the text ends with it;
            // otherwise the text is printed unchanged -- the trailing-end mirror of stripprefix
            // (== Python str.removesuffix). "stripsuffix ing running" -> "runn", "stripsuffix
            // .txt about.txt" -> "about", "stripsuffix ing happy" -> "happy" (no change). The
            // suffix is the first token and the haystack is the remainder; the match is anchored
            // at the END, distinct from replace/tr (anywhere/per-char) and strippunct/despace.
            let (suffix, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if suffix.is_empty() || text.is_empty() {
                crate::kprintln!("usage: stripsuffix <suffix> <text>");
            } else {
                let out = if text.ends_with(suffix) {
                    &text[..text.len() - suffix.len()]
                } else {
                    text
                };
                crate::kprintln!("stripsuffix: {}", out);
            }
        }
        "tr" => cmd_tr(line, cmd, rest),
        "unique" => cmd_unique(line, cmd, rest),
        "rot47f" => cmd_rot47f(line, cmd, rest),
        "rot47" => cmd_rot47(line, cmd, rest),
        "bacon" => cmd_bacon(line, cmd, rest),
        "a1z26" => cmd_a1z26(line, cmd, rest),
        "una1z26" => cmd_una1z26(line, cmd, rest),
        "polybius" => cmd_polybius(line, cmd, rest),
        "unpolybius" => cmd_unpolybius(line, cmd, rest),
        "debacon" => cmd_debacon(line, cmd, rest),
        "atbash" => cmd_atbash(line, cmd, rest),
        "atbashf" => cmd_atbashf(line, cmd, rest),
        "chars" => {
            // Character-class breakdown of the inline text (letters/digits/spaces/other).
            if rest.is_empty() {
                crate::kprintln!("usage: chars <text>");
            } else {
                let bytes = rest.as_bytes();
                let total = bytes.len();
                let letters = bytes.iter().filter(|b| b.is_ascii_alphabetic()).count();
                let digits = bytes.iter().filter(|b| b.is_ascii_digit()).count();
                let spaces = bytes.iter().filter(|&&b| b == b' ').count();
                let other = total - letters - digits - spaces;
                crate::kprintln!(
                    "chars: total={} letters={} digits={} spaces={} other={}",
                    total, letters, digits, spaces, other
                );
            }
        }
        "vowels" => {
            // Count vowels vs consonants in inline text (ASCII letters only; y is
            // counted as a consonant). Complements chars (which does not split letters).
            if rest.trim().is_empty() {
                crate::kprintln!("usage: vowels <text>");
            } else {
                let mut v = 0u32;
                let mut c = 0u32;
                for &x in rest.as_bytes() {
                    let l = x | 0x20;
                    if (b'a'..=b'z').contains(&l) {
                        if matches!(l, b'a' | b'e' | b'i' | b'o' | b'u') {
                            v += 1;
                        } else {
                            c += 1;
                        }
                    }
                }
                crate::kprintln!("vowels: {} consonants: {} (letters: {})", v, c, v + c);
            }
        }
        "syllables" => {
            // Naive syllable estimate: count the maximal RUNS of ASCII vowels
            // (a,e,i,o,u, case-insensitive) -- consecutive vowels count as ONE syllable,
            // so "beautiful" -> eau|i|u = 3. Distinct from vowels (which counts each
            // individual vowel) and vowelcount (a FILE tally). A rough heuristic: a
            // trailing silent 'e' and the letter 'y' are not specially handled.
            if rest.trim().is_empty() {
                crate::kprintln!("usage: syllables <text>");
            } else {
                let text = rest.trim();
                let mut count = 0u32;
                let mut in_vowel = false;
                for &x in text.as_bytes() {
                    let l = x | 0x20;
                    let is_v = (b'a'..=b'z').contains(&l)
                        && matches!(l, b'a' | b'e' | b'i' | b'o' | b'u');
                    if is_v && !in_vowel {
                        count += 1;
                    }
                    in_vowel = is_v;
                }
                crate::kprintln!("syllables: {} = {} syllable(s)", text, count);
            }
        }
        "countsentences" => {
            // Count sentences in the inline text: a sentence is a maximal run of
            // characters containing at least one non-space, terminated by one or more
            // of . ! ? (a run of terminators ends a single sentence). Trailing text with
            // no terminator still counts. "Hi there. How are you? Good!" -> 3, "Wait...
            // really?!" -> 2, "No punctuation" -> 1, "..." -> 0. Equivalent to counting
            // the non-empty segments of a split on [.!?]. Distinct from sentencecase/
            // sentencecap (which TRANSFORM) and wordsperline (a per-LINE word tally).
            if rest.trim().is_empty() {
                crate::kprintln!("usage: countsentences <text>");
            } else {
                let mut count = 0u32;
                let mut has_content = false;
                for &b in rest.as_bytes() {
                    if matches!(b, b'.' | b'!' | b'?') {
                        if has_content {
                            count += 1;
                            has_content = false;
                        }
                    } else if !b.is_ascii_whitespace() {
                        has_content = true;
                    }
                }
                if has_content {
                    count += 1; // trailing sentence with no terminator
                }
                crate::kprintln!("countsentences: {} sentence(s)", count);
            }
        }
        "countsentencesf" => {
            // Count sentences in a VFS FILE -- the file analogue of countsentences (inline),
            // just as vowelcount is to vowels and casecount is to the case counters. Same
            // rule: a sentence is a maximal run with at least one non-space char, terminated
            // by one or more of . ! ? (a run of terminators ends one sentence); trailing
            // unterminated text still counts. motd -> 1 ("...OS."), about.txt -> 2.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: countsentencesf <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("countsentencesf: no such file: {}", name),
                    Some(s) => {
                        let mut count = 0u32;
                        let mut has_content = false;
                        for b in s.bytes() {
                            if matches!(b, b'.' | b'!' | b'?') {
                                if has_content {
                                    count += 1;
                                    has_content = false;
                                }
                            } else if !b.is_ascii_whitespace() {
                                has_content = true;
                            }
                        }
                        if has_content {
                            count += 1;
                        }
                        crate::kprintln!("countsentencesf: {} = {} sentence(s)", name, count);
                    }
                }
            }
        }
        "nthsentence" => cmd_nthsentence(line, cmd, rest),
        "lastsentence" => {
            // Print the LAST sentence of INLINE text -- the end-anchored counterpart of
            // nthsentence (arbitrary index, caller must know it) and the sentence-level
            // analogue of lastword (last WORD)/lastchars (last N chars). Same segmentation
            // rule as nthsentence/countsentences: a sentence is a maximal run of characters
            // containing at least one non-space char, terminated by one or more of . ! ? (a
            // run of terminators ends one sentence); trailing unterminated text still counts
            // as a final sentence. The terminator itself is not included, surrounding
            // whitespace is trimmed. "lastsentence Hi there. How are you? Good!" -> "Good",
            // "lastsentence No punctuation" -> "No punctuation" (single trailing sentence).
            // Distinct from nthsentence (arbitrary INDEX)/countsentences (COUNT only, no
            // extraction)/lastword (WORD not sentence granularity).
            let text = rest;
            let bytes = text.as_bytes();
            let mut seg_start = 0usize;
            let mut has_content = false;
            let mut last: Option<&str> = None;
            for (i, &b) in bytes.iter().enumerate() {
                if matches!(b, b'.' | b'!' | b'?') {
                    if has_content {
                        last = Some(text[seg_start..i].trim());
                        has_content = false;
                    }
                    seg_start = i + 1;
                } else if !b.is_ascii_whitespace() {
                    has_content = true;
                }
            }
            if has_content {
                last = Some(text[seg_start..].trim());
            }
            match last {
                Some(s) => crate::kprintln!("lastsentence: {}", s),
                None => crate::kprintln!("lastsentence: text has no sentences"),
            }
        }
        "firstsentence" => {
            // Print the FIRST sentence of INLINE text -- the start-anchored counterpart of
            // lastsentence (end-anchored) and the sentence-level analogue of firstword (first
            // WORD)/firstchars (first N chars). Same segmentation rule as lastsentence/
            // nthsentence/countsentences: a sentence is a maximal run of characters containing
            // at least one non-space char, terminated by one or more of . ! ? (a run of
            // terminators ends one sentence); trailing unterminated text still counts as a
            // final sentence. The terminator itself is not included, surrounding whitespace is
            // trimmed. "firstsentence Hi there. How are you? Good!" -> "Hi there",
            // "firstsentence No punctuation" -> "No punctuation" (single trailing sentence, no
            // terminator). Distinct from lastsentence (end-anchored)/nthsentence (arbitrary
            // INDEX)/firstword (WORD not sentence granularity).
            let text = rest;
            let bytes = text.as_bytes();
            let mut seg_start = 0usize;
            let mut has_content = false;
            let mut first: Option<&str> = None;
            for (i, &b) in bytes.iter().enumerate() {
                if matches!(b, b'.' | b'!' | b'?') {
                    if has_content {
                        first = Some(text[seg_start..i].trim());
                        break;
                    }
                    seg_start = i + 1;
                } else if !b.is_ascii_whitespace() {
                    has_content = true;
                }
            }
            if first.is_none() && has_content {
                first = Some(text[seg_start..].trim());
            }
            match first {
                Some(s) => crate::kprintln!("firstsentence: {}", s),
                None => crate::kprintln!("firstsentence: text has no sentences"),
            }
        }
        "firstsentencef" => cmd_firstsentencef(line, cmd, rest),
        "lastsentencef" => cmd_lastsentencef(line, cmd, rest),
        "nthsentencef" => cmd_nthsentencef(line, cmd, rest),
        "vowelcount" => {
            // Count the ASCII vowels (a,e,i,o,u both cases) and the total letters
            // in a VFS FILE. Distinct from `vowels` (which counts INLINE TEXT typed
            // as an argument), `charcount` (one specific char), and `histogram`
            // (byte frequency): this is a file-level vowel/letter tally.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: vowelcount <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("vowelcount: no such file: {}", name),
                    Some(s) => {
                        let mut letters = 0u32;
                        let mut vowels = 0u32;
                        for b in s.bytes() {
                            if b.is_ascii_alphabetic() {
                                letters += 1;
                                if matches!(b | 0x20, b'a' | b'e' | b'i' | b'o' | b'u') {
                                    vowels += 1;
                                }
                            }
                        }
                        crate::kprintln!(
                            "vowelcount: {} = {} vowels of {} letters",
                            name, vowels, letters
                        );
                    }
                }
            }
        }
        "titlecount" => cmd_titlecount(line, cmd, rest),
        "titlecountf" => cmd_titlecountf(line, cmd, rest),
        "casecount" => {
            // Count the uppercase and lowercase ASCII letters in a VFS FILE (of the total
            // letters). The case companion of vowelcount; distinct from vowelcount (vowels),
            // charcount (one specific char), shout/whisper (which TRANSFORM case), and
            // histogram (byte frequency): this is a file-level upper/lower letter tally.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: casecount <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("casecount: no such file: {}", name),
                    Some(s) => {
                        let mut upper = 0u32;
                        let mut lower = 0u32;
                        for b in s.bytes() {
                            if b.is_ascii_uppercase() {
                                upper += 1;
                            } else if b.is_ascii_lowercase() {
                                lower += 1;
                            }
                        }
                        crate::kprintln!(
                            "casecount: {} = {} uppercase, {} lowercase (of {} letters)",
                            name, upper, lower, upper + lower
                        );
                    }
                }
            }
        }
        "punctcount" => {
            // Count the ASCII PUNCTUATION characters in a VFS file (the 32-symbol set
            // !"#$%&'()*+,-./:;<=>?@[\]^_`{|}~). The char-class companion of vowelcount
            // (which tallies vowels/letters); distinct from charcount (ONE specific
            // char), wc (line/word/byte totals), and histogram (byte frequency).
            // Letters, digits, and whitespace are not counted.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: punctcount <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("punctcount: no such file: {}", name),
                    Some(s) => {
                        let n = s
                            .as_bytes()
                            .iter()
                            .filter(|&&b| b.is_ascii_punctuation())
                            .count();
                        crate::kprintln!(
                            "punctcount: {} has {} punctuation characters",
                            name, n
                        );
                    }
                }
            }
        }
        "spacecount" => {
            // Count the SPACE characters (0x20) in a VFS file. The char-class companion
            // of punctcount (punctuation) and vowelcount (vowels/letters); distinct from
            // charcount (ONE arbitrary char), wc (line/word/byte totals), and despace
            // (which strips spaces). Only the literal space byte is counted -- not tabs
            // or newlines.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: spacecount <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("spacecount: no such file: {}", name),
                    Some(s) => {
                        let n = s.as_bytes().iter().filter(|&&b| b == b' ').count();
                        crate::kprintln!("spacecount: {} has {} space characters", name, n);
                    }
                }
            }
        }
        "mock" => cmd_mock(line, cmd, rest),
        "mockf" => cmd_mockf(line, cmd, rest),
        "vowelcase" => cmd_vowelcase(line, cmd, rest),
        "consonantcase" => cmd_consonantcase(line, cmd, rest),
        "disemvowelf" => cmd_disemvowelf(line, cmd, rest),
        "disemvowel" => cmd_disemvowel(line, cmd, rest),
        "vowelsonlyf" => cmd_vowelsonlyf(line, cmd, rest),
        "vowelsonly" => cmd_vowelsonly(line, cmd, rest),
        "consonantsonly" => cmd_consonantsonly(line, cmd, rest),
        "digitsonly" => cmd_digitsonly(line, cmd, rest),
        "stripdigits" => cmd_stripdigits(line, cmd, rest),
        "lettersonly" => cmd_lettersonly(line, cmd, rest),
        "uppersonly" => cmd_uppersonly(line, cmd, rest),
        "lowersonly" => cmd_lowersonly(line, cmd, rest),
        "alnum" => cmd_alnum(line, cmd, rest),
        "strippunct" => cmd_strippunct(line, cmd, rest),
        "punctonly" => cmd_punctonly(line, cmd, rest),
        "despace" => cmd_despace(line, cmd, rest),
        "trim" => cmd_trim(line, cmd, rest),
        "delete" => cmd_delete(line, cmd, rest),
        "sortchars" => cmd_sortchars(line, cmd, rest),
        "spaceout" => cmd_spaceout(line, cmd, rest),
        "chunk" => cmd_chunk(line, cmd, rest),
        "double" => cmd_double(line, cmd, rest),
        "repeatto" => cmd_repeatto(line, cmd, rest),
        "repeateach" => cmd_repeateach(line, cmd, rest),
        "doublewords" => cmd_doublewords(line, cmd, rest),
        "mirror" => cmd_mirror(line, cmd, rest),
        "repeat" => cmd_repeat(line, cmd, rest),
        "repeatword" => cmd_repeatword(line, cmd, rest),
        "everynth" => cmd_everynth(line, cmd, rest),
        "everynthword" => cmd_everynthword(line, cmd, rest),
        "firstn" => cmd_firstn(line, cmd, rest),
        "lastn" => {
            // Emit the LAST <n> bytes of a run of text, delimited by [ ] so the
            // truncation is visible. If the text is shorter than n the whole text is
            // emitted (n is clamped to the length). The mirror of firstn (which
            // emits the FIRST n) -- same `<n> <text>` parse, but slices the tail
            // bytes[len-take..]. Distinct from chars, cut, wordn, head, and firstn.
            let r = rest.trim_start();
            let parsed = r.find(' ').and_then(|sp| {
                r[..sp].parse::<usize>().ok().filter(|&k| k > 0).map(|k| (k, &r[sp + 1..]))
            });
            match parsed {
                None => crate::kprintln!("usage: lastn <n> <text>"),
                Some((n, text)) => {
                    let bytes = text.as_bytes();
                    let take = if n < bytes.len() { n } else { bytes.len() };
                    let start = bytes.len() - take; // tail slice offset
                    crate::kprintln!(
                        "lastn: [{}]",
                        core::str::from_utf8(&bytes[start..]).unwrap_or("?")
                    );
                }
            }
        }
        "wordrange" => cmd_wordrange(line, cmd, rest),
        "lastwords" => cmd_lastwords(line, cmd, rest),
        "abbreviate" => cmd_abbreviate(line, cmd, rest),
        "truncatewords" => cmd_truncatewords(line, cmd, rest),
        "substr" => cmd_substr(line, cmd, rest),
        "charrange" => cmd_charrange(line, cmd, rest),
        "insert" => cmd_insert(line, cmd, rest),
        "overlay" => cmd_overlay(line, cmd, rest),
        "deletechar" => cmd_deletechar(line, cmd, rest),
        "trunc" => cmd_trunc(line, cmd, rest),
        "middlechar" => cmd_middlechar(line, cmd, rest),
        "middleword" => cmd_middleword(line, cmd, rest),
        "leetf" => cmd_leetf(line, cmd, rest),
        "leet" => cmd_leet(line, cmd, rest),
        "railfence" => cmd_railfence(line, cmd, rest),
        "boxed" => cmd_boxed(line, cmd, rest),
        "underline" => cmd_underline(line, cmd, rest),
        "bits" => cmd_bits(line, cmd, rest),
        "unbits" => cmd_unbits(line, cmd, rest),
        "soundex" => cmd_soundex(line, cmd, rest),
        "capitalize" => cmd_capitalize(line, cmd, rest),
        "decapitalize" => cmd_decapitalize(line, cmd, rest),
        "capitalizef" => cmd_capitalizef(line, cmd, rest),
        "decapitalizef" => cmd_decapitalizef(line, cmd, rest),
        "sentencecasef" => cmd_sentencecasef(line, cmd, rest),
        "capitalizelast" => cmd_capitalizelast(line, cmd, rest),
        "sentencecase" => cmd_sentencecase(line, cmd, rest),
        "sentencecap" => cmd_sentencecap(line, cmd, rest),
        "acronym" => cmd_acronym(line, cmd, rest),
        "dotacronym" => cmd_dotacronym(line, cmd, rest),
        "acronymf" => cmd_acronymf(line, cmd, rest),
        "initials" => cmd_initials(line, cmd, rest),
        "lastletters" => cmd_lastletters(line, cmd, rest),
        "stutter" => cmd_stutter(line, cmd, rest),
        "pluralize" => cmd_pluralize(line, cmd, rest),
        "singularize" => cmd_singularize(line, cmd, rest),
        "firstlast" => cmd_firstlast(line, cmd, rest),
        "numeronym" => cmd_numeronym(line, cmd, rest),
        "mask" => cmd_mask(line, cmd, rest),
        "censor" => cmd_censor(line, cmd, rest),
        "redact" => cmd_redact(line, cmd, rest),
        "wordlengths" => cmd_wordlengths(line, cmd, rest),
        "wordlensf" => cmd_wordlensf(line, cmd, rest),
        "scrabble" => {
            // Sum the English Scrabble tile values of the letters in the text (case-insensitive;
            // non-letters ignored). A/E/I/O/U/L/N/S/T/R=1, D/G=2, B/C/M/P=3, F/H/V/W/Y=4, K=5,
            // J/X=8, Q/Z=10. "scrabble quiz" -> 22, "scrabble hello" -> 8, "scrabble jazz" -> 29.
            // Distinct from wordvalue (positional a=1..z=26) -- this uses the Scrabble tile scores.
            const VAL: [u32; 26] = [
                1, 3, 3, 2, 1, 4, 2, 4, 1, 8, 5, 1, 3, 1, 1, 3, 10, 1, 1, 1, 1, 4, 4, 8, 4, 10,
            ];
            if rest.trim().is_empty() {
                crate::kprintln!("usage: scrabble <text>");
            } else {
                let mut sum = 0u32;
                for &b in rest.as_bytes() {
                    let c = b | 0x20; // fold to lowercase
                    if (b'a'..=b'z').contains(&c) {
                        sum += VAL[(c - b'a') as usize];
                    }
                }
                crate::kprintln!("scrabble: {}", sum);
            }
        }
        "wordvalue" => cmd_wordvalue(line, cmd, rest),
        "wordvowels" => cmd_wordvowels(line, cmd, rest),
        "numberwords" => cmd_numberwords(line, cmd, rest),
        "nato" => cmd_nato(line, cmd, rest),
        "icbrt" => {
            // Integer floor cube root of n + perfect-cube check. Binary search with
            // a u128 cube to avoid overflow (2642245^3 is the largest cube <= u64::MAX).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let target = n as u128;
                    let mut lo: u64 = 0;
                    let mut hi: u64 = 2_642_246;
                    while lo < hi {
                        let mid = (lo + hi + 1) / 2;
                        if (mid as u128).pow(3) <= target {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    let perfect = (lo as u128).pow(3) == target;
                    crate::kprintln!(
                        "icbrt: cbrt({}) = {} ({})",
                        n,
                        lo,
                        if perfect { "perfect cube" } else { "floor" }
                    );
                }
                _ => crate::kprintln!("usage: icbrt <n>"),
            }
        }
        "iroot" => cmd_iroot(line, cmd, rest),
        "isdudeney" => cmd_isdudeney(line, cmd, rest),
        "nextpalindrome" => {
            // The smallest numeric PALINDROME strictly greater than n (a number whose decimal
            // digits read the same forwards and backwards). nextpalindrome(123)=131,
            // nextpalindrome(99)=101, nextpalindrome(9)=11, nextpalindrome(191)=202. Scans
            // upward from n+1. Distinct from palindrome (a STRING membership test) and revnum
            // (which reverses digits + reports whether n itself is a palindrome).
            fn is_pal(x: u64) -> bool {
                let mut digs = [0u8; 20];
                let mut n = 0usize;
                let mut v = x;
                if v == 0 {
                    return true;
                }
                while v > 0 {
                    digs[n] = (v % 10) as u8;
                    n += 1;
                    v /= 10;
                }
                let (mut i, mut j) = (0usize, n - 1);
                while i < j {
                    if digs[i] != digs[j] {
                        return false;
                    }
                    i += 1;
                    j -= 1;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut c = n + 1;
                    while !is_pal(c) {
                        c += 1;
                    }
                    crate::kprintln!("nextpalindrome: next palindrome after {} is {}", n, c);
                }
                Err(_) => crate::kprintln!("usage: nextpalindrome <n>"),
            }
        }
        "revnum" => {
            // Reverse a number's decimal digits + palindrome check (u128, overflow-safe).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut m = n;
                    let mut r: u128 = 0;
                    while m > 0 {
                        r = r * 10 + (m % 10) as u128;
                        m /= 10;
                    }
                    let palindrome = r == n as u128;
                    crate::kprintln!(
                        "revnum: {} reversed = {} ({})",
                        n,
                        r,
                        if palindrome { "palindrome" } else { "not a palindrome" }
                    );
                }
                _ => crate::kprintln!("usage: revnum <n>"),
            }
        }
        "bitrev" => {
            // Reverse the 32 bits of an integer (MSB<->LSB).
            match rest.trim().parse::<u32>() {
                Ok(n) => {
                    let r = n.reverse_bits();
                    crate::kprintln!("bitrev: {} = {} (0x{:08x})", n, r, r);
                }
                _ => crate::kprintln!("usage: bitrev <n>"),
            }
        }
        "rotl" => {
            // Rotate the 64 bits of n LEFT by k positions -- bits shifted off the top wrap
            // around to the bottom: rotl 1 1 -> 2, rotl 1 63 -> 2^63, rotl 2^63 1 -> 1,
            // rotl 255 8 -> 65280. k is taken mod 64 (so k = 0 or 64 is a no-op). Distinct
            // from bitrev (reverses bit ORDER), a plain shift (drops the off-end bits), and
            // rotate (a TEXT char-position shift). u64, the barrel-rotate used in hashing.
            let mut it = rest.split_whitespace();
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            let k = it.next().and_then(|s| s.parse::<u32>().ok());
            match (n, k) {
                (Some(n), Some(k)) => {
                    let r = n.rotate_left(k);
                    crate::kprintln!("rotl: {} rotl {} = {} (0x{:016x})", n, k, r, r);
                }
                _ => crate::kprintln!("usage: rotl <n> <k>"),
            }
        }
        "rotr" => {
            // Rotate the 64 bits of n RIGHT by k positions -- bits shifted off the bottom
            // wrap around to the top: rotr 2 1 -> 1, rotr 1 1 -> 2^63, rotr 65280 8 -> 255.
            // k is taken mod 64 (so k = 0 or 64 is a no-op). The inverse of rotl (rotl n k
            // then rotr n k returns n); distinct from bitrev (reverses bit ORDER) and a
            // plain shift (drops the off-end bits). u64 barrel rotate as used in hashing.
            let mut it = rest.split_whitespace();
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            let k = it.next().and_then(|s| s.parse::<u32>().ok());
            match (n, k) {
                (Some(n), Some(k)) => {
                    let r = n.rotate_right(k);
                    crate::kprintln!("rotr: {} rotr {} = {} (0x{:016x})", n, k, r, r);
                }
                _ => crate::kprintln!("usage: rotr <n> <k>"),
            }
        }
        "isbinpalindrome" => cmd_isbinpalindrome(line, cmd, rest),
        "nthprime" => {
            // The nth prime number (1-indexed), via trial-division primality testing.
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k == 2 {
                    return true;
                }
                if k % 2 == 0 {
                    return false;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 && n <= 100_000 => {
                    let mut count = 0u64;
                    let mut cand = 1u64;
                    while count < n {
                        cand += 1;
                        if is_prime(cand) {
                            count += 1;
                        }
                    }
                    crate::kprintln!("nthprime: prime #{} = {}", n, cand);
                }
                Ok(n) if n > 100_000 => {
                    crate::kprintln!("nthprime: n too large (max 100000)");
                }
                _ => crate::kprintln!("usage: nthprime <n>"),
            }
        }
        "nextprime" => {
            // The SMALLEST PRIME STRICTLY GREATER THAN n (the successor-prime
            // function). Distinct from nthprime (the nth prime by index), primepi
            // (count of primes <= n), isprime (a primality test), and primes (list
            // primes up to a bound).
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k == 2 {
                    return true;
                }
                if k % 2 == 0 {
                    return false;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => match n.checked_add(1) {
                    None => crate::kprintln!("nextprime: no next prime representable in u64"),
                    Some(mut cand) => {
                        while !is_prime(cand) {
                            match cand.checked_add(1) {
                                Some(v) => cand = v,
                                None => break, // ran off the top of u64 (unreachable for real n)
                            }
                        }
                        crate::kprintln!("nextprime: next prime after {} is {}", n, cand);
                    }
                },
                Err(_) => crate::kprintln!("usage: nextprime <n>"),
            }
        }
        "prevprime" => {
            // The LARGEST PRIME STRICTLY LESS THAN n (the predecessor-prime function, the
            // mirror of nextprime). Since the smallest prime is 2, no prime exists below 3,
            // so n <= 2 reports none. Distinct from nthprime (the nth prime by index),
            // primepi (count of primes <= n), isprime (a primality test), and primes.
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k == 2 {
                    return true;
                }
                if k % 2 == 0 {
                    return false;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    if n <= 2 {
                        crate::kprintln!("prevprime: no prime below {}", n);
                    } else {
                        let mut cand = n - 1;
                        while cand >= 2 && !is_prime(cand) {
                            cand -= 1;
                        }
                        // n > 2 => n-1 >= 2 and 2 is prime, so the scan always finds one
                        crate::kprintln!("prevprime: prev prime before {} is {}", n, cand);
                    }
                }
                Err(_) => crate::kprintln!("usage: prevprime <n>"),
            }
        }
        "nextcomposite" => {
            // The smallest COMPOSITE number strictly greater than n (composites are the integers
            // > 1 that are NOT prime: 4, 6, 8, 9, 10, ...). nextcomposite(3)=4, nextcomposite(4)=6,
            // nextcomposite(8)=9, nextcomposite(1)=4. The composite mirror of nextprime; distinct
            // from nextprime (next PRIME) and compositorial (product of composites).
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k % 2 == 0 {
                    return k == 2;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut c = n + 1;
                    while c < 4 || is_prime(c) {
                        c += 1;
                    }
                    crate::kprintln!("nextcomposite: next composite after {} is {}", n, c);
                }
                Err(_) => crate::kprintln!("usage: nextcomposite <n>"),
            }
        }
        "prevcomposite" => {
            // The largest COMPOSITE number strictly less than n (composites 4, 6, 8, 9, 10, ...).
            // Since the smallest composite is 4, n <= 4 reports none. prevcomposite(10)=9,
            // prevcomposite(5)=4, prevcomposite(13)=12. The composite mirror of prevprime and the
            // downward counterpart of nextcomposite.
            fn is_prime(k: u64) -> bool {
                if k < 2 {
                    return false;
                }
                if k % 2 == 0 {
                    return k == 2;
                }
                let mut d = 3u64;
                while d <= k / d {
                    if k % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    if n <= 4 {
                        crate::kprintln!("prevcomposite: no composite below {}", n);
                    } else {
                        let mut c = n - 1;
                        while c >= 4 && is_prime(c) {
                            c -= 1;
                        }
                        // n > 4 => n-1 >= 4, and 4 is composite, so the scan always finds one
                        crate::kprintln!("prevcomposite: prev composite before {} is {}", n, c);
                    }
                }
                Err(_) => crate::kprintln!("usage: prevcomposite <n>"),
            }
        }
        "primepi" => {
            // The prime-counting function pi(n): how many primes are <= n. Distinct
            // from primes (lists them), nthprime (the nth), and isprime (predicate).
            fn is_prime(x: u64) -> bool {
                if x < 2 {
                    return false;
                }
                if x % 2 == 0 {
                    return x == 2;
                }
                let mut d = 3u64;
                while d * d <= x {
                    if x % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n <= 100_000 => {
                    let mut count = 0u64;
                    let mut k = 2u64;
                    while k <= n {
                        if is_prime(k) {
                            count += 1;
                        }
                        k += 1;
                    }
                    crate::kprintln!("primepi: pi({}) = {}", n, count);
                }
                Ok(_) => crate::kprintln!("primepi: n too large (max 100000)"),
                Err(_) => crate::kprintln!("usage: primepi <n>"),
            }
        }
        "sumprimes" => {
            // The SUM of all primes <= n (2 + 3 + 5 + ... ): sumprimes 10 = 17, sumprimes 2
            // = 2, sumprimes 1 = 0. Distinct from primepi (COUNTS primes <= n), primes (LISTS
            // them), primorial (PRODUCT of the first n primes), and nthprime (the nth). Trial
            // division for shell-scale n (capped at 100000, same as primepi); the sum through
            // 100000 (~454 million) stays well within u64.
            fn is_prime(x: u64) -> bool {
                if x < 2 {
                    return false;
                }
                if x % 2 == 0 {
                    return x == 2;
                }
                let mut d = 3u64;
                while d * d <= x {
                    if x % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n <= 100_000 => {
                    let mut sum = 0u64;
                    let mut k = 2u64;
                    while k <= n {
                        if is_prime(k) {
                            sum += k;
                        }
                        k += 1;
                    }
                    crate::kprintln!("sumprimes: sum of primes <= {} is {}", n, sum);
                }
                Ok(_) => crate::kprintln!("sumprimes: n too large (max 100000)"),
                Err(_) => crate::kprintln!("usage: sumprimes <n>"),
            }
        }
        "catalan" => cmd_catalan(line, cmd, rest),
        "motzkin" => cmd_motzkin(line, cmd, rest),
        "mode" => cmd_mode(line, cmd, rest),
        "median" => cmd_median(line, cmd, rest),
        "luhn" => cmd_luhn(line, cmd, rest),
        "luhngen" => cmd_luhngen(line, cmd, rest),
        "product" => cmd_product(line, cmd, rest),
        "divisors" => {
            // Report a number's divisor count tau(n) and divisor sum sigma(n),
            // via sqrt-time trial division (d <= n/d avoids d*d overflow).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut count = 0u64;
                    let mut sum = 0u64;
                    let mut d = 1u64;
                    while d <= n / d {
                        if n % d == 0 {
                            count += 1;
                            sum += d;
                            let e = n / d;
                            if e != d {
                                count += 1;
                                sum += e;
                            }
                        }
                        d += 1;
                    }
                    crate::kprintln!("divisors: {} has {} divisors, sum {}", n, count, sum);
                }
                _ => crate::kprintln!("usage: divisors <n>"),
            }
        }
        "ispractical" => cmd_ispractical(line, cmd, rest),
        "totient" => {
            // Euler's totient phi(n): count of integers in 1..=n coprime to n.
            // Sqrt-time via the prime-factorization product formula.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut result = n;
                    let mut m = n;
                    let mut p = 2u64;
                    while p * p <= m {
                        if m % p == 0 {
                            while m % p == 0 {
                                m /= p;
                            }
                            result -= result / p;
                        }
                        p += 1;
                    }
                    if m > 1 {
                        result -= result / m;
                    }
                    crate::kprintln!("totient: phi({}) = {}", n, result);
                }
                _ => crate::kprintln!("usage: totient <n>"),
            }
        }
        "lazycaterer" => {
            // The LAZY CATERER's sequence (central polygonal numbers) p(n) = n(n+1)/2 + 1 -- the
            // maximum number of pieces a disk (or pancake) can be cut into with n straight cuts.
            // p(0)=1, p(1)=2, p(2)=4, p(3)=7, p(4)=11, p(5)=16, p(6)=22, p(10)=56. It is one MORE
            // than the nth triangular number; distinct from triangular and the centered-polygonal
            // families. u128 avoids overflow for large n.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let p = nn * (nn + 1) / 2 + 1;
                    crate::kprintln!("lazycaterer: p({}) = {}", n, p);
                }
                Err(_) => crate::kprintln!("usage: lazycaterer <n>"),
            }
        }
        "cake" => {
            // The CAKE numbers C(n) = (n^3 + 5n + 6)/6 -- the 3D analogue of the lazy caterer's
            // sequence: the maximum number of pieces a cake can be cut into with n straight PLANAR
            // cuts. C(0)=1, C(1)=2, C(2)=4, C(3)=8, C(4)=15, C(5)=26, C(6)=42, C(10)=176. The
            // spatial companion of `lazycaterer` (planar cuts); distinct from tetrahedral and the
            // figurate families. u128 avoids overflow (n^3 is large).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let c = (nn * nn * nn + 5 * nn + 6) / 6;
                    crate::kprintln!("cake: C({}) = {}", n, c);
                }
                Err(_) => crate::kprintln!("usage: cake <n>"),
            }
        }
        "schroeder" => cmd_schroeder(line, cmd, rest),
        "delannoy" => cmd_delannoy(line, cmd, rest),
        "stern" => {
            // Stern's diatomic sequence (the "fusc" function) a(n): a(0)=0, a(1)=1,
            // a(2n)=a(n), a(2n+1)=a(n)+a(n+1). First terms (n=0..): 0,1,1,2,1,3,2,3,1,4,3,5,2.
            // Consecutive terms a(n)/a(n+1) enumerate every positive rational exactly once
            // (the Stern-Brocot / Calkin-Wilf ordering). Computed iteratively over the bits of n
            // (no recursion). Distinct from fib/lucas/pell and the other linear recurrences --
            // this recurrence branches on parity. a(9)=4, a(11)=5, a(12)=2.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // fusc(n): a=1,b=0; for each bit LSB->MSB, even -> a+=b, odd -> b+=a; result b.
                    let mut a: u64 = 1;
                    let mut b: u64 = 0;
                    let mut m = n;
                    while m > 0 {
                        if m & 1 == 1 {
                            b += a;
                        } else {
                            a += b;
                        }
                        m >>= 1;
                    }
                    crate::kprintln!("stern: a({}) = {}", n, b);
                }
                Err(_) => crate::kprintln!("usage: stern <n>"),
            }
        }
        "carmichael" => {
            // Carmichael function lambda(n): the least m with a^m = 1 (mod n) for every a
            // coprime to n. lambda(1)=1; for an odd prime power p^k it is phi(p^k)=p^(k-1)(p-1);
            // for 2^k it is 1, 2, then 2^(k-2) (k>=3); lambda(n) = lcm over its prime-power
            // factors. Distinct from totient (Euler phi): lambda(15)=4 but phi(15)=8. Companion
            // of order/modinv -- lambda(n) is the exponent of the multiplicative group mod n.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    fn gcd(mut a: u64, mut b: u64) -> u64 {
                        while b != 0 {
                            let t = b;
                            b = a % b;
                            a = t;
                        }
                        a
                    }
                    fn lcm(a: u64, b: u64) -> u64 {
                        if a == 0 || b == 0 {
                            0
                        } else {
                            a / gcd(a, b) * b
                        }
                    }
                    let mut m = n;
                    let mut result: u64 = 1;
                    let mut p = 2u64;
                    while p <= m / p {
                        if m % p == 0 {
                            let mut k = 0u32;
                            let mut pk = 1u64;
                            while m % p == 0 {
                                m /= p;
                                k += 1;
                                pk *= p;
                            }
                            let lam = if p == 2 {
                                match k {
                                    1 => 1,
                                    2 => 2,
                                    _ => pk / 4, // 2^(k-2)
                                }
                            } else {
                                pk / p * (p - 1) // p^(k-1) * (p-1)
                            };
                            result = lcm(result, lam);
                        }
                        p += 1;
                    }
                    if m > 1 {
                        // a leftover prime factor to the first power
                        let lam = if m == 2 { 1 } else { m - 1 };
                        result = lcm(result, lam);
                    }
                    crate::kprintln!("carmichael: lambda({}) = {}", n, result);
                }
                _ => crate::kprintln!("usage: carmichael <n>"),
            }
        }
        "sumtotient" => {
            // The totient summatory function Phi(n) = phi(1) + phi(2) + ... + phi(n). Phi(1)=1,
            // Phi(5)=10, Phi(10)=32, Phi(100)=3044. It counts the coprime pairs (a,b) with
            // 1<=a<=b<=n roughly, and 2*Phi(n)-1 is the number of Farey fractions of order n.
            // Distinct from totient (a single phi(n)) and cototient (n - phi(n)). n capped at 100000.
            fn phi(mut m: u64) -> u64 {
                let mut result = m;
                let mut p = 2u64;
                while p * p <= m {
                    if m % p == 0 {
                        while m % p == 0 {
                            m /= p;
                        }
                        result -= result / p;
                    }
                    p += 1;
                }
                if m > 1 {
                    result -= result / m;
                }
                result
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n <= 100_000 => {
                    let mut sum = 0u64;
                    let mut k = 1u64;
                    while k <= n {
                        sum += phi(k);
                        k += 1;
                    }
                    crate::kprintln!("sumtotient: Phi({}) = {}", n, sum);
                }
                Ok(_) => crate::kprintln!("sumtotient: n too large (max 100000)"),
                Err(_) => crate::kprintln!("usage: sumtotient <n>"),
            }
        }
        "cototient" => {
            // Euler cototient co-phi(n) = n - phi(n): the count of integers in 1..=n that
            // SHARE a factor with n (i.e. are NOT coprime to n). OEIS A051953. The
            // complement of totient: co-phi(prime) = 1, co-phi(1) = 0, co-phi(p^k) counts
            // the multiples of p. Same sqrt-time prime-factorization product as totient.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut phi = n;
                    let mut m = n;
                    let mut p = 2u64;
                    while p * p <= m {
                        if m % p == 0 {
                            while m % p == 0 {
                                m /= p;
                            }
                            phi -= phi / p;
                        }
                        p += 1;
                    }
                    if m > 1 {
                        phi -= phi / m;
                    }
                    crate::kprintln!("cototient: cophi({}) = {} - {} = {}", n, n, phi, n - phi);
                }
                _ => crate::kprintln!("usage: cototient <n>"),
            }
        }
        "collatz" => {
            // Iterate the Collatz (3n+1) map to 1; report step count and peak value.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let mut x = n;
                    let mut steps = 0u64;
                    let mut peak = n;
                    let mut overflow = false;
                    while x != 1 {
                        if x % 2 == 0 {
                            x /= 2;
                        } else {
                            if x > (u64::MAX - 1) / 3 {
                                overflow = true;
                                break;
                            }
                            x = 3 * x + 1;
                        }
                        if x > peak {
                            peak = x;
                        }
                        steps += 1;
                    }
                    if overflow {
                        crate::kprintln!("collatz: {} overflowed u64 after {} steps", n, steps);
                    } else {
                        crate::kprintln!("collatz: {} -> {} steps, peak {}", n, steps, peak);
                    }
                }
                _ => crate::kprintln!("usage: collatz <n>"),
            }
        }
        "dlog" => cmd_dlog(line, cmd, rest),
        "sqrtmod" => cmd_sqrtmod(line, cmd, rest),
        "isprimitiveroot" => cmd_isprimitiveroot(line, cmd, rest),
        "primitiveroot" => cmd_primitiveroot(line, cmd, rest),
        "pisano" => {
            // Pisano period pi(m): the period with which the Fibonacci sequence taken modulo m
            // repeats. Iterate (a,b)=(0,1) as (a,b)->(b,(a+b) mod m) until the pair returns to
            // (0,1); the number of steps is the period. pi(1)=1, pi(2)=3, pi(3)=8, pi(5)=20,
            // pi(10)=60. A number-theory function distinct from fib (nth term) and order.
            match rest.trim().parse::<u64>() {
                Ok(m) if m >= 1 && m <= 100000 => {
                    if m == 1 {
                        crate::kprintln!("pisano: pi({}) = 1", m);
                    } else {
                        let mut a: u64 = 0;
                        let mut b: u64 = 1;
                        let mut period: u64 = 0;
                        // the period never exceeds 6*m, so this loop is bounded.
                        for i in 1..=(6 * m) {
                            let next = (a + b) % m;
                            a = b;
                            b = next;
                            if a == 0 && b == 1 {
                                period = i;
                                break;
                            }
                        }
                        crate::kprintln!("pisano: pi({}) = {}", m, period);
                    }
                }
                _ => crate::kprintln!("usage: pisano <m>  (1..=100000)"),
            }
        }
        "morse" => cmd_morse(line, cmd, rest),
        "demorse" => cmd_demorse(line, cmd, rest),
        "fnv1a" => {
            // 32-bit FNV-1a hash: offset basis 0x811c9dc5, prime 0x01000193.
            if rest.is_empty() {
                crate::kprintln!("usage: fnv1a <text>");
            } else {
                let bytes = rest.as_bytes();
                let mut h: u32 = 0x811c9dc5;
                for &b in bytes {
                    h ^= b as u32;
                    h = h.wrapping_mul(0x0100_0193);
                }
                crate::kprintln!("fnv1a: 0x{:08x} ({} bytes)", h, bytes.len());
            }
        }
        "gicbase" => {
            // Which addresses gic.rs is really driving, where they came from, and the
            // proof that nothing touched the GIC before the map was resolved.
            // Distinct from gicinfo/giccinfo/gicid, which read the controller's own
            // registers: this one is about the *addresses*, not their contents.
            #[cfg(not(feature = "board-raspi"))]
            {
                crate::gic::describe_bases();
            }
            #[cfg(feature = "board-raspi")]
            {
                crate::kprintln!("gicbase: no GICv2 on this board (local intc @ 0x4000_0000)");
            }
        }
        "gicinfo" => {
            // Read the GICv2 distributor's type register (GICD_TYPER) over MMIO.
            // The base comes from gic.rs (resolved + audited), not from the raw
            // compile-time constant: an inspector that reads a different address
            // than the driver drives is worse than no inspector.
            #[cfg(not(feature = "board-raspi"))]
            {
                let base = crate::gic::dist_base();
                // SAFETY: read-only MMIO of the GICv2 distributor (present on virt).
                let typer = unsafe { core::ptr::read_volatile((base + 0x004) as *const u32) };
                let ctlr = unsafe { core::ptr::read_volatile((base + 0x000) as *const u32) };
                let it_lines = typer & 0x1f;
                let irq_lines = (it_lines + 1) * 32;
                let cpu_number = ((typer >> 5) & 0x7) + 1;
                let enabled = ctlr & 1 != 0;
                crate::kprintln!("gicinfo: GICD_TYPER = 0x{:08x}", typer);
                crate::kprintln!("gicinfo: IRQ lines = {} (ITLinesNumber={})", irq_lines, it_lines);
                crate::kprintln!("gicinfo: CPU interfaces = {}", cpu_number);
                crate::kprintln!(
                    "gicinfo: distributor {}",
                    if enabled { "enabled" } else { "disabled" }
                );
            }
            #[cfg(feature = "board-raspi")]
            {
                crate::kprintln!("gicinfo: GICv2 distributor not modeled on this board");
            }
        }
        "giccinfo" => cmd_giccinfo(line, cmd, rest),
        "gicid" => {
            // Read the GICv2 distributor's Implementer ID register (GICD_IIDR).
            // Distinct from gicinfo (GICD_TYPER topology) and giccinfo (GICC_IIDR).
            // Same base as the driver (resolved + audited), for the same reason as
            // gicinfo: three inspectors of one controller must not read three
            // addresses.
            #[cfg(not(feature = "board-raspi"))]
            {
                let base = crate::gic::dist_base();
                // SAFETY: read-only MMIO of the GICv2 distributor (present on virt).
                let iidr = unsafe { core::ptr::read_volatile((base + 0x008) as *const u32) };
                let implementer = iidr & 0xfff;
                let revision = (iidr >> 12) & 0xf;
                let variant = (iidr >> 16) & 0xf;
                let product = (iidr >> 24) & 0xff;
                let name = if implementer == 0x43b { "ARM" } else { "?" };
                crate::kprintln!("gicid: GICD_IIDR = 0x{:08x}", iidr);
                crate::kprintln!(
                    "gicid: implementer = 0x{:03x} ({})  product = 0x{:02x}",
                    implementer, name, product
                );
                crate::kprintln!("gicid: variant = {}  revision = {}", variant, revision);
            }
            #[cfg(feature = "board-raspi")]
            {
                crate::kprintln!("gicid: no GICv2 distributor on this board");
            }
        }
        "ascii" => cmd_ascii(line, cmd, rest),
        "weekday" => cmd_weekday(line, cmd, rest),
        "cut" => cmd_cut(line, cmd, rest),
        "nthfield" => cmd_nthfield(line, cmd, rest),
        "fieldcount" => {
            // Count the fields produced by splitting <text> on an arbitrary separator <sep>.
            // "fieldcount : a:b:c" -> 3, "fieldcount , x,y,z,w" -> 4, "fieldcount : nosep" -> 1
            // (no separator = one field). A trailing separator yields a trailing empty field
            // ("fieldcount - a-b-" -> 3). The count companion of nthfield (which SELECTS a
            // field); distinct from countword (WHITESPACE words) and countsubstr (occurrences,
            // which equals fieldcount - 1).
            let (sep, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if sep.is_empty() || text.is_empty() {
                crate::kprintln!("usage: fieldcount <sep> <text>");
            } else {
                crate::kprintln!("fieldcount: {}", text.split(sep).count());
            }
        }
        "col" => cmd_col(line, cmd, rest),
        "line" => cmd_line(line, cmd, rest),
        "linefromend" => cmd_linefromend(line, cmd, rest),
        "nthchar" => {
            // The Nth (1-based) CHARACTER (byte) of the inline text, counting EVERY byte --
            // spaces, digits, and punctuation included. "nthchar 1 hello" -> "h", "nthchar 5
            // hello" -> "o", "nthchar 2 a,b" -> "," (punctuation IS counted, unlike nthletter
            // which would give "b"). An out-of-range index reports "(none)". The inline byte-
            // indexer completing the family: charat=Nth BYTE of a FILE, nthletter=Nth LETTER
            // inline (skips non-letters), wordat=Nth WORD inline.
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthchar <n> <text>"),
                Some((n, text)) => {
                    let bytes = text.as_bytes();
                    if n >= 1 && n <= bytes.len() {
                        crate::kprintln!("nthchar: {}", bytes[n - 1] as char);
                    } else {
                        crate::kprintln!("nthchar: (none)");
                    }
                }
            }
        }
        "charfromend" => {
            // The Nth (1-based) CHARACTER (byte) counting from the END of the inline text.
            // "charfromend 1 hello" -> "o", "charfromend 2 hello" -> "l", "charfromend 5 hello" ->
            // "h", "charfromend 6 hello" -> "(none)". The end-anchored mirror of nthchar (Nth char
            // from the START); distinct from lastn (the last n CHARS) and lastchars.
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: charfromend <n> <text>"),
                Some((n, text)) => {
                    let bytes = text.as_bytes();
                    if n >= 1 && n <= bytes.len() {
                        // Nth from end = index (len - n) from the start.
                        crate::kprintln!("charfromend: {}", bytes[bytes.len() - n] as char);
                    } else {
                        crate::kprintln!("charfromend: (none)");
                    }
                }
            }
        }
        "nthletter" => {
            // The Nth (1-based) LETTER of the inline text, skipping all non-letters (spaces,
            // digits, punctuation). "nthletter 3 a b c d" -> "c", "nthletter 5 hello world" ->
            // "o", "nthletter 2 a1b2c3" -> "b". An out-of-range index reports "(none)". Distinct
            // from charat (the Nth BYTE of a FILE) and wordn (the Nth WORD of inline text).
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthletter <n> <text>"),
                Some((n, text)) => {
                    let mut count = 0usize;
                    let mut found: Option<u8> = None;
                    for &b in text.as_bytes() {
                        if b.is_ascii_alphabetic() {
                            count += 1;
                            if count == n {
                                found = Some(b);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(c) => crate::kprintln!("nthletter: {}", c as char),
                        None => crate::kprintln!("nthletter: (none)"),
                    }
                }
            }
        }
        "nthvowel" => {
            // The Nth (1-based) VOWEL (a/e/i/o/u, case-insensitive; y excluded) of the inline
            // text, skipping consonants and everything else. "nthvowel 1 hello -> e", "nthvowel 2
            // hello -> o", "nthvowel 3 hello world -> o", "nthvowel 1 AEIOU -> A"; an out-of-range
            // index reports "(none)". Distinct from nthletter (Nth LETTER) and vowelsonly (ALL
            // vowels); the vowel-filtered analogue of nthletter.
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthvowel <n> <text>"),
                Some((n, text)) => {
                    let mut count = 0usize;
                    let mut found: Option<u8> = None;
                    for &b in text.as_bytes() {
                        if matches!(
                            b,
                            b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'
                        ) {
                            count += 1;
                            if count == n {
                                found = Some(b);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(c) => crate::kprintln!("nthvowel: {}", c as char),
                        None => crate::kprintln!("nthvowel: (none)"),
                    }
                }
            }
        }
        "nthletterfromend" => {
            // The Nth (1-based) LETTER counting from the END of the inline text, skipping all
            // non-letters. "nthletterfromend 1 a b c -> c", "nthletterfromend 2 a b c -> b",
            // "nthletterfromend 1 hello! -> o" (skips the '!'), "nthletterfromend 2 a1b2c3 -> b";
            // an out-of-range index reports "(none)". The end-anchored mirror of nthletter (Nth
            // letter from the START); the letter analogue of charfromend.
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthletterfromend <n> <text>"),
                Some((n, text)) => {
                    // Count letters from the RIGHT: the nth letter from the end.
                    let mut count = 0usize;
                    let mut found: Option<u8> = None;
                    for &b in text.as_bytes().iter().rev() {
                        if b.is_ascii_alphabetic() {
                            count += 1;
                            if count == n {
                                found = Some(b);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(c) => crate::kprintln!("nthletterfromend: {}", c as char),
                        None => crate::kprintln!("nthletterfromend: (none)"),
                    }
                }
            }
        }
        "nthconsonant" => {
            // The Nth (1-based) CONSONANT (an ASCII letter that is NOT a vowel a/e/i/o/u; y counts
            // as a consonant) of the inline text. "nthconsonant 1 hello -> h", "nthconsonant 2
            // hello -> l", "nthconsonant 4 hello world -> w", "nthconsonant 1 sky -> s"; an
            // out-of-range index reports "(none)". The consonant complement of nthvowel; distinct
            // from nthletter (ANY letter) and consonantsonly (ALL consonants).
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthconsonant <n> <text>"),
                Some((n, text)) => {
                    let mut count = 0usize;
                    let mut found: Option<u8> = None;
                    for &b in text.as_bytes() {
                        let is_vowel = matches!(
                            b,
                            b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'
                        );
                        if b.is_ascii_alphabetic() && !is_vowel {
                            count += 1;
                            if count == n {
                                found = Some(b);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(c) => crate::kprintln!("nthconsonant: {}", c as char),
                        None => crate::kprintln!("nthconsonant: (none)"),
                    }
                }
            }
        }
        "nthvowelfromend" => {
            // The Nth (1-based) VOWEL (a/e/i/o/u, case-insensitive; y excluded) counting from
            // the END of the inline text, skipping consonants and everything else.
            // "nthvowelfromend 1 hello -> o", "nthvowelfromend 2 hello -> e", "nthvowelfromend 1
            // AEIOU -> U"; an out-of-range index reports "(none)". The end-anchored mirror of
            // nthvowel (Nth vowel from the START), the vowel analogue of nthletterfromend.
            let r = rest.trim_start();
            let parsed = r
                .find(' ')
                .and_then(|sp| r[..sp].parse::<usize>().ok().filter(|&k| k >= 1).map(|k| (k, &r[sp + 1..])));
            match parsed {
                None => crate::kprintln!("usage: nthvowelfromend <n> <text>"),
                Some((n, text)) => {
                    let mut count = 0usize;
                    let mut found: Option<u8> = None;
                    for &b in text.as_bytes().iter().rev() {
                        if matches!(
                            b,
                            b'a' | b'e' | b'i' | b'o' | b'u' | b'A' | b'E' | b'I' | b'O' | b'U'
                        ) {
                            count += 1;
                            if count == n {
                                found = Some(b);
                                break;
                            }
                        }
                    }
                    match found {
                        Some(c) => crate::kprintln!("nthvowelfromend: {}", c as char),
                        None => crate::kprintln!("nthvowelfromend: (none)"),
                    }
                }
            }
        }
        "charat" => cmd_charat(line, cmd, rest),
        "linelen" => cmd_linelen(line, cmd, rest),
        "linewords" => cmd_linewords(line, cmd, rest),
        "wordsperline" => cmd_wordsperline(line, cmd, rest),
        "charsperline" => cmd_charsperline(line, cmd, rest),
        "firstword" => cmd_firstword(line, cmd, rest),
        "firstchars" => cmd_firstchars(line, cmd, rest),
        "lastchars" => cmd_lastchars(line, cmd, rest),
        "lastword" => cmd_lastword(line, cmd, rest),
        "maxline" => cmd_maxline(line, cmd, rest),
        "minline" => cmd_minline(line, cmd, rest),
        "wordindex" => cmd_wordindex(line, cmd, rest),
        "wordn" => cmd_wordn(line, cmd, rest),
        "wordat" => cmd_wordat(line, cmd, rest),
        "wordfromend" => cmd_wordfromend(line, cmd, rest),
        "revf" => cmd_revf(line, cmd, rest),
        "rwordsf" => cmd_rwordsf(line, cmd, rest),
        "rotlines" => cmd_rotlines(line, cmd, rest),
        "rotatef" => cmd_rotatef(line, cmd, rest),
        "rotwordsf" => cmd_rotwordsf(line, cmd, rest),
        "reveachf" => cmd_reveachf(line, cmd, rest),
        "avglen" => cmd_avglen(line, cmd, rest),
        "avgwordlen" => cmd_avgwordlen(line, cmd, rest),
        "bytesum" => {
            // The plain sum of all byte values in a VFS file (mod 2^64; files are
            // tiny so no overflow). The simplest additive checksum (Sigma of byte
            // values), used e.g. in serial protocols. Distinct from crc32 (a
            // polynomial CRC), fletcher16 (a positional running checksum), and
            // histogram (per-byte frequency).
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: bytesum <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("bytesum: no such file: {}", name),
                    Some(s) => {
                        let sum: u64 = s.as_bytes().iter().map(|&b| b as u64).sum();
                        crate::kprintln!("bytesum: {} over {} bytes", sum, s.len());
                    }
                }
            }
        }
        "lrc" => {
            // The XOR of all byte values in a VFS file -- an 8-bit Longitudinal
            // Redundancy Check, the checksum used in serial/track protocols.
            // Distinct from bytesum (an additive SUM), crc32 (a polynomial CRC),
            // and fletcher16 (a positional checksum): lrc XORs the bytes. bytesum
            // (sum) + lrc (xor) form a natural simple-checksum pair.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: lrc <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("lrc: no such file: {}", name),
                    Some(s) => {
                        let lrc = s.as_bytes().iter().fold(0u8, |acc, &b| acc ^ b);
                        crate::kprintln!("lrc: {:#04x} over {} bytes", lrc, s.len());
                    }
                }
            }
        }
        "inetsum" => {
            // The 16-bit one's-complement Internet checksum (RFC 1071) of a VFS
            // file's bytes -- the checksum used in IP/TCP/UDP headers. Distinct from
            // bytesum (plain u64 sum), lrc (XOR), fletcher16 (positional), and crc32
            // (polynomial): inetsum sums big-endian 16-bit words, folds the carries,
            // and complements. Ties to the kernel's networking subsystem.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: inetsum <file>");
            } else {
                match vfs::read_str(name) {
                    None => crate::kprintln!("inetsum: no such file: {}", name),
                    Some(s) => {
                        let b = s.as_bytes();
                        let mut sum: u32 = 0;
                        let mut i = 0;
                        while i + 1 < b.len() {
                            sum += ((b[i] as u32) << 8) | (b[i + 1] as u32);
                            i += 2;
                        }
                        if i < b.len() {
                            // Odd trailing byte -> high byte of a word padded with 0.
                            sum += (b[i] as u32) << 8;
                        }
                        // Fold the carries into the low 16 bits, then complement.
                        while (sum >> 16) != 0 {
                            sum = (sum & 0xffff) + (sum >> 16);
                        }
                        let checksum = (!sum) & 0xffff;
                        crate::kprintln!("inetsum: {:#06x} over {} bytes", checksum, b.len());
                    }
                }
            }
        }
        "count" => cmd_count(line, cmd, rest),
        "popcount" => {
            // Population count (set bits) + leading/trailing zeros of a u64.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let ones = n.count_ones();
                    let lz = n.leading_zeros();
                    let tz = if n == 0 { 64 } else { n.trailing_zeros() };
                    crate::kprintln!(
                        "popcount: {} (0x{:x}) -> {} set bits, {} leading zeros, {} trailing zeros",
                        n, n, ones, lz, tz
                    );
                }
                Err(_) => crate::kprintln!("usage: popcount <n>"),
            }
        }
        "parity" => {
            // The binary PARITY of n: the XOR of all its bits, i.e. its population count taken
            // mod 2. EVEN parity (0) means an even number of 1-bits, ODD parity (1) an odd number.
            // parity(0)=even (0 bits), parity(1)=odd (1 bit), parity(3)=even (2 bits), parity(7)=
            // odd (3 bits), parity(255)=even (8 bits). Distinct from popcount (the COUNT of set
            // bits), ispernicious (whether that count is PRIME), and ctz (trailing-zero count).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let ones = n.count_ones();
                    let p = ones & 1;
                    crate::kprintln!(
                        "parity: {} has {} set bits -> {} parity ({})",
                        n,
                        ones,
                        if p == 0 { "even" } else { "odd" },
                        p
                    );
                }
                Err(_) => crate::kprintln!("usage: parity <n>"),
            }
        }
        "ispernicious" => {
            // Classify whether n is a PERNICIOUS number: its population count (the
            // number of 1-bits in its binary form) is a PRIME. e.g. 3 = 0b11 has
            // 2 set bits (prime) so IS; 5 = 0b101 has 2 (prime) IS; 7 = 0b111 has
            // 3 (prime) IS; 4 = 0b100 has 1 (not prime) NOT. Distinct from popcount
            // (which only COUNTS the set bits) and isprime (which tests n itself).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let ones = n.count_ones() as u64;
                    // Primality of the (small, <= 64) set-bit count by trial division.
                    let prime = if ones < 2 {
                        false
                    } else {
                        let mut is_p = true;
                        let mut i = 2u64;
                        while i * i <= ones {
                            if ones % i == 0 {
                                is_p = false;
                                break;
                            }
                            i += 1;
                        }
                        is_p
                    };
                    if prime {
                        crate::kprintln!(
                            "ispernicious: {} IS pernicious ({} set bits, prime)",
                            n, ones
                        );
                    } else {
                        crate::kprintln!(
                            "ispernicious: {} is NOT pernicious ({} set bits, not prime)",
                            n, ones
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: ispernicious <n>"),
            }
        }
        "isevil" => {
            // Classify by the PARITY of the population count: n is EVIL when it has an
            // EVEN number of 1-bits in binary, and ODIOUS when the count is ODD.
            // 0=0b0 (0 ones) EVIL; 3=0b11 (2) EVIL; 5=0b101 (2) EVIL; 6=0b110 (2) EVIL;
            // 1/2/4 (1 one each) ODIOUS; 7=0b111 (3) ODIOUS. Distinct from popcount
            // (which COUNTS the bits), and from ispernicious (whose test is whether the
            // count is PRIME, not its parity).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let ones = n.count_ones();
                    if ones % 2 == 0 {
                        crate::kprintln!("isevil: {} IS evil ({} set bits, even)", n, ones);
                    } else {
                        crate::kprintln!(
                            "isevil: {} is NOT evil -- ODIOUS ({} set bits, odd)",
                            n, ones
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: isevil <n>"),
            }
        }
        "uartinfo" => cmd_uartinfo(line, cmd, rest),
        "uartflags" => cmd_uartflags(line, cmd, rest),
        "uart8250" => cmd_uart8250(line, cmd, rest),
        "ilog2" => {
            // Integer base-2 log floor(log2 n) + exact-power-of-two check.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("ilog2: undefined for 0"),
                Ok(n) => {
                    let l = 63 - n.leading_zeros();
                    let is_pow2 = n & (n - 1) == 0;
                    crate::kprintln!(
                        "ilog2: floor(log2({})) = {}{}",
                        n,
                        l,
                        if is_pow2 { " (exact power of 2)" } else { "" }
                    );
                }
                Err(_) => crate::kprintln!("usage: ilog2 <n>"),
            }
        }
        "ctz" => {
            // Count TRAILING ZERO bits of n: the exponent of the largest power of two dividing n
            // (its 2-adic valuation). ctz(8)=3 (0b1000), ctz(12)=2 (0b1100), ctz(6)=1, ctz(1)=0
            // (odd), ctz(1024)=10. n=0 has every bit zero, reported specially. Distinct from ilog2
            // (floor log2, the HIGH bit position), popcount (COUNT of set bits), and bitrev.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("ctz: 0 has no set bit (all 64 bits are zero)"),
                Ok(n) => {
                    let t = n.trailing_zeros();
                    crate::kprintln!(
                        "ctz: {} has {} trailing zero bit{} (2^{} divides it)",
                        n,
                        t,
                        if t == 1 { "" } else { "s" },
                        t
                    );
                }
                Err(_) => crate::kprintln!("usage: ctz <n>"),
            }
        }
        "clz" => {
            // Count LEADING ZERO bits of n in its 64-bit representation -- the number of high-order
            // zero bits before the most significant set bit. clz(1)=63, clz(2)=62, clz(255)=56,
            // clz(256)=55, clz(0)=64 (all bits zero). The width-64 complement of ctz (trailing
            // zeros); related to ilog2 by floor(log2 n) = 63 - clz(n) for n>=1. Distinct from ctz,
            // ilog2 (the high-bit POSITION), and popcount (the COUNT of set bits).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let lz = n.leading_zeros();
                    crate::kprintln!(
                        "clz: {} has {} leading zero bit{} (in 64 bits)",
                        n,
                        lz,
                        if lz == 1 { "" } else { "s" }
                    );
                }
                Err(_) => crate::kprintln!("usage: clz <n>"),
            }
        }
        "ilog" => cmd_ilog(line, cmd, rest),
        "prevpow2" => {
            // The LARGEST power of two that is <= n (round n DOWN to a power of two), reporting
            // the exponent -- the mirror of nextpow2 (which rounds UP). prevpow2(100)=64,
            // prevpow2(128)=128, prevpow2(1)=1, prevpow2(1000)=512, prevpow2(5)=4. Undefined
            // for 0. Distinct from ilog2 (the EXPONENT only), nextpow2 (rounds up), and issquare.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("prevpow2: undefined for 0"),
                Ok(n) => {
                    let e = 63 - n.leading_zeros();
                    let p = 1u64 << e;
                    crate::kprintln!(
                        "prevpow2: largest power of 2 <= {} is {} (2^{})",
                        n, p, e
                    );
                }
                Err(_) => crate::kprintln!("usage: prevpow2 <n>"),
            }
        }
        "nextpow2" => {
            // The SMALLEST power of two that is >= n (round n up to a power of two),
            // reporting the exponent. Distinct from ilog2 (FLOOR log2 -- the power
            // of 2 <= n), popcount (bit COUNT), pow (a^b), and gray (Gray code):
            // nextpow2 rounds UP.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut p: u64 = 1; // 2^0
                    let mut k: u32 = 0;
                    let mut overflow = false;
                    while p < n {
                        match p.checked_mul(2) {
                            Some(v) => {
                                p = v;
                                k += 1;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("nextpow2: no power of 2 >= {} fits in u64", n);
                    } else {
                        crate::kprintln!(
                            "nextpow2: smallest power of 2 >= {} is 2^{} = {}",
                            n, k, p
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: nextpow2 <n>"),
            }
        }
        "ismersenne" => {
            // Classify whether n is a MERSENNE number: n = 2^p - 1 for some p >= 1,
            // i.e. its binary form is all ones (1, 3, 7, 15, 31, 63, 127, ...). The
            // clean test is n >= 1 AND (n & (n + 1)) == 0, since n+1 is then a single
            // power of two; the exponent p is the bit length (= set-bit count here).
            // Distinct from nextpow2 (rounds UP to a power of 2), ilog2 (FLOOR log2),
            // and popcount (just the bit count). (Whether 2^p-1 is a Mersenne PRIME
            // is a further question this does not test.)
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // wrapping_add keeps u64::MAX (itself 2^64-1, all ones) from
                    // panicking on overflow: MAX+1 wraps to 0 and MAX & 0 == 0.
                    if n >= 1 && (n & n.wrapping_add(1)) == 0 {
                        let p = 64 - n.leading_zeros(); // bit length = exponent
                        crate::kprintln!(
                            "ismersenne: {} IS a Mersenne number (2^{} - 1)",
                            n, p
                        );
                    } else {
                        crate::kprintln!(
                            "ismersenne: {} is NOT a Mersenne number (not all-ones in binary)",
                            n
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: ismersenne <n>"),
            }
        }
        "longestrun" => {
            // The longest run of identical CONSECUTIVE characters in the text, reporting the
            // character and its run length; the FIRST such run wins a tie. "longestrun aaabbbbcc"
            // -> 'b' x 4, "longestrun abc" -> 'a' x 1, "longestrun mississippi" -> 's' x 2.
            // Distinct from squeeze/collapse (which REMOVE runs) and rle (which encodes ALL runs)
            // -- this reports just the maximum run.
            if rest.is_empty() {
                crate::kprintln!("usage: longestrun <text>");
            } else {
                let b = rest.as_bytes();
                let mut best_char = b[0];
                let mut best_len = 1usize;
                let mut cur_char = b[0];
                let mut cur_len = 1usize;
                for &c in &b[1..] {
                    if c == cur_char {
                        cur_len += 1;
                    } else {
                        cur_char = c;
                        cur_len = 1;
                    }
                    if cur_len > best_len {
                        best_len = cur_len;
                        best_char = cur_char;
                    }
                }
                crate::kprintln!("longestrun: '{}' x {}", best_char as char, best_len);
            }
        }
        "squeeze" => cmd_squeeze(line, cmd, rest),
        "dedupwords" => cmd_dedupwords(line, cmd, rest),
        "distinctwords" => cmd_distinctwords(line, cmd, rest),
        "hapax" => cmd_hapax(line, cmd, rest),
        "hapaxf" => cmd_hapaxf(line, cmd, rest),
        "uniqchars" => cmd_uniqchars(line, cmd, rest),
        "hapaxchars" => cmd_hapaxchars(line, cmd, rest),
        "hapaxcharsf" => cmd_hapaxcharsf(line, cmd, rest),
        "firstunique" => cmd_firstunique(line, cmd, rest),
        "firstrepeat" => cmd_firstrepeat(line, cmd, rest),
        "collapse" => cmd_collapse(line, cmd, rest),
        "gray" => {
            // Reflected binary Gray code (n ^ n>>1) + prefix-xor decode round-trip.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let g = n ^ (n >> 1);
                    // Decode g back to binary (should equal n).
                    let mut b = g;
                    let mut shift = 1;
                    while (b >> shift) != 0 {
                        b ^= b >> shift;
                        shift <<= 1;
                    }
                    crate::kprintln!("gray: {} -> gray 0x{:x} ({}) -> decoded {}", n, g, g, b);
                }
                Err(_) => crate::kprintln!("usage: gray <n>"),
            }
        }
        "ttbr" => {
            // Read TTBR0_EL1/TTBR1_EL1 (Translation Table Base Registers): the base
            // addresses of the low (user) and high (kernel) page tables + ASID.
            // Complements mmuinfo (SCTLR control) and tcrinfo (TCR layout config).
            let ttbr0: u64;
            let ttbr1: u64;
            // SAFETY: TTBR0_EL1/TTBR1_EL1 are readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0); #[cfg(not(target_arch = "aarch64"))] { ttbr0 = crate::shell_pc::sysreg("ttbr0_el1"); } #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, ttbr1_el1", out(reg) ttbr1); #[cfg(not(target_arch = "aarch64"))] { ttbr1 = crate::shell_pc::sysreg("ttbr1_el1"); }
            }
            // Table base = bits [47:0] with the top-16 ASID masked off; ASID = [63:48].
            let base0 = ttbr0 & 0x0000_ffff_ffff_ffff;
            let base1 = ttbr1 & 0x0000_ffff_ffff_ffff;
            let asid = (ttbr0 >> 48) & 0xffff;
            crate::kprintln!("ttbr: TTBR0_EL1 = 0x{:016x}", ttbr0);
            crate::kprintln!("ttbr: TTBR1_EL1 = 0x{:016x}", ttbr1);
            crate::kprintln!("ttbr: TTBR0 base = 0x{:x}  ASID = {}", base0, asid);
            crate::kprintln!("ttbr: TTBR1 base = 0x{:x}", base1);
        }
        "vbar" => {
            // Read VBAR_EL1 (Vector Base Address Register): the base of the EL1
            // exception-vector table. Arch requires 2KB alignment (low 11 bits 0).
            let vbar: u64;
            // SAFETY: VBAR_EL1 is readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, vbar_el1", out(reg) vbar); #[cfg(not(target_arch = "aarch64"))] { vbar = crate::shell_pc::sysreg("vbar_el1"); }
            }
            let aligned = vbar & 0x7ff == 0;
            crate::kprintln!("vbar: VBAR_EL1 = 0x{:016x}", vbar);
            crate::kprintln!("vbar: exception vector base = 0x{:x}", vbar);
            crate::kprintln!("vbar: 2KB-aligned = {}", if aligned { "yes" } else { "no" });
        }
        "cpacr" => {
            // Decode CPACR_EL1 (Architectural Feature Access Control): FPEN = FP/SIMD
            // trap/enable state, ZEN = SVE access. Distinct from mmuinfo (SCTLR).
            let cpacr: u64;
            // SAFETY: CPACR_EL1 is readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, cpacr_el1", out(reg) cpacr); #[cfg(not(target_arch = "aarch64"))] { cpacr = crate::shell_pc::sysreg("cpacr_el1"); }
            }
            let fpen = (cpacr >> 20) & 0x3;
            let zen = (cpacr >> 16) & 0x3;
            let desc = match fpen {
                0b11 => "enabled (no trap)",
                0b01 => "trapped at EL0 only",
                _ => "trapped at EL0/EL1",
            };
            crate::kprintln!("cpacr: CPACR_EL1 = 0x{:016x}", cpacr);
            crate::kprintln!("cpacr: FPEN = {} -- FP/SIMD {}", fpen, desc);
            crate::kprintln!("cpacr: ZEN = {} (SVE access)", zen);
        }
        "tcrinfo" => {
            // Decode TCR_EL1 (Translation Control Register): TTBR0 VA size + granule.
            let tcr: u64;
            // SAFETY: TCR_EL1 is readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, tcr_el1", out(reg) tcr); #[cfg(not(target_arch = "aarch64"))] { tcr = crate::shell_pc::sysreg("tcr_el1"); }
            }
            let t0sz = tcr & 0x3f;
            let va_bits = 64 - t0sz;
            let granule = match (tcr >> 14) & 0x3 {
                0b00 => "4KB",
                0b01 => "64KB",
                0b10 => "16KB",
                _ => "?",
            };
            let ips = match (tcr >> 32) & 0x7 {
                0 => "32-bit (4GB)",
                1 => "36-bit (64GB)",
                2 => "40-bit (1TB)",
                3 => "42-bit",
                4 => "44-bit",
                5 => "48-bit",
                6 => "52-bit",
                _ => "?",
            };
            crate::kprintln!("tcrinfo: TCR_EL1 = 0x{:016x}", tcr);
            crate::kprintln!("tcrinfo: TTBR0 VA size = {} bits (T0SZ={})", va_bits, t0sz);
            crate::kprintln!("tcrinfo: TTBR0 granule = {}", granule);
            crate::kprintln!("tcrinfo: phys addr size (IPS) = {}", ips);
        }
        "sumfibs" => {
            // The sum of the first n Fibonacci numbers F(1)+F(2)+...+F(n) (with F(1)=F(2)=1).
            // By the identity this equals F(n+2)-1: sumfibs(1)=1, sumfibs(5)=12, sumfibs(10)=143,
            // sumfibs(20)=17710, sumfibs(0)=0. Accumulated in u128 with checked_add so a large n
            // reports overflow. Distinct from fib (the nth term), nextfib, and sumto (1+2+..+n).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut a: u128 = 1; // F(1)
                    let mut b: u128 = 1; // F(2)
                    let mut sum: u128 = 0;
                    let mut overflow = false;
                    for _ in 0..n {
                        match sum.checked_add(a) {
                            Some(v) => sum = v,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                        match a.checked_add(b) {
                            Some(t) => {
                                a = b;
                                b = t;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("sumfibs: sum of first {} Fibonacci numbers overflowed u128", n);
                    } else {
                        crate::kprintln!("sumfibs: sum of first {} Fibonacci numbers is {}", n, sum);
                    }
                }
                Err(_) => crate::kprintln!("usage: sumfibs <n>"),
            }
        }
        "fib" => {
            // nth Fibonacci (F0=0, F1=1) with an honest u64 overflow guard. The
            // guard trips only when F(n) itself overflows -- b holds F(k) as k
            // climbs to n, so we never compute the F(n+1) lookahead needlessly.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("fib: F(0) = 0"),
                Ok(n) => {
                    let mut a: u64 = 0; // F(k-1)
                    let mut b: u64 = 1; // F(k), starting at k = 1
                    let mut overflow = false;
                    for _ in 1..n {
                        match a.checked_add(b) {
                            Some(s) => {
                                a = b;
                                b = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("fib: F({}) overflows u64 (n too large)", n);
                    } else {
                        crate::kprintln!("fib: F({}) = {}", n, b);
                    }
                }
                Err(_) => crate::kprintln!("usage: fib <n>"),
            }
        }
        "lucas" => {
            // nth Lucas number (L0=2, L1=1; same recurrence as fib, different seed)
            // with an honest u64 overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("lucas: L(0) = 2"),
                Ok(n) => {
                    let mut a: u64 = 2; // L(k-1)
                    let mut b: u64 = 1; // L(k), starting at k = 1
                    let mut overflow = false;
                    for _ in 1..n {
                        match a.checked_add(b) {
                            Some(s) => {
                                a = b;
                                b = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("lucas: L({}) overflows u64", n);
                    } else {
                        crate::kprintln!("lucas: L({}) = {}", n, b);
                    }
                }
                Err(_) => crate::kprintln!("usage: lucas <n>"),
            }
        }
        "tribonacci" => {
            // nth tribonacci (T0=0, T1=0, T2=1; a THREE-term recurrence, unlike the
            // 2-term fib/lucas) with an honest u64 overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("tribonacci: T(0) = 0"),
                Ok(1) => crate::kprintln!("tribonacci: T(1) = 0"),
                Ok(2) => crate::kprintln!("tribonacci: T(2) = 1"),
                Ok(n) => {
                    let mut a: u64 = 0; // T(k-3)
                    let mut b: u64 = 0; // T(k-2)
                    let mut c: u64 = 1; // T(k-1), starting at k = 2
                    let mut overflow = false;
                    for _ in 3..=n {
                        match a.checked_add(b).and_then(|x| x.checked_add(c)) {
                            Some(s) => {
                                a = b;
                                b = c;
                                c = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("tribonacci: T({}) overflows u64", n);
                    } else {
                        crate::kprintln!("tribonacci: T({}) = {}", n, c);
                    }
                }
                Err(_) => crate::kprintln!("usage: tribonacci <n>"),
            }
        }
        "tetranacci" => cmd_tetranacci(line, cmd, rest),
        "pentanacci" => cmd_pentanacci(line, cmd, rest),
        "hexanacci" => cmd_hexanacci(line, cmd, rest),
        "padovan" => cmd_padovan(line, cmd, rest),
        "narayana" => cmd_narayana(line, cmd, rest),
        "perrin" => {
            // nth Perrin number: SAME gap recurrence as padovan, P(n)=P(n-2)+P(n-3),
            // but seeds P(0)=3, P(1)=0, P(2)=2 -> 3,0,2,3,2,5,5,7,10,12,17,22,29,...
            // Distinct from padovan exactly as lucas is distinct from fib (same
            // recurrence, different seeds). Famous for the Perrin-pseudoprime
            // property: n divides Perrin(n) when n is prime. Honest overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("perrin: P(0) = 3"),
                Ok(1) => crate::kprintln!("perrin: P(1) = 0"),
                Ok(2) => crate::kprintln!("perrin: P(2) = 2"),
                Ok(n) => {
                    let mut a: u64 = 3; // P(k-3) = P(0)
                    let mut b: u64 = 0; // P(k-2) = P(1)
                    let mut c: u64 = 2; // P(k-1) = P(2), starting at k = 3
                    let mut overflow = false;
                    for _ in 3..=n {
                        // P(k) = P(k-2) + P(k-3) = b + a
                        match a.checked_add(b) {
                            Some(s) => {
                                a = b;
                                b = c;
                                c = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("perrin: P({}) overflows u64", n);
                    } else {
                        crate::kprintln!("perrin: P({}) = {}", n, c);
                    }
                }
                Err(_) => crate::kprintln!("usage: perrin <n>"),
            }
        }
        "pell" => {
            // nth Pell number: P(0)=0, P(1)=1, P(n)=2*P(n-1)+P(n-2) -> 0,1,2,5,
            // 12,29,70,169,408,985,... Distinct from fib/lucas (1*prev+1*prev2)
            // and tribonacci (three unit terms): the "2*" coefficient makes Pell
            // grow at the silver ratio 1+sqrt2 ~= 2.414, so it overflows u64 far
            // earlier than fib. Honest u64 overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("pell: P(0) = 0"),
                Ok(1) => crate::kprintln!("pell: P(1) = 1"),
                Ok(n) => {
                    let mut a: u64 = 0; // P(k-2)
                    let mut b: u64 = 1; // P(k-1), starting at k = 1
                    let mut overflow = false;
                    for _ in 2..=n {
                        match b.checked_mul(2).and_then(|x| x.checked_add(a)) {
                            Some(s) => {
                                a = b;
                                b = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("pell: P({}) overflows u64", n);
                    } else {
                        crate::kprintln!("pell: P({}) = {}", n, b);
                    }
                }
                Err(_) => crate::kprintln!("usage: pell <n>"),
            }
        }
        "sylvester" => {
            // nth term of Sylvester's sequence: a(0)=2, a(n)=a(n-1)^2 - a(n-1) + 1
            // = a(n-1)*(a(n-1)-1) + 1 -> 2,3,7,43,1807,3263443,10650056950807,...
            // The FIRST NONLINEAR (quadratic) recurrence here: it SQUARES the prior
            // term rather than summing prior terms linearly, giving DOUBLY-exponential
            // growth (each term is one more than the product of all previous terms),
            // so it overflows u64 at the earliest n yet (~7). Honest overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("sylvester: a(0) = 2"),
                Ok(n) => {
                    let mut a: u64 = 2;
                    let mut overflow = false;
                    for _ in 1..=n {
                        // a - 1 never underflows: Sylvester terms are strictly
                        // increasing from 2, so a >= 2 throughout.
                        match a.checked_mul(a - 1).and_then(|x| x.checked_add(1)) {
                            Some(v) => a = v,
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("sylvester: a({}) overflows u64", n);
                    } else {
                        crate::kprintln!("sylvester: a({}) = {}", n, a);
                    }
                }
                Err(_) => crate::kprintln!("usage: sylvester <n>"),
            }
        }
        "jacobsthal" => {
            // nth Jacobsthal number: J(0)=0, J(1)=1, J(n)=J(n-1)+2*J(n-2) ->
            // 0,1,1,3,5,11,21,43,85,171,341,... (closed form (2^n-(-1)^n)/3, so it
            // grows ~2^n/3). Distinct from fib (prev1+prev2) and pell (2*prev1+
            // prev2): here the "2*" is on the SECOND-back term (prev2). Honest u64
            // overflow guard.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("jacobsthal: J(0) = 0"),
                Ok(1) => crate::kprintln!("jacobsthal: J(1) = 1"),
                Ok(n) => {
                    let mut a: u64 = 0; // J(k-2)
                    let mut b: u64 = 1; // J(k-1), starting at k = 1
                    let mut overflow = false;
                    for _ in 2..=n {
                        // next = J(k-1) + 2*J(k-2) = b + 2*a
                        match a.checked_mul(2).and_then(|x| x.checked_add(b)) {
                            Some(s) => {
                                a = b;
                                b = s;
                            }
                            None => {
                                overflow = true;
                                break;
                            }
                        }
                    }
                    if overflow {
                        crate::kprintln!("jacobsthal: J({}) overflows u64", n);
                    } else {
                        crate::kprintln!("jacobsthal: J({}) = {}", n, b);
                    }
                }
                Err(_) => crate::kprintln!("usage: jacobsthal <n>"),
            }
        }
        "derangement" => cmd_derangement(line, cmd, rest),
        "stirling2" => cmd_stirling2(line, cmd, rest),
        "lah" => cmd_lah(line, cmd, rest),
        "eulerian" => cmd_eulerian(line, cmd, rest),
        "stirling1" => cmd_stirling1(line, cmd, rest),
        "bell" => cmd_bell(line, cmd, rest),
        "partition" => cmd_partition(line, cmd, rest),
        "dedup" => cmd_dedup(line, cmd, rest),
        "digitsum" => {
            // Decimal digit sum + iterated digital root of a u64.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        s += x % 10;
                        x /= 10;
                    }
                    // Digital root: repeat the digit-sum until a single digit.
                    let mut r = s;
                    while r >= 10 {
                        let mut t = 0u64;
                        let mut y = r;
                        while y > 0 {
                            t += y % 10;
                            y /= 10;
                        }
                        r = t;
                    }
                    crate::kprintln!("digitsum: {} -> digit sum {}, digital root {}", n, s, r);
                }
                Err(_) => crate::kprintln!("usage: digitsum <n>"),
            }
        }
        "sumdigitsquares" => {
            // Sum of the SQUARES of the decimal digits (OEIS A003132) -- the single step of the
            // happy-number iteration. "sumdigitsquares 23 -> 13" (2^2+3^2 = 4+9), "sumdigitsquares
            // 7 -> 49", "sumdigitsquares 100 -> 1", "sumdigitsquares 999 -> 243", "0 -> 0".
            // Distinct from digitsum (sum of digits) and digitprod (product of digits).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        let d = x % 10;
                        s += d * d;
                        x /= 10;
                    }
                    crate::kprintln!("sumdigitsquares: {} -> {}", n, s);
                }
                Err(_) => crate::kprintln!("usage: sumdigitsquares <n>"),
            }
        }
        "sumdigitcubes" => {
            // Sum of the CUBES of the decimal digits (OEIS A055012) -- the cube analogue of
            // sumdigitsquares and the basis of the Armstrong/narcissistic 3-digit check.
            // "sumdigitcubes 153 -> 153" (1+125+27, a fixed point), "sumdigitcubes 371 -> 371",
            // "sumdigitcubes 9 -> 729", "sumdigitcubes 100 -> 1", "0 -> 0". Distinct from
            // sumdigitsquares (squares), digitsum (sum), and digitprod (product).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        let d = x % 10;
                        s += d * d * d;
                        x /= 10;
                    }
                    crate::kprintln!("sumdigitcubes: {} -> {}", n, s);
                }
                Err(_) => crate::kprintln!("usage: sumdigitcubes <n>"),
            }
        }
        "digitsumbase" => {
            // Digit sum of n written in base b (2..=36): repeatedly add n % b. Generalizes
            // digitsum (base 10) to any base: "digitsumbase 16 255" -> 30 (FF), "digitsumbase 2
            // 7" -> 3 (111 = popcount), "digitsumbase 10 123" -> 6, "digitsumbase 16 256" -> 1
            // (100), "digitsumbase 5 100" -> 4 (400). Distinct from digitsum (fixed base 10) and
            // popcount (which is exactly the base-2 case). A digit-sum in an arbitrary radix.
            let mut it = rest.split_whitespace();
            let b = it.next().and_then(|s| s.parse::<u64>().ok());
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            match (b, n) {
                (Some(b), Some(n)) if (2..=36).contains(&b) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        s += x % b;
                        x /= b;
                    }
                    crate::kprintln!("digitsumbase: digit sum of {} in base {} is {}", n, b, s);
                }
                _ => crate::kprintln!("usage: digitsumbase <b> <n> (2 <= b <= 36)"),
            }
        }
        "altdigitsum" => {
            // The ALTERNATING digit sum from the right: d0 - d1 + d2 - d3 + ... A
            // number is divisible by 11 iff this alternating sum is a multiple of 11
            // (121 -> 0 divisible; 2728 -> 8-2+7-2 = 11 divisible; 1234 -> 4-3+2-1 = 2
            // not). The sum is bounded by +-180 for a u64 so i64 never overflows.
            // Distinct from digitsum (plain sum + digital root) and digitprod.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut x = n;
                    let mut sum: i64 = 0;
                    let mut sign: i64 = 1;
                    while x > 0 {
                        sum += sign * (x % 10) as i64;
                        sign = -sign;
                        x /= 10;
                    }
                    // sum % 11 == 0 detects multiples of 11 for negative sums too.
                    if sum % 11 == 0 {
                        crate::kprintln!(
                            "altdigitsum: {} has alternating digit sum {} (divisible by 11)",
                            n, sum
                        );
                    } else {
                        crate::kprintln!(
                            "altdigitsum: {} has alternating digit sum {} (not divisible by 11)",
                            n, sum
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: altdigitsum <n>"),
            }
        }
        "addper" => {
            // ADDITIVE PERSISTENCE: how many repeated digit-SUM steps it takes to
            // reduce n to a single digit (7->0 steps; 39->12->3 = 2; 199->19->10
            // ->1 = 3; 9999->36->9 = 2; single digits including 0 -> 0). The SUM
            // analog of digitprod's multiplicative persistence. Distinct from
            // digitsum, which prints the digit sum + digital ROOT (the RESULT)
            // but NOT the step COUNT.
            fn dsum(mut x: u64) -> u64 {
                let mut s = 0u64;
                while x > 0 {
                    s += x % 10;
                    x /= 10;
                }
                s
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut cur = n;
                    let mut steps = 0u32;
                    while cur >= 10 {
                        cur = dsum(cur);
                        steps += 1;
                    }
                    crate::kprintln!(
                        "addper: {} has additive persistence {} (reaches {})",
                        n, steps, cur
                    );
                }
                Err(_) => crate::kprintln!("usage: addper <n>"),
            }
        }
        "digitcount" => {
            // The number of DECIMAL DIGITS of n (0 has 1 digit). Distinct from
            // digitsum (SUM of digits), digitprod (PRODUCT of digits), ilog2 (floor
            // log2, a base-2 magnitude), and popcount (bit count) -- this is the
            // base-10 length.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut d = 0u32;
                    let mut x = n;
                    while x > 0 {
                        d += 1;
                        x /= 10;
                    }
                    if d == 0 {
                        d = 1; // n == 0 has one digit
                    }
                    crate::kprintln!("digitcount: {} has {} digits", n, d);
                }
                Err(_) => crate::kprintln!("usage: digitcount <n>"),
            }
        }
        "countdigit" => {
            // Count how many times the digit <d> (0..9) appears in the decimal representation of
            // <n>. "countdigit 1 12131 -> 3", "countdigit 0 1000 -> 3", "countdigit 5 12345 -> 1",
            // "countdigit 9 12345 -> 0", "countdigit 0 0 -> 1". Distinct from digitcount (the TOTAL
            // number of digits), maxdigit (extreme digit values), and digitsum (SUM of digits).
            let mut it = rest.split_whitespace();
            let d = it.next().and_then(|s| s.parse::<u64>().ok());
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            match (d, n) {
                (Some(d), Some(n)) if d <= 9 => {
                    let mut count = 0u32;
                    let mut x = n;
                    if x == 0 {
                        if d == 0 {
                            count = 1; // n == 0 is the single digit 0
                        }
                    } else {
                        while x > 0 {
                            if x % 10 == d {
                                count += 1;
                            }
                            x /= 10;
                        }
                    }
                    crate::kprintln!("countdigit: {} occurs {} times in {}", d, count, n);
                }
                _ => crate::kprintln!("usage: countdigit <d> <n>  (d in 0..9)"),
            }
        }
        "evendigits" => {
            // Count how many of n's DECIMAL digits are EVEN vs ODD (0,2,4,6,8 vs
            // 1,3,5,7,9). 2468 -> 4 even, 0 odd; 1357 -> 0 even, 4 odd;
            // 1234567890 -> 5 even, 5 odd; 0 -> 1 even, 0 odd. Distinct from
            // digitcount (TOTAL digit count), digitsum (SUM), and digitprod
            // (PRODUCT) -- this splits the digits by parity.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut even = 0u32;
                    let mut odd = 0u32;
                    let mut x = n;
                    // n == 0 has the single digit 0 (even); the loop below would
                    // skip it, so seed that case explicitly.
                    if n == 0 {
                        even = 1;
                    }
                    while x > 0 {
                        if (x % 10) % 2 == 0 {
                            even += 1;
                        } else {
                            odd += 1;
                        }
                        x /= 10;
                    }
                    crate::kprintln!("evendigits: {} has {} even, {} odd", n, even, odd);
                }
                Err(_) => crate::kprintln!("usage: evendigits <n>"),
            }
        }
        "sumevendigits" => {
            // Sum of the EVEN-valued decimal digits of n (0,2,4,6,8). "sumevendigits 12345 -> 6"
            // (2+4), "sumevendigits 2468 -> 20", "sumevendigits 13579 -> 0" (all odd),
            // "sumevendigits 246 -> 12", "sumevendigits 0 -> 0". Distinct from digitsum (sum of ALL
            // digits), evendigits (COUNTS even/odd digits), and sumdigitsquares.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        let d = x % 10;
                        if d % 2 == 0 {
                            s += d;
                        }
                        x /= 10;
                    }
                    crate::kprintln!("sumevendigits: {} -> {}", n, s);
                }
                Err(_) => crate::kprintln!("usage: sumevendigits <n>"),
            }
        }
        "sumodddigits" => {
            // Sum of the ODD-valued decimal digits of n (1,3,5,7,9) -- the complement of
            // sumevendigits. "sumodddigits 12345 -> 9" (1+3+5), "sumodddigits 2468 -> 0" (all
            // even), "sumodddigits 13579 -> 25", "sumodddigits 135 -> 9", "sumodddigits 0 -> 0".
            // Note sumevendigits(n) + sumodddigits(n) = digitsum(n).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut s = 0u64;
                    let mut x = n;
                    while x > 0 {
                        let d = x % 10;
                        if d % 2 == 1 {
                            s += d;
                        }
                        x /= 10;
                    }
                    crate::kprintln!("sumodddigits: {} -> {}", n, s);
                }
                Err(_) => crate::kprintln!("usage: sumodddigits <n>"),
            }
        }
        "maxdigit" => {
            // Report BOTH the largest and smallest DECIMAL digit of n in one
            // line (2468 -> max 8, min 2; 90210 -> max 9, min 0; 555 -> max 5,
            // min 5; 0 -> max 0, min 0). Distinct from digitsum (SUM), digitcount
            // (COUNT), and evendigits (even/odd split) -- this reports the
            // extreme digit values.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut mx = 0u8;
                    let mut mn = 9u8;
                    if n == 0 {
                        mx = 0;
                        mn = 0;
                    }
                    let mut x = n;
                    while x > 0 {
                        let d = (x % 10) as u8;
                        if d > mx {
                            mx = d;
                        }
                        if d < mn {
                            mn = d;
                        }
                        x /= 10;
                    }
                    crate::kprintln!("maxdigit: {} max {}, min {}", n, mx, mn);
                }
                Err(_) => crate::kprintln!("usage: maxdigit <n>"),
            }
        }
        "rotdigits" => {
            // Rotate the DECIMAL digits of n LEFT by one: move the most-significant
            // digit to the least-significant end. 1234 -> 2341, 90210 -> 2109 (leading
            // zero collapses: 02109 = 2109), 1000 -> 1 (0001), 5 -> 5, 0 -> 0. Distinct
            // from revnum (REVERSE the digit order) and rotl/rotr (BIT rotation).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // Highest power of ten <= n gives the most-significant place.
                    let mut p = 1u64;
                    while p <= n / 10 {
                        p *= 10;
                    }
                    let msd = n / p; // most-significant digit
                    let rest_digits = n % p; // everything below it
                    let rotated = rest_digits * 10 + msd;
                    crate::kprintln!("rotdigits: {} -> {}", n, rotated);
                }
                Err(_) => crate::kprintln!("usage: rotdigits <n>"),
            }
        }
        "nthdigit" => {
            // The kth digit (1-based, counting from the MOST-SIGNIFICANT / left) of n. "nthdigit
            // 1 12345 -> 1", "nthdigit 3 12345 -> 3", "nthdigit 5 12345 -> 5", "nthdigit 1 7 -> 7";
            // a k past the digit count reports "(out of range)". A digit-EXTRACTION primitive,
            // distinct from maxdigit (max/min), digitcount (COUNT), and digitsum (SUM).
            let mut it = rest.split_whitespace();
            let k = it.next().and_then(|s| s.parse::<usize>().ok());
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            match (k, n) {
                (Some(k), Some(n)) if k >= 1 => {
                    // Collect digits least-significant-first, then index from the left.
                    let mut digits = [0u8; 20];
                    let mut cnt = 0usize;
                    let mut x = n;
                    if x == 0 {
                        digits[0] = 0;
                        cnt = 1;
                    } else {
                        while x > 0 {
                            digits[cnt] = (x % 10) as u8;
                            cnt += 1;
                            x /= 10;
                        }
                    }
                    if k <= cnt {
                        crate::kprintln!("nthdigit: {}", digits[cnt - k]);
                    } else {
                        crate::kprintln!("nthdigit: (out of range)");
                    }
                }
                _ => crate::kprintln!("usage: nthdigit <k> <n>"),
            }
        }
        "nthdigitfromend" => {
            // The kth digit (1-based) counting from the RIGHT / least-significant end of n =
            // (n / 10^(k-1)) mod 10. "nthdigitfromend 1 12345 -> 5", "nthdigitfromend 2 12345 ->
            // 4", "nthdigitfromend 5 12345 -> 1", "nthdigitfromend 1 7 -> 7"; a k past the digit
            // count reports "(out of range)". The numeric end-anchored mirror of nthdigit (kth
            // from the LEFT); the digit analogue of charfromend.
            let mut it = rest.split_whitespace();
            let k = it.next().and_then(|s| s.parse::<usize>().ok());
            let n = it.next().and_then(|s| s.parse::<u64>().ok());
            match (k, n) {
                (Some(k), Some(n)) if k >= 1 => {
                    // Collect digits least-significant-first; the kth from the end is index k-1.
                    let mut digits = [0u8; 20];
                    let mut cnt = 0usize;
                    let mut x = n;
                    if x == 0 {
                        digits[0] = 0;
                        cnt = 1;
                    } else {
                        while x > 0 {
                            digits[cnt] = (x % 10) as u8;
                            cnt += 1;
                            x /= 10;
                        }
                    }
                    if k <= cnt {
                        crate::kprintln!("nthdigitfromend: {}", digits[k - 1]);
                    } else {
                        crate::kprintln!("nthdigitfromend: (out of range)");
                    }
                }
                _ => crate::kprintln!("usage: nthdigitfromend <k> <n>"),
            }
        }
        "isrepdigit" => {
            // Classify whether n is a REPDIGIT (monodigit): all of its decimal
            // digits are identical (7, 11, 55, 777, 999999, ...). Single-digit
            // numbers (including 0) trivially qualify. A plain digit walk comparing
            // every digit to the last one -- no overflow possible. Distinct from
            // maxdigit (reports the extreme digits), digitcount (COUNTS digits), and
            // uniqchars (which dedups a text string, not a number).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let first = (n % 10) as u8; // the last digit
                    let mut x = n;
                    let mut all_same = true;
                    while x > 0 {
                        if (x % 10) as u8 != first {
                            all_same = false;
                            break;
                        }
                        x /= 10;
                    }
                    // n == 0 is a single digit, so the (never-entered) loop leaves
                    // all_same = true, which is correct.
                    if all_same {
                        crate::kprintln!(
                            "isrepdigit: {} IS a repdigit (all digits are {})",
                            n, first
                        );
                    } else {
                        crate::kprintln!("isrepdigit: {} is NOT a repdigit", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: isrepdigit <n>"),
            }
        }
        "ispandigital" => {
            // Classify whether n is (1-9) PANDIGITAL: its decimal digits include
            // every digit from 1 to 9 at least once (123456789 IS, 1234567890 IS --
            // the extra 0 is ignored, 12345 is NOT because 6-9 are missing). A digit-
            // set membership test with a 10-slot seen table; the smallest missing
            // digit is reported. Distinct from isrepdigit (all digits the SAME) and
            // isdigitsorted (digits non-decreasing).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut seen = [false; 10];
                    let mut x = n;
                    if x == 0 {
                        seen[0] = true;
                    }
                    while x > 0 {
                        seen[(x % 10) as usize] = true;
                        x /= 10;
                    }
                    let mut missing = 0u8;
                    for d in 1..=9u8 {
                        if !seen[d as usize] {
                            missing = d;
                            break;
                        }
                    }
                    if missing == 0 {
                        crate::kprintln!(
                            "ispandigital: {} IS pandigital (has every digit 1-9)",
                            n
                        );
                    } else {
                        crate::kprintln!(
                            "ispandigital: {} is NOT pandigital (missing {})",
                            n, missing
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: ispandigital <n>"),
            }
        }
        "isstrobogrammatic" => {
            // Classify whether n is STROBOGRAMMATIC: it reads the same after a
            // 180-degree rotation, where 0/1/8 map to themselves and 6<->9 swap
            // (69, 88, 96, 101, 818, 609, 6009 IS). A digit like 2,3,4,5,7 (no
            // rotation) disqualifies, and 6 alone rotates to 9 so is NOT. Distinct
            // from palindrome/revnum (same REVERSED, no glyph remap) and isrepdigit.
            fn rot(d: u8) -> Option<u8> {
                match d {
                    0 => Some(0),
                    1 => Some(1),
                    8 => Some(8),
                    6 => Some(9),
                    9 => Some(6),
                    _ => None,
                }
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // Collect little-endian digits (digits[0] = ones place).
                    let mut digits = [0u8; 20];
                    let mut k = 0usize;
                    let mut x = n;
                    if x == 0 {
                        k = 1; // the single digit 0
                    }
                    while x > 0 {
                        digits[k] = (x % 10) as u8;
                        k += 1;
                        x /= 10;
                    }
                    // n is strobogrammatic iff rot(digits[i]) == digits[k-1-i] for all i.
                    let mut strobo = true;
                    for i in 0..k {
                        match rot(digits[i]) {
                            Some(r) if r == digits[k - 1 - i] => {}
                            _ => {
                                strobo = false;
                                break;
                            }
                        }
                    }
                    if strobo {
                        crate::kprintln!("isstrobogrammatic: {} IS strobogrammatic", n);
                    } else {
                        crate::kprintln!("isstrobogrammatic: {} is NOT strobogrammatic", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: isstrobogrammatic <n>"),
            }
        }
        "isdigitsorted" => {
            // Classify whether n's decimal digits are NON-DECREASING left to right
            // (1234, 1122, 1119 IS; 4321, 1032 NOT). Single-digit numbers including
            // 0 trivially qualify. A right-to-left digit walk: each more-significant
            // digit must be <= the digit to its right (no overflow possible).
            // Distinct from isrepdigit (all digits EQUAL) and maxdigit (the extreme
            // digit values); this checks digit ORDER.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let mut prev = (n % 10) as u8; // rightmost digit
                    let mut x = n / 10;
                    let mut sorted = true;
                    while x > 0 {
                        let cur = (x % 10) as u8; // next digit to the LEFT
                        if cur > prev {
                            sorted = false;
                            break;
                        }
                        prev = cur;
                        x /= 10;
                    }
                    // single digit (incl 0): the loop never runs, sorted stays true.
                    if sorted {
                        crate::kprintln!(
                            "isdigitsorted: {} IS digit-sorted (digits non-decreasing)",
                            n
                        );
                    } else {
                        crate::kprintln!(
                            "isdigitsorted: {} is NOT digit-sorted",
                            n
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: isdigitsorted <n>"),
            }
        }
        "isbouncy" => {
            // Classify whether n is a BOUNCY number: its decimal digits are NEITHER
            // non-decreasing (an "increasing" number like 134468) NOR non-increasing
            // (a "decreasing" number like 66420) left to right. The smallest bouncy
            // number is 101; every number < 100 is non-bouncy. Distinct from
            // isdigitsorted (which tests ONLY the non-decreasing direction).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // Walk adjacent digit pairs from the right; track both directions.
                    let mut prev = (n % 10) as u8; // rightmost (less significant)
                    let mut x = n / 10;
                    let mut nondec = true; // no digit decreases going left -> right
                    let mut noninc = true; // no digit increases going left -> right
                    while x > 0 {
                        let cur = (x % 10) as u8; // next digit to the LEFT (more sig.)
                        if cur > prev {
                            nondec = false; // left digit bigger => a decrease L->R
                        }
                        if cur < prev {
                            noninc = false; // left digit smaller => an increase L->R
                        }
                        prev = cur;
                        x /= 10;
                    }
                    if !nondec && !noninc {
                        crate::kprintln!("isbouncy: {} IS bouncy", n);
                    } else if nondec {
                        crate::kprintln!(
                            "isbouncy: {} is NOT bouncy (digits non-decreasing)",
                            n
                        );
                    } else {
                        crate::kprintln!(
                            "isbouncy: {} is NOT bouncy (digits non-increasing)",
                            n
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: isbouncy <n>"),
            }
        }
        "isundulating" => cmd_isundulating(line, cmd, rest),
        "isfactorion" => cmd_isfactorion(line, cmd, rest),
        "ismunchausen" => cmd_ismunchausen(line, cmd, rest),
        "digitprod" => {
            // Decimal digit PRODUCT + multiplicative persistence + multiplicative
            // digital root of a u64 (the multiplicative parallel of digitsum).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    fn dprod(mut x: u64) -> u64 {
                        if x == 0 {
                            return 0;
                        }
                        let mut p: u64 = 1;
                        while x > 0 {
                            p = p.saturating_mul(x % 10);
                            x /= 10;
                        }
                        p
                    }
                    let one = dprod(n);
                    // Multiplicative persistence: iterate the digit product to one digit.
                    let mut cur = n;
                    let mut steps = 0u32;
                    while cur >= 10 {
                        cur = dprod(cur);
                        steps += 1;
                    }
                    crate::kprintln!("digitprod: {} digit product = {}", n, one);
                    crate::kprintln!(
                        "digitprod: multiplicative persistence = {}, root = {}",
                        steps, cur
                    );
                }
                Err(_) => crate::kprintln!("usage: digitprod <n>"),
            }
        }
        "armstrong" => cmd_armstrong(line, cmd, rest),
        "isdisarium" => cmd_isdisarium(line, cmd, rest),
        "automorphic" => cmd_automorphic(line, cmd, rest),
        "istrimorphic" => cmd_istrimorphic(line, cmd, rest),
        "happy" => {
            // Classify whether n is a HAPPY number: repeatedly replace n with the
            // sum of the squares of its decimal digits; if this reaches 1 the number
            // is happy, otherwise it falls into the cycle 4->16->37->58->89->145->
            // 42->20->4 and is unhappy. Distinct from digitsum (plain sum),
            // digitprod (product), armstrong (digit^k sum), automorphic (n^2 ends
            // in n), and collatz (3n+1).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    fn ssd(mut x: u64) -> u64 {
                        let mut s = 0u64;
                        while x > 0 {
                            let d = x % 10;
                            s += d * d;
                            x /= 10;
                        }
                        s
                    }
                    // Iterate until 1 (happy) or 4 (in the unhappy cycle). 0 maps to
                    // 0 forever, so treat cur==0 as unhappy to avoid an infinite loop.
                    let mut cur = n;
                    let is_happy = loop {
                        if cur == 1 {
                            break true;
                        }
                        if cur == 4 || cur == 0 {
                            break false;
                        }
                        cur = ssd(cur);
                    };
                    if is_happy {
                        crate::kprintln!("happy: {} is a happy number", n);
                    } else {
                        crate::kprintln!("happy: {} is NOT a happy number (unhappy)", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: happy <n>"),
            }
        }
        "harshad" => cmd_harshad(line, cmd, rest),
        "ispolydivisible" => cmd_ispolydivisible(line, cmd, rest),
        "isgapful" => {
            // Classify whether n (>= 100) is a GAPFUL number: divisible by the two-
            // digit number formed by concatenating its FIRST and LAST decimal digits
            // (100 -> 10, 100 % 10 == 0 IS; 108 -> 18, 108 % 18 == 0 IS; 106 -> 16,
            // 106 % 16 != 0 NOT). The concept needs at least three digits, so n < 100
            // is reported out of range. Distinct from harshad (divisible by its digit
            // SUM) and iszuckerman (divisible by its digit PRODUCT).
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 100 => {
                    let mut f = n;
                    while f >= 10 {
                        f /= 10; // leading digit
                    }
                    let concat = f * 10 + n % 10; // first*10 + last, always 10..=99
                    if n % concat == 0 {
                        crate::kprintln!("isgapful: {} IS gapful (divisible by {})", n, concat);
                    } else {
                        crate::kprintln!(
                            "isgapful: {} is NOT gapful (not divisible by {})",
                            n, concat
                        );
                    }
                }
                Ok(n) => {
                    crate::kprintln!("isgapful: {} is out of range (needs n >= 100)", n)
                }
                Err(_) => crate::kprintln!("usage: isgapful <n>"),
            }
        }
        "iszuckerman" => cmd_iszuckerman(line, cmd, rest),
        "triangular" => cmd_triangular(line, cmd, rest),
        "nexttriangular" => cmd_nexttriangular(line, cmd, rest),
        "prevtriangular" => {
            // The LARGEST triangular number T(k)=k(k+1)/2 that is <= n, reporting the
            // index k -- the mirror of nexttriangular (which rounds UP). prevtriangular(11)
            // = T(4) = 10, prevtriangular(10) = T(4) = 10 (already triangular),
            // prevtriangular(9) = T(3) = 6, prevtriangular(1) = T(1) = 1,
            // prevtriangular(0) = T(0) = 0. Uses the floor triangular index
            // k = (sqrt(8n+1)-1)/2 directly. With nexttriangular it brackets n between
            // consecutive triangular numbers. Distinct from triangular (a MEMBERSHIP test)
            // and sumto (the nth triangular by index).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let s = isqrt(8 * nn + 1);
                    let k = (s - 1) / 2; // floor index: T(k) <= n < T(k+1)
                    let t = k * (k + 1) / 2; // <= n, largest triangular <= n
                    crate::kprintln!(
                        "prevtriangular: largest triangular <= {} is T({}) = {}",
                        n, k as u64, t as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: prevtriangular <n>"),
            }
        }
        "tetrahedral" => {
            // Compute the nth TETRAHEDRAL number Te(n) = n*(n+1)*(n+2)/6 -- the count of
            // spheres stacked in a triangular pyramid, i.e. the sum of the first n
            // triangular numbers. Te(1..7) = 1, 4, 10, 20, 35, 56, 84. The 3D analogue
            // of sumto (the nth triangular number); distinct from the figurate
            // MEMBERSHIP tests (triangular/square/pentagonal/...) which classify a given
            // n. The triple product is computed in u128 (checked) to avoid overflow.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let a = n as u128;
                    let prod = a.checked_mul(a + 1).and_then(|x| x.checked_mul(a + 2));
                    match prod {
                        Some(p) => crate::kprintln!("tetrahedral: Te({}) = {}", n, p / 6),
                        None => crate::kprintln!("tetrahedral: Te({}) overflowed u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: tetrahedral <n>"),
            }
        }
        "squarepyramidal" => {
            // Compute the nth SQUARE PYRAMIDAL number SP(n) = n*(n+1)*(2n+1)/6 -- the
            // count of spheres stacked in a square pyramid, i.e. the sum of the first n
            // SQUARES (1 + 4 + 9 + ...). SP(1..6) = 1, 5, 14, 30, 55, 91 (OEIS A000330).
            // Distinct from tetrahedral (sum of TRIANGULAR numbers, n(n+1)(n+2)/6) and
            // from the figurate MEMBERSHIP tests. The triple product is computed in u128
            // (checked) to avoid overflow; n(n+1)(2n+1) is always divisible by 6.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let a = n as u128;
                    let prod = a.checked_mul(a + 1).and_then(|x| x.checked_mul(2 * a + 1));
                    match prod {
                        Some(p) => crate::kprintln!("squarepyramidal: SP({}) = {}", n, p / 6),
                        None => crate::kprintln!("squarepyramidal: SP({}) overflowed u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: squarepyramidal <n>"),
            }
        }
        "pentatope" => {
            // Compute the nth PENTATOPE number P(n) = n*(n+1)*(n+2)*(n+3)/24 -- the 4D
            // simplex (4-simplex / pentachoron) figurate number, i.e. the sum of the
            // first n tetrahedral numbers. P(1..7) = 1, 5, 15, 35, 70, 126, 210. The 4D
            // analogue of tetrahedral (3D) and sumto (2D triangular); distinct from the
            // figurate MEMBERSHIP tests. The quadruple product is computed in u128
            // (checked) to avoid overflow.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let a = n as u128;
                    let prod = a
                        .checked_mul(a + 1)
                        .and_then(|x| x.checked_mul(a + 2))
                        .and_then(|x| x.checked_mul(a + 3));
                    match prod {
                        Some(p) => crate::kprintln!("pentatope: P({}) = {}", n, p / 24),
                        None => crate::kprintln!("pentatope: P({}) overflowed u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: pentatope <n>"),
            }
        }
        "stellaoctangula" => {
            // The nth STELLA OCTANGULA number: n(2n^2 - 1) -- the count of spheres in a stella
            // octangula (two interpenetrating tetrahedra), OEIS A007588: 1, 14, 51, 124, 245, ...
            // Distinct from octahedral (n(2n^2+1)/3) and the pyramidal / centered figurate
            // numbers. Computed in u128 (checked) to avoid overflow; 2n^2 - 1 >= 1 for n >= 1.
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let a = n as u128;
                    let val = a
                        .checked_mul(a)
                        .and_then(|sq| sq.checked_mul(2))
                        .map(|two_sq| two_sq - 1)
                        .and_then(|t| a.checked_mul(t));
                    match val {
                        Some(v) => crate::kprintln!("stellaoctangula: S({}) = {}", n, v),
                        None => crate::kprintln!("stellaoctangula: S({}) overflowed u128", n),
                    }
                }
                _ => crate::kprintln!("usage: stellaoctangula <n> (n >= 1)"),
            }
        }
        "octahedral" => {
            // Compute the nth OCTAHEDRAL number O(n) = n*(2n^2 + 1)/3 -- the count of spheres
            // packed in an octahedron (two square pyramids joined base-to-base). O(1..7) = 1,
            // 6, 19, 44, 85, 146, 231 (OEIS A005900). The 3D companion of tetrahedral,
            // squarepyramidal, and pentatope; distinct from the figurate MEMBERSHIP tests.
            // n(2n^2+1) is always divisible by 3; computed in u128 (checked) to avoid overflow.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let a = n as u128;
                    let val = a
                        .checked_mul(a)
                        .and_then(|sq| sq.checked_mul(2))
                        .and_then(|two_sq| two_sq.checked_add(1))
                        .and_then(|t| a.checked_mul(t));
                    match val {
                        Some(p) => crate::kprintln!("octahedral: O({}) = {}", n, p / 3),
                        None => crate::kprintln!("octahedral: O({}) overflowed u128", n),
                    }
                }
                Err(_) => crate::kprintln!("usage: octahedral <n>"),
            }
        }
        "pronic" => cmd_pronic(line, cmd, rest),
        "issquare" => {
            // Classify whether n is a PERFECT SQUARE (n = k^2), reporting the root
            // k. Distinct from isqrt (which COMPUTES the floor square root of any n)
            // and the figurate classifiers triangular/pronic: issquare tests
            // perfect-square membership.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let s = isqrt(nn);
                    if s * s == nn {
                        crate::kprintln!("issquare: {} IS a perfect square ({}^2 = {})", n, s as u64, n);
                    } else {
                        crate::kprintln!("issquare: {} is NOT a perfect square", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: issquare <n>"),
            }
        }
        "iscube" => {
            // Classify whether n is a PERFECT CUBE (n = k^3), reporting the root k.
            // Distinct from icbrt (which COMPUTES the floor cube root of any n),
            // issquare (perfect squares), and isperfectpower (any a^b): iscube tests
            // perfect-cube membership. Integer cube root by binary search in u128 so
            // mid^3 (mid <= cbrt(u64::MAX) ~ 2.64e6) never overflows.
            fn icbrt_u64(n: u64) -> u64 {
                let nn = n as u128;
                let mut lo = 0u64;
                let mut hi = 2_642_246u64; // ceil(cbrt(u64::MAX))
                while lo < hi {
                    let mid = (lo + hi + 1) / 2;
                    let m = mid as u128;
                    if m * m * m <= nn {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                lo
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let c = icbrt_u64(n);
                    if (c as u128) * (c as u128) * (c as u128) == n as u128 {
                        crate::kprintln!("iscube: {} IS a perfect cube ({}^3 = {})", n, c, n);
                    } else {
                        crate::kprintln!("iscube: {} is NOT a perfect cube", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: iscube <n>"),
            }
        }
        "isperfectpower" => cmd_isperfectpower(line, cmd, rest),
        "isprimepower" => cmd_isprimepower(line, cmd, rest),
        "nextfib" => {
            // The smallest Fibonacci number STRICTLY GREATER THAN n, from the sequence 0, 1, 1,
            // 2, 3, 5, 8, 13, ... nextfib(10)=13, nextfib(13)=21, nextfib(100)=144, nextfib(0)=1,
            // nextfib(1)=2. Companion of nextprime/nextsquare/nextpow2; distinct from isfib (a
            // membership test) and fib (the nth term). Generated in u128 to avoid overflow near u64.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let target = n as u128;
                    let mut a: u128 = 0;
                    let mut b: u128 = 1;
                    while a <= target {
                        let t = a + b;
                        a = b;
                        b = t;
                    }
                    crate::kprintln!("nextfib: next Fibonacci after {} is {}", n, a);
                }
                Err(_) => crate::kprintln!("usage: nextfib <n>"),
            }
        }
        "prevfib" => {
            // The largest Fibonacci number STRICTLY LESS THAN n (0, 1, 1, 2, 3, 5, 8, 13, ...) --
            // the mirror of nextfib. prevfib(10)=8, prevfib(13)=8, prevfib(100)=89, prevfib(1)=0,
            // prevfib(2)=1. n = 0 reports none (no Fibonacci below 0). Distinct from isfib (a
            // membership test) and fib (the nth term). Generated in u128.
            match rest.trim().parse::<u64>() {
                Ok(0) => crate::kprintln!("prevfib: no Fibonacci below 0"),
                Ok(n) => {
                    let target = n as u128;
                    let mut a: u128 = 0;
                    let mut b: u128 = 1;
                    let mut last: u128 = 0;
                    while a < target {
                        last = a;
                        let t = a + b;
                        a = b;
                        b = t;
                    }
                    crate::kprintln!("prevfib: prev Fibonacci before {} is {}", n, last);
                }
                Err(_) => crate::kprintln!("usage: prevfib <n>"),
            }
        }
        "prevsquare" => {
            // The LARGEST perfect square that is <= n (round n DOWN to a square), reporting the
            // root -- the mirror of nextsquare (which rounds UP). prevsquare(10)=9 (3^2),
            // prevsquare(16)=16 (4^2), prevsquare(17)=16, prevsquare(1)=1. With nextsquare it
            // brackets n between consecutive squares. Distinct from issquare (a MEMBERSHIP test),
            // isqrt (the floor root VALUE), and prevpow2 (a power of TWO down).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let r = isqrt(n as u128) as u64;
                    let sq = r * r;
                    crate::kprintln!("prevsquare: largest square <= {} is {} ({}^2)", n, sq, r);
                }
                Err(_) => crate::kprintln!("usage: prevsquare <n>"),
            }
        }
        "nextsquare" => cmd_nextsquare(line, cmd, rest),
        "nextcube" => {
            // The SMALLEST perfect cube that is >= n (round n up to a perfect cube),
            // reporting the root. The cube analogue of nextsquare. Distinct from icbrt
            // (the FLOOR cube root), iscube (a MEMBERSHIP test), and nextsquare (rounds
            // up to a SQUARE): nextcube rounds UP to a cube.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let target = n as u128;
                    // Floor cube root via binary search (same bound as icbrt).
                    let mut lo: u64 = 0;
                    let mut hi: u64 = 2_642_246;
                    while lo < hi {
                        let mid = (lo + hi + 1) / 2;
                        if (mid as u128).pow(3) <= target {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    // Round up to the next root if n is not already a perfect cube.
                    let k = if (lo as u128).pow(3) == target { lo } else { lo + 1 };
                    let cube = (k as u128).pow(3); // >= n, smallest perfect cube >= n
                    if cube > u64::MAX as u128 {
                        crate::kprintln!("nextcube: no perfect cube >= {} fits in u64", n);
                    } else {
                        crate::kprintln!(
                            "nextcube: smallest perfect cube >= {} is {}^3 = {}",
                            n, k, cube as u64
                        );
                    }
                }
                Err(_) => crate::kprintln!("usage: nextcube <n>"),
            }
        }
        "prevcube" => {
            // The LARGEST perfect cube that is <= n (round n DOWN to a perfect cube),
            // reporting the root -- the mirror of nextcube (which rounds UP). prevcube(30)
            // = 27 (3^3), prevcube(27) = 27, prevcube(8) = 8, prevcube(7) = 1, prevcube(0)
            // = 0. With nextcube it brackets n between consecutive cubes. Distinct from
            // icbrt (the floor root VALUE), iscube (a MEMBERSHIP test), and prevsquare.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let target = n as u128;
                    // Floor cube root via binary search (same bound as icbrt / nextcube).
                    let mut lo: u64 = 0;
                    let mut hi: u64 = 2_642_246;
                    while lo < hi {
                        let mid = (lo + hi + 1) / 2;
                        if (mid as u128).pow(3) <= target {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    let cube = (lo as u128).pow(3) as u64; // <= n, largest perfect cube <= n
                    crate::kprintln!("prevcube: largest cube <= {} is {} ({}^3)", n, cube, lo);
                }
                Err(_) => crate::kprintln!("usage: prevcube <n>"),
            }
        }
        "polygonal" => cmd_polygonal(line, cmd, rest),
        "pentagonal" => {
            // Classify whether n is a PENTAGONAL number P(k)=k(3k-1)/2 (k>=1:
            // 1,5,12,22,35,...), reporting the index k. n is pentagonal iff 24n+1
            // is a perfect square AND its root % 6 == 5 (since 24*P(k)+1=(6k-1)^2);
            // then k=(root+1)/6. A different figurate family from triangular
            // (k(k+1)/2), pronic (k(k+1)), and issquare (k^2).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 24u128 * (n as u128) + 1;
                    let s = isqrt(m);
                    if s * s == m && s % 6 == 5 {
                        let k = ((s + 1) / 6) as u64;
                        crate::kprintln!("pentagonal: {} IS pentagonal (P({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("pentagonal: {} is NOT pentagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: pentagonal <n>"),
            }
        }
        "nextpentagonal" => {
            // The SMALLEST pentagonal number P(k)=k(3k-1)/2 (k>=1) that is >= n, reporting
            // the index k -- the pentagonal member of the nextsquare/nextcube/nexttriangular
            // family. nextpentagonal(2)=P(2)=5, nextpentagonal(12)=P(3)=12 (already
            // pentagonal), nextpentagonal(13)=P(4)=22, nextpentagonal(0)=P(1)=1,
            // nextpentagonal(36)=P(6)=51. Estimates k from k=(1+sqrt(24n+1))/6 then adjusts
            // by one either way. Distinct from pentagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn penta(k: u128) -> u128 {
                k * (3 * k - 1) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(24 * nn + 1)) / 6;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with P(k) >= n.
                    while penta(k) < nn {
                        k += 1;
                    }
                    while k > 1 && penta(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextpentagonal: smallest pentagonal >= {} is P({}) = {}",
                        n, k as u64, penta(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextpentagonal <n>"),
            }
        }
        "prevpentagonal" => {
            // The LARGEST pentagonal number P(k)=k(3k-1)/2 (k>=1) that is <= n, reporting
            // the index k -- the mirror of nextpentagonal (which rounds UP). prevpentagonal(13)
            // = P(3) = 12, prevpentagonal(12) = P(3) = 12 (already pentagonal), prevpentagonal(4)
            // = P(2) = 5? no: P(1)=1,P(2)=5 so 4 -> P(1) = 1, prevpentagonal(1) = P(1) = 1,
            // prevpentagonal(50) = P(5) = 35. With nextpentagonal it brackets n. n<1 has no
            // pentagonal (k>=1), reported as "(none)". Distinct from pentagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn penta(k: u128) -> u128 {
                k * (3 * k - 1) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(24 * nn + 1)) / 6;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with P(k) <= n.
                    while penta(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && penta(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevpentagonal: largest pentagonal <= {} is P({}) = {}",
                        n, k as u64, penta(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevpentagonal: no pentagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevpentagonal <n>"),
            }
        }
        "hexagonal" => {
            // Classify whether n is a HEXAGONAL number H(k)=k(2k-1) (k>=1:
            // 1,6,15,28,45,...), reporting the index k. n is hexagonal iff 8n+1 is
            // a perfect square AND its root % 4 == 3 (since 8*H(k)+1=(4k-1)^2); then
            // k=(root+1)/4. Rounds out the figurate family (triangular/pronic/
            // issquare/pentagonal). NOTE every hexagonal number is also triangular
            // (H(k)=T(2k-1)), but the membership test is distinct.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 8u128 * (n as u128) + 1;
                    let s = isqrt(m);
                    if s * s == m && s % 4 == 3 {
                        let k = ((s + 1) / 4) as u64;
                        crate::kprintln!("hexagonal: {} IS hexagonal (H({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("hexagonal: {} is NOT hexagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: hexagonal <n>"),
            }
        }
        "nexthexagonal" => {
            // The SMALLEST hexagonal number H(k)=k(2k-1) (k>=1: 1,6,15,28,45,...) that is
            // >= n, reporting the index k -- the hexagonal member of the nextsquare/nextcube/
            // nexttriangular/nextpentagonal family. nexthexagonal(2)=H(2)=6, nexthexagonal(6)
            // =H(2)=6 (already hexagonal), nexthexagonal(7)=H(3)=15, nexthexagonal(0)=H(1)=1,
            // nexthexagonal(29)=H(5)=45. Estimates k from k=(1+sqrt(8n+1))/4 then adjusts by
            // one either way. Distinct from hexagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hexa(k: u128) -> u128 {
                k * (2 * k - 1)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(8 * nn + 1)) / 4;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with H(k) >= n.
                    while hexa(k) < nn {
                        k += 1;
                    }
                    while k > 1 && hexa(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nexthexagonal: smallest hexagonal >= {} is H({}) = {}",
                        n, k as u64, hexa(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nexthexagonal <n>"),
            }
        }
        "prevhexagonal" => {
            // The LARGEST hexagonal number H(k)=k(2k-1) (k>=1: 1,6,15,28,45,...) that is <= n,
            // reporting the index k -- the mirror of nexthexagonal (which rounds UP), matching
            // how prevpentagonal mirrors nextpentagonal. prevhexagonal(7)=H(2)=6, prevhexagonal(6)
            // =H(2)=6 (already hexagonal), prevhexagonal(5)=H(1)=1, prevhexagonal(1)=H(1)=1,
            // prevhexagonal(50)=H(5)=45. With nexthexagonal it brackets n. n<1 has no hexagonal
            // (k>=1), reported as "(none)". Distinct from hexagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hexa(k: u128) -> u128 {
                k * (2 * k - 1)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(8 * nn + 1)) / 4;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with H(k) <= n.
                    while hexa(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && hexa(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevhexagonal: largest hexagonal <= {} is H({}) = {}",
                        n, k as u64, hexa(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevhexagonal: no hexagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevhexagonal <n>"),
            }
        }
        "centeredcube" => cmd_centeredcube(line, cmd, rest),
        "centeredhex" => cmd_centeredhex(line, cmd, rest),
        "centeredsquare" => cmd_centeredsquare(line, cmd, rest),
        "centeredtriangular" => cmd_centeredtriangular(line, cmd, rest),
        "centeredpentagonal" => cmd_centeredpentagonal(line, cmd, rest),
        "centeredheptagonal" => cmd_centeredheptagonal(line, cmd, rest),
        "starnumber" => cmd_starnumber(line, cmd, rest),
        "heptagonal" => {
            // Classify whether n is a HEPTAGONAL number Hep(k)=k(5k-3)/2 (k>=1:
            // 1,7,18,34,55,...), reporting the index k. n is heptagonal iff
            // 40n+9 is a perfect square AND its root % 10 == 7 (since
            // 40*Hep(k)+9=(10k-3)^2); then k=(root+3)/10. Extends the figurate
            // family (triangular/pronic/issquare/pentagonal/hexagonal) with its
            // own distinct root-gate.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 40u128 * (n as u128) + 9;
                    let s = isqrt(m);
                    if s * s == m && s % 10 == 7 {
                        let k = ((s + 3) / 10) as u64;
                        crate::kprintln!("heptagonal: {} IS heptagonal (Hep({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("heptagonal: {} is NOT heptagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: heptagonal <n>"),
            }
        }
        "nextheptagonal" => {
            // The SMALLEST heptagonal number Hep(k)=k(5k-3)/2 (k>=1: 1,7,18,34,55,...) that is
            // >= n, reporting the index k -- the heptagonal member of the nextsquare/nextcube/
            // nexttriangular/nextpentagonal/nexthexagonal family. nextheptagonal(2)=Hep(2)=7,
            // nextheptagonal(7)=Hep(2)=7 (already heptagonal), nextheptagonal(8)=Hep(3)=18,
            // nextheptagonal(0)=Hep(1)=1, nextheptagonal(35)=Hep(5)=55. Estimates k from
            // k=(3+sqrt(40n+9))/10 then adjusts by one either way. Distinct from heptagonal
            // (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hepta(k: u128) -> u128 {
                k * (5 * k - 3) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (3 + isqrt(40 * nn + 9)) / 10;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Hep(k) >= n.
                    while hepta(k) < nn {
                        k += 1;
                    }
                    while k > 1 && hepta(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextheptagonal: smallest heptagonal >= {} is Hep({}) = {}",
                        n, k as u64, hepta(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextheptagonal <n>"),
            }
        }
        "prevheptagonal" => {
            // The LARGEST heptagonal number Hep(k)=k(5k-3)/2 (k>=1: 1,7,18,34,55,...) that is
            // <= n, reporting the index k -- the mirror of nextheptagonal (which rounds UP),
            // matching how prevhexagonal mirrors nexthexagonal. prevheptagonal(8)=Hep(2)=7,
            // prevheptagonal(7)=Hep(2)=7 (already heptagonal), prevheptagonal(6)=Hep(1)=1,
            // prevheptagonal(1)=Hep(1)=1, prevheptagonal(50)=Hep(4)=34. With nextheptagonal it
            // brackets n. n<1 has no heptagonal (k>=1), reported as "(none)". Distinct from
            // heptagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hepta(k: u128) -> u128 {
                k * (5 * k - 3) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (3 + isqrt(40 * nn + 9)) / 10;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Hep(k) <= n.
                    while hepta(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && hepta(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevheptagonal: largest heptagonal <= {} is Hep({}) = {}",
                        n, k as u64, hepta(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevheptagonal: no heptagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevheptagonal <n>"),
            }
        }
        "octagonal" => {
            // Classify whether n is an OCTAGONAL number Oct(k)=k(3k-2) (k>=1:
            // 1,8,21,40,65,...), reporting the index k. n is octagonal iff 3n+1
            // is a perfect square AND its root % 3 == 2 (since 3*Oct(k)+1=
            // (3k-1)^2); then k=(root+1)/3. Extends the figurate family
            // (triangular/pronic/issquare/pentagonal/hexagonal/heptagonal) with
            // its own distinct root-gate.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 3u128 * (n as u128) + 1;
                    let s = isqrt(m);
                    if s * s == m && s % 3 == 2 {
                        let k = ((s + 1) / 3) as u64;
                        crate::kprintln!("octagonal: {} IS octagonal (Oct({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("octagonal: {} is NOT octagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: octagonal <n>"),
            }
        }
        "nextoctagonal" => {
            // The SMALLEST octagonal number Oct(k)=k(3k-2) (k>=1: 1,8,21,40,65,...) that is
            // >= n, reporting the index k -- the octagonal member of the nextsquare/nextcube/
            // nexttriangular/nextpentagonal/nexthexagonal/nextheptagonal family. nextoctagonal(2)
            // =Oct(2)=8, nextoctagonal(8)=Oct(2)=8 (already octagonal), nextoctagonal(9)=Oct(3)=21,
            // nextoctagonal(0)=Oct(1)=1, nextoctagonal(50)=Oct(5)=65. Estimates k from
            // k=(1+sqrt(3n+1))/3 then adjusts by one either way. Distinct from octagonal
            // (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn octa(k: u128) -> u128 {
                k * (3 * k - 2)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(3 * nn + 1)) / 3;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Oct(k) >= n.
                    while octa(k) < nn {
                        k += 1;
                    }
                    while k > 1 && octa(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextoctagonal: smallest octagonal >= {} is Oct({}) = {}",
                        n, k as u64, octa(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextoctagonal <n>"),
            }
        }
        "prevoctagonal" => {
            // The LARGEST octagonal number Oct(k)=k(3k-2) (k>=1: 1,8,21,40,65,...) that is <= n,
            // reporting the index k -- the mirror of nextoctagonal (which rounds UP), matching how
            // prevheptagonal mirrors nextheptagonal. prevoctagonal(9)=Oct(2)=8, prevoctagonal(8)=
            // Oct(2)=8 (already octagonal), prevoctagonal(7)=Oct(1)=1, prevoctagonal(1)=Oct(1)=1,
            // prevoctagonal(50)=Oct(4)=40. With nextoctagonal it brackets n. n<1 has no octagonal
            // (k>=1), reported as "(none)". Distinct from octagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn octa(k: u128) -> u128 {
                k * (3 * k - 2)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (1 + isqrt(3 * nn + 1)) / 3;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Oct(k) <= n.
                    while octa(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && octa(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevoctagonal: largest octagonal <= {} is Oct({}) = {}",
                        n, k as u64, octa(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevoctagonal: no octagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevoctagonal <n>"),
            }
        }
        "nonagonal" => {
            // Classify whether n is a NONAGONAL (9-gonal) number Non(k)=k(7k-5)/2 (k>=1:
            // 1,9,24,46,75,111,...), reporting the index k. Solving 7k^2-5k-2n=0 gives
            // k=(5+sqrt(56n+25))/14, so n is nonagonal iff 56n+25 is a perfect square AND
            // its root == -5 (mod 14); then k=(root+5)/14. Extends the figurate family
            // (triangular..octagonal) with its own distinct root-gate. u128 avoids overflow.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 56u128 * (n as u128) + 25;
                    let s = isqrt(m);
                    if s * s == m && (s + 5) % 14 == 0 {
                        let k = ((s + 5) / 14) as u64;
                        crate::kprintln!("nonagonal: {} IS nonagonal (Non({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("nonagonal: {} is NOT nonagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: nonagonal <n>"),
            }
        }
        "nextnonagonal" => {
            // The SMALLEST nonagonal number Non(k)=k(7k-5)/2 (k>=1: 1,9,24,46,75,111,...) that is
            // >= n, reporting the index k -- the nonagonal member of the nextsquare/.../nextoctagonal
            // family. nextnonagonal(2)=Non(2)=9, nextnonagonal(9)=Non(2)=9 (already nonagonal),
            // nextnonagonal(10)=Non(3)=24, nextnonagonal(0)=Non(1)=1, nextnonagonal(50)=Non(5)=75.
            // Estimates k from k=(5+sqrt(56n+25))/14 then adjusts by one either way. Distinct from
            // nonagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn nona(k: u128) -> u128 {
                k * (7 * k - 5) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (5 + isqrt(56 * nn + 25)) / 14;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Non(k) >= n.
                    while nona(k) < nn {
                        k += 1;
                    }
                    while k > 1 && nona(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextnonagonal: smallest nonagonal >= {} is Non({}) = {}",
                        n, k as u64, nona(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextnonagonal <n>"),
            }
        }
        "prevnonagonal" => {
            // The LARGEST nonagonal number Non(k)=k(7k-5)/2 (k>=1: 1,9,24,46,75,111,...) that is
            // <= n, reporting the index k -- the mirror of nextnonagonal (which rounds UP), matching
            // how prevoctagonal mirrors nextoctagonal. prevnonagonal(10)=Non(2)=9, prevnonagonal(9)=
            // Non(2)=9 (already nonagonal), prevnonagonal(8)=Non(1)=1, prevnonagonal(1)=Non(1)=1,
            // prevnonagonal(50)=Non(4)=46. With nextnonagonal it brackets n. n<1 has no nonagonal
            // (k>=1), reported as "(none)". Distinct from nonagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn nona(k: u128) -> u128 {
                k * (7 * k - 5) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (5 + isqrt(56 * nn + 25)) / 14;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Non(k) <= n.
                    while nona(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && nona(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevnonagonal: largest nonagonal <= {} is Non({}) = {}",
                        n, k as u64, nona(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevnonagonal: no nonagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevnonagonal <n>"),
            }
        }
        "decagonal" => {
            // Classify whether n is a DECAGONAL (10-gonal) number Dec(k)=k(4k-3) (k>=1:
            // 1,10,27,52,85,126,...), reporting the index k. Solving 4k^2-3k-n=0 gives
            // k=(3+sqrt(16n+9))/8, so n is decagonal iff 16n+9 is a perfect square AND its
            // root == -3 (mod 8); then k=(root+3)/8. Completes the figurate family
            // (triangular..nonagonal) with its own distinct root-gate. u128 avoids overflow.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 16u128 * (n as u128) + 9;
                    let s = isqrt(m);
                    if s * s == m && (s + 3) % 8 == 0 {
                        let k = ((s + 3) / 8) as u64;
                        crate::kprintln!("decagonal: {} IS decagonal (Dec({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("decagonal: {} is NOT decagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: decagonal <n>"),
            }
        }
        "hendecagonal" => {
            // Classify whether n is a HENDECAGONAL (11-gonal) number Hen(k)=k(9k-7)/2 (k>=1:
            // 1,11,30,58,95,141,...), reporting the index k. Solving 9k^2-7k-2n=0 gives
            // k=(7+sqrt(49+72n))/18, so n is hendecagonal iff 49+72n is a perfect square AND
            // its root == -7 (mod 18); then k=(root+7)/18. Extends the figurate MEMBERSHIP
            // family (triangular..decagonal) to 11 sides with its own root-gate. u128 avoids
            // overflow. Distinct from the next*/prev* round-to-figurate family.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 72u128 * (n as u128) + 49;
                    let s = isqrt(m);
                    if s * s == m && (s + 7) % 18 == 0 {
                        let k = ((s + 7) / 18) as u64;
                        crate::kprintln!(
                            "hendecagonal: {} IS hendecagonal (Hen({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("hendecagonal: {} is NOT hendecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: hendecagonal <n>"),
            }
        }
        "dodecagonal" => {
            // Classify whether n is a DODECAGONAL (12-gonal) number Dod(k)=k(5k-4) (k>=1:
            // 1,12,33,64,105,156,...), reporting the index k. Solving 5k^2-4k-n=0 gives
            // k=(2+sqrt(5n+4))/5, so n is dodecagonal iff 5n+4 is a perfect square AND its root
            // == -2 (mod 5); then k=(root+2)/5. Extends the figurate MEMBERSHIP family
            // (triangular..hendecagonal) to 12 sides with its own root-gate. u128 avoids overflow.
            // Distinct from the next*/prev* round-to-figurate family.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 5u128 * (n as u128) + 4;
                    let s = isqrt(m);
                    if s * s == m && (s + 2) % 5 == 0 {
                        let k = ((s + 2) / 5) as u64;
                        crate::kprintln!("dodecagonal: {} IS dodecagonal (Dod({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("dodecagonal: {} is NOT dodecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: dodecagonal <n>"),
            }
        }
        "tridecagonal" => {
            // Classify whether n is a TRIDECAGONAL (13-gonal) number Tridec(k)=k(11k-9)/2 (k>=1:
            // 1,13,36,70,115,171,...), reporting the index k. Solving 11k^2-9k-2n=0 gives
            // k=(9+sqrt(81+88n))/22, so n is tridecagonal iff 81+88n is a perfect square AND its
            // root == -9 (mod 22); then k=(root+9)/22. Extends the figurate MEMBERSHIP family
            // (triangular..dodecagonal) to 13 sides with its own root-gate. u128 avoids overflow.
            // Distinct from the next*/prev* round-to-figurate family (not yet added for 13-gon).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 88u128 * (n as u128) + 81;
                    let s = isqrt(m);
                    if s * s == m && (s + 9) % 22 == 0 {
                        let k = ((s + 9) / 22) as u64;
                        crate::kprintln!(
                            "tridecagonal: {} IS tridecagonal (Tridec({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("tridecagonal: {} is NOT tridecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: tridecagonal <n>"),
            }
        }
        "tetradecagonal" => {
            // Classify whether n is a TETRADECAGONAL (14-gonal) number Tetradec(k)=k(6k-5) (k>=1:
            // 1,14,39,76,125,186,...), reporting the index k. Solving 6k^2-5k-n=0 gives
            // k=(5+sqrt(24n+25))/12, so n is tetradecagonal iff 24n+25 is a perfect square AND its
            // root == -5 (mod 12); then k=(root+5)/12. Extends the figurate MEMBERSHIP family
            // (triangular..tridecagonal) to 14 sides with its own root-gate. u128 avoids overflow.
            // Distinct from the next*/prev* round-to-figurate family (not yet added for 14-gon).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 24u128 * (n as u128) + 25;
                    let s = isqrt(m);
                    if s * s == m && (s + 5) % 12 == 0 {
                        let k = ((s + 5) / 12) as u64;
                        crate::kprintln!(
                            "tetradecagonal: {} IS tetradecagonal (Tetradec({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("tetradecagonal: {} is NOT tetradecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: tetradecagonal <n>"),
            }
        }
        "pentadecagonal" => {
            // Classify whether n is a PENTADECAGONAL (15-gonal) number Pentadec(k)=k(13k-11)/2
            // (k>=1: 1,15,42,82,135,201,...), reporting the index k. Solving 13k^2-11k-2n=0 gives
            // k=(11+sqrt(104n+121))/26, so n is pentadecagonal iff 104n+121 is a perfect square AND
            // its root == -11 (mod 26); then k=(root+11)/26. Extends the figurate MEMBERSHIP family
            // (triangular..tetradecagonal) to 15 sides with its own root-gate. u128 avoids overflow.
            // Distinct from the next*/prev* round-to-figurate family (not yet added for 15-gon).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 104u128 * (n as u128) + 121;
                    let s = isqrt(m);
                    if s * s == m && (s + 11) % 26 == 0 {
                        let k = ((s + 11) / 26) as u64;
                        crate::kprintln!(
                            "pentadecagonal: {} IS pentadecagonal (Pentadec({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("pentadecagonal: {} is NOT pentadecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: pentadecagonal <n>"),
            }
        }
        "hexadecagonal" => {
            // Classify whether n is a HEXADECAGONAL (16-gonal) number Hexadec(k)=k(7k-6) (k>=1:
            // 1,16,45,88,145,216,...), reporting the index k. Unlike the 15-gon (odd side count,
            // which needs the k(13k-11)/2 half-form), 16 sides give integer coefficients directly:
            // solving 7k^2-6k-n=0 gives k=(3+sqrt(7n+9))/7, so n is hexadecagonal iff 7n+9 is a
            // perfect square AND its root == -3 (mod 7); then k=(root+3)/7. Extends the figurate
            // MEMBERSHIP family (triangular..pentadecagonal) to 16 sides with its own root-gate.
            // u128 avoids overflow. Distinct from the next*/prev* round-to-figurate family.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 7u128 * (n as u128) + 9;
                    let s = isqrt(m);
                    if s * s == m && (s + 3) % 7 == 0 {
                        let k = ((s + 3) / 7) as u64;
                        crate::kprintln!(
                            "hexadecagonal: {} IS hexadecagonal (Hexadec({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("hexadecagonal: {} is NOT hexadecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: hexadecagonal <n>"),
            }
        }
        "heptadecagonal" => {
            // Classify whether n is a HEPTADECAGONAL (17-gonal) number Heptadec(k)=k(15k-13)/2
            // (k>=1: 1,17,48,94,155,231,322,...), reporting the index k. 17 sides is odd, so we
            // are back to the half-form (like the 13-gon/15-gon) rather than the 16-gon's integer
            // coefficients: solving 15k^2-13k-2n=0 gives k=(13+sqrt(120n+169))/30, so n is
            // heptadecagonal iff 120n+169 is a perfect square AND its root == -13 (mod 30); then
            // k=(root+13)/30. The root-gate is what rejects n=10 (120n+169=1369=37^2 is a perfect
            // square, but 37+13=50 is not a multiple of 30). Extends the figurate MEMBERSHIP family
            // (triangular..hexadecagonal) to 17 sides. u128 avoids overflow on the discriminant.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 120u128 * (n as u128) + 169;
                    let s = isqrt(m);
                    if s * s == m && (s + 13) % 30 == 0 {
                        let k = ((s + 13) / 30) as u64;
                        crate::kprintln!(
                            "heptadecagonal: {} IS heptadecagonal (Heptadec({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("heptadecagonal: {} is NOT heptadecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: heptadecagonal <n>"),
            }
        }
        "octadecagonal" => {
            // Classify whether n is an OCTADECAGONAL (18-gonal) number Oct18(k)=k(8k-7) (k>=1:
            // 1,18,51,100,165,246,343,...), reporting the index k. 18 sides is even, so -- like the
            // 16-gon and unlike the 17-gon's k(15k-13)/2 half-form -- the coefficients stay integral:
            // solving 8k^2-7k-n=0 gives k=(7+sqrt(32n+49))/16, so n is octadecagonal iff 32n+49 is a
            // perfect square AND its root == -7 (mod 16); then k=(root+7)/16. The root-gate is what
            // rejects n=15 (32n+49=529=23^2 is a perfect square, but 23+7=30 is not a multiple of 16).
            // Extends the figurate MEMBERSHIP family (triangular..heptadecagonal) to 18 sides.
            // u128 avoids overflow on the discriminant.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 32u128 * (n as u128) + 49;
                    let s = isqrt(m);
                    if s * s == m && (s + 7) % 16 == 0 {
                        let k = ((s + 7) / 16) as u64;
                        crate::kprintln!(
                            "octadecagonal: {} IS octadecagonal (Oct18({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("octadecagonal: {} is NOT octadecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: octadecagonal <n>"),
            }
        }
        "enneadecagonal" => {
            // Classify whether n is an ENNEADECAGONAL (19-gonal) number Ennea19(k)=k(17k-15)/2
            // (k>=1: 1,19,54,106,175,261,364,...), reporting the index k. 19 sides is ODD, so this is
            // back to the half-form (like the 15-gon/17-gon) rather than the 18-gon's integer
            // coefficients: solving 17k^2-15k-2n=0 gives k=(15+sqrt(136n+225))/34, so n is
            // enneadecagonal iff 136n+225 is a perfect square AND its root == -15 (mod 34); then
            // k=(root+15)/34. The root-gate is what rejects n=16 (136n+225=2401=49^2 is a perfect
            // square, but 49+15=64 is not a multiple of 34).
            // Extends the figurate MEMBERSHIP family (triangular..octadecagonal) to 19 sides.
            // u128 avoids overflow on the discriminant.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 136u128 * (n as u128) + 225;
                    let s = isqrt(m);
                    if s * s == m && (s + 15) % 34 == 0 {
                        let k = ((s + 15) / 34) as u64;
                        crate::kprintln!(
                            "enneadecagonal: {} IS enneadecagonal (Ennea19({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("enneadecagonal: {} is NOT enneadecagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: enneadecagonal <n>"),
            }
        }
        "icosagonal" => {
            // Classify whether n is an ICOSAGONAL (20-gonal) number Icosa(k)=k(9k-8)
            // (k>=1: 1,20,57,112,185,276,385,...), reporting the index k. 20 sides is EVEN, so -- like
            // the 16-gon/18-gon and unlike the 19-gon's k(17k-15)/2 half-form -- the coefficients stay
            // integral: solving 9k^2-8k-n=0 gives k=(4+sqrt(9n+16))/9, so n is icosagonal iff 9n+16 is
            // a perfect square AND its root == -4 (mod 9); then k=(root+4)/9. The root-gate is what
            // rejects n=17 (9n+16=169=13^2 is a perfect square, but 13+4=17 is not a multiple of 9).
            // Extends the figurate MEMBERSHIP family (triangular..enneadecagonal) to 20 sides.
            // u128 avoids overflow on the discriminant.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 9u128 * (n as u128) + 16;
                    let s = isqrt(m);
                    if s * s == m && (s + 4) % 9 == 0 {
                        let k = ((s + 4) / 9) as u64;
                        crate::kprintln!(
                            "icosagonal: {} IS icosagonal (Icosa({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("icosagonal: {} is NOT icosagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: icosagonal <n>"),
            }
        }
        "henicosagonal" => {
            // Classify whether n is a HENICOSAGONAL (21-gonal) number Heni21(k)=k(19k-17)/2
            // (k>=1: 1,21,60,118,195,291,406,...), reporting the index k. 21 sides is ODD, so -- like
            // the 19-gon and unlike the 20-gon's integral k(9k-8) -- we are back to the half-form:
            // solving 19k^2-17k-2n=0 gives k=(17+sqrt(152n+289))/38, so n is henicosagonal iff
            // 152n+289 is a perfect square AND its root == -17 (mod 38); then k=(root+17)/38. The
            // root-gate is what rejects n=18 (152n+289=3025=55^2 is a perfect square, but 55+17=72
            // is not a multiple of 38).
            // Extends the figurate MEMBERSHIP family (triangular..icosagonal) to 21 sides.
            // u128 avoids overflow on the discriminant.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let m = 152u128 * (n as u128) + 289;
                    let s = isqrt(m);
                    if s * s == m && (s + 17) % 38 == 0 {
                        let k = ((s + 17) / 38) as u64;
                        crate::kprintln!(
                            "henicosagonal: {} IS henicosagonal (Heni21({}) = {})",
                            n, k, n
                        );
                    } else {
                        crate::kprintln!("henicosagonal: {} is NOT henicosagonal", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: henicosagonal <n>"),
            }
        }
        "nextdodecagonal" => {
            // The SMALLEST dodecagonal number Dod(k)=k(5k-4) (k>=1: 1,12,33,64,105,156,...) that is
            // >= n, reporting the index k -- the 12-gonal member of the nextsquare/.../nexthendecagonal
            // family, pairing with the dodecagonal MEMBERSHIP test. nextdodecagonal(2)=Dod(2)=12,
            // nextdodecagonal(12)=Dod(2)=12 (already dodecagonal), nextdodecagonal(13)=Dod(3)=33,
            // nextdodecagonal(0)=Dod(1)=1, nextdodecagonal(70)=Dod(5)=105. Estimates k from
            // k=(2+sqrt(5n+4))/5 then adjusts by one either way. u128 avoids overflow.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn dodec(k: u128) -> u128 {
                k * (5 * k - 4)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (2 + isqrt(5 * nn + 4)) / 5;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Dod(k) >= n.
                    while dodec(k) < nn {
                        k += 1;
                    }
                    while k > 1 && dodec(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextdodecagonal: smallest dodecagonal >= {} is Dod({}) = {}",
                        n, k as u64, dodec(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextdodecagonal <n>"),
            }
        }
        "nexthendecagonal" => {
            // The SMALLEST hendecagonal number Hen(k)=k(9k-7)/2 (k>=1: 1,11,30,58,95,141,...) that
            // is >= n, reporting the index k -- the 11-gonal member of the nextsquare/.../nextdecagonal
            // family, pairing with the hendecagonal MEMBERSHIP test. nexthendecagonal(2)=Hen(2)=11,
            // nexthendecagonal(11)=Hen(2)=11 (already hendecagonal), nexthendecagonal(12)=Hen(3)=30,
            // nexthendecagonal(0)=Hen(1)=1, nexthendecagonal(60)=Hen(5)=95. Estimates k from
            // k=(7+sqrt(72n+49))/18 then adjusts by one either way. u128 avoids overflow.
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hendec(k: u128) -> u128 {
                k * (9 * k - 7) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (7 + isqrt(72 * nn + 49)) / 18;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Hen(k) >= n.
                    while hendec(k) < nn {
                        k += 1;
                    }
                    while k > 1 && hendec(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nexthendecagonal: smallest hendecagonal >= {} is Hen({}) = {}",
                        n, k as u64, hendec(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nexthendecagonal <n>"),
            }
        }
        "nextdecagonal" => {
            // The SMALLEST decagonal number Dec(k)=k(4k-3) (k>=1: 1,10,27,52,85,126,...) that is
            // >= n, reporting the index k -- the decagonal member of the nextsquare/.../nextnonagonal
            // family. nextdecagonal(2)=Dec(2)=10, nextdecagonal(10)=Dec(2)=10 (already decagonal),
            // nextdecagonal(11)=Dec(3)=27, nextdecagonal(0)=Dec(1)=1, nextdecagonal(60)=Dec(5)=85.
            // Estimates k from k=(3+sqrt(16n+9))/8 then adjusts by one either way. Distinct from
            // decagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn deca(k: u128) -> u128 {
                k * (4 * k - 3)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let nn = n as u128;
                    let mut k = (3 + isqrt(16 * nn + 9)) / 8;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the smallest k>=1 with Dec(k) >= n.
                    while deca(k) < nn {
                        k += 1;
                    }
                    while k > 1 && deca(k - 1) >= nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "nextdecagonal: smallest decagonal >= {} is Dec({}) = {}",
                        n, k as u64, deca(k) as u64
                    );
                }
                Err(_) => crate::kprintln!("usage: nextdecagonal <n>"),
            }
        }
        "prevhendecagonal" => {
            // The LARGEST hendecagonal number Hen(k)=k(9k-7)/2 (k>=1: 1,11,30,58,95,141,...) that is
            // <= n, reporting the index k -- the mirror of nexthendecagonal (which rounds UP),
            // completing the 11-gonal pair. prevhendecagonal(12)=Hen(2)=11, prevhendecagonal(11)=
            // Hen(2)=11 (already hendecagonal), prevhendecagonal(30)=Hen(3)=30, prevhendecagonal(60)=
            // Hen(4)=58, prevhendecagonal(1)=Hen(1)=1. With nexthendecagonal it brackets n. n<1 has no
            // hendecagonal (k>=1), reported as "(none)". Distinct from hendecagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn hendec(k: u128) -> u128 {
                k * (9 * k - 7) / 2
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (7 + isqrt(72 * nn + 49)) / 18;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Hen(k) <= n.
                    while hendec(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && hendec(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevhendecagonal: largest hendecagonal <= {} is Hen({}) = {}",
                        n, k as u64, hendec(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevhendecagonal: no hendecagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevhendecagonal <n>"),
            }
        }
        "prevdodecagonal" => {
            // The LARGEST dodecagonal number Dod(k)=k(5k-4) (k>=1: 1,12,33,64,105,156,...) that is
            // <= n, reporting the index k -- the mirror of nextdodecagonal (which rounds UP),
            // completing the 12-gonal pair. prevdodecagonal(13)=Dod(2)=12, prevdodecagonal(12)=
            // Dod(2)=12 (already dodecagonal), prevdodecagonal(33)=Dod(3)=33, prevdodecagonal(100)=
            // Dod(4)=64, prevdodecagonal(1)=Dod(1)=1. With nextdodecagonal it brackets n. n<1 has no
            // dodecagonal (k>=1), reported as "(none)". Distinct from dodecagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn dodec(k: u128) -> u128 {
                k * (5 * k - 4)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (2 + isqrt(5 * nn + 4)) / 5;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Dod(k) <= n.
                    while dodec(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && dodec(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevdodecagonal: largest dodecagonal <= {} is Dod({}) = {}",
                        n, k as u64, dodec(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevdodecagonal: no dodecagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevdodecagonal <n>"),
            }
        }
        "prevdecagonal" => {
            // The LARGEST decagonal number Dec(k)=k(4k-3) (k>=1: 1,10,27,52,85,126,...) that is
            // <= n, reporting the index k -- the mirror of nextdecagonal (which rounds UP), completing
            // the decagonal pair just as prevnonagonal mirrors nextnonagonal. prevdecagonal(11)=
            // Dec(2)=10, prevdecagonal(10)=Dec(2)=10 (already decagonal), prevdecagonal(2)=Dec(1)=1,
            // prevdecagonal(1)=Dec(1)=1, prevdecagonal(50)=Dec(3)=27, prevdecagonal(85)=Dec(5)=85.
            // With nextdecagonal it brackets n. n<1 has no decagonal (k>=1), reported as "(none)".
            // Distinct from decagonal (a MEMBERSHIP test).
            fn isqrt(v: u128) -> u128 {
                if v < 2 {
                    return v;
                }
                let mut x = v;
                let mut y = (x + 1) / 2;
                while y < x {
                    x = y;
                    y = (x + v / x) / 2;
                }
                x
            }
            fn deca(k: u128) -> u128 {
                k * (4 * k - 3)
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n >= 1 => {
                    let nn = n as u128;
                    let mut k = (3 + isqrt(16 * nn + 9)) / 8;
                    if k < 1 {
                        k = 1;
                    }
                    // Adjust to the largest k>=1 with Dec(k) <= n.
                    while deca(k + 1) <= nn {
                        k += 1;
                    }
                    while k > 1 && deca(k) > nn {
                        k -= 1;
                    }
                    crate::kprintln!(
                        "prevdecagonal: largest decagonal <= {} is Dec({}) = {}",
                        n, k as u64, deca(k) as u64
                    );
                }
                Ok(_) => crate::kprintln!("prevdecagonal: no decagonal <= 0 (none)"),
                Err(_) => crate::kprintln!("usage: prevdecagonal <n>"),
            }
        }
        "zeckendorf" => cmd_zeckendorf(line, cmd, rest),
        "isfib" => {
            // Classify whether n is a FIBONACCI number, reporting its index k.
            // Fibonacci: F(0)=0,F(1)=1,F(2)=1,F(3)=2,F(4)=3,F(5)=5,F(6)=8,... A
            // number is Fibonacci iff it appears in the sequence. Distinct from
            // fib (which COMPUTES the nth Fibonacci) -- isfib tests membership.
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let target = n as u128;
                    let mut a: u128 = 0; // F(k)
                    let mut b: u128 = 1; // F(k+1)
                    let mut k: u64 = 0;
                    while a < target {
                        let t = a + b;
                        a = b;
                        b = t;
                        k += 1;
                    }
                    if a == target {
                        crate::kprintln!("isfib: {} IS a Fibonacci number (F({}) = {})", n, k, n);
                    } else {
                        crate::kprintln!("isfib: {} is NOT a Fibonacci number", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: isfib <n>"),
            }
        }
        "iskeith" => cmd_iskeith(line, cmd, rest),
        "histogram" => cmd_histogram(line, cmd, rest),
        "letterfreq" => cmd_letterfreq(line, cmd, rest),
        "wordfreq" => cmd_wordfreq(line, cmd, rest),
        "rarewordf" => cmd_rarewordf(line, cmd, rest),
        "topword" => cmd_topword(line, cmd, rest),
        "rareword" => cmd_rareword(line, cmd, rest),
        "topchar" => cmd_topchar(line, cmd, rest),
        "rarechar" => cmd_rarechar(line, cmd, rest),
        "duplicatewords" => cmd_duplicatewords(line, cmd, rest),
        "uniqwords" => cmd_uniqwords(line, cmd, rest),
        "rtcinfo" => cmd_rtcinfo(line, cmd, rest),
        "title" => cmd_title(line, cmd, rest),
        "titlef" => cmd_titlef(line, cmd, rest),
        "altwords" => cmd_altwords(line, cmd, rest),
        "isprime" => {
            // Primality predicate by trial division (overflow-safe d <= n/d bound).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // factor = smallest divisor found (0 = none, i.e. prime or n<2).
                    let (prime, factor) = if n < 2 {
                        (false, 0)
                    } else if n < 4 {
                        (true, 0)
                    } else if n % 2 == 0 {
                        (false, 2)
                    } else {
                        let mut d = 3u64;
                        let mut f = 0u64;
                        while d <= n / d {
                            if n % d == 0 {
                                f = d;
                                break;
                            }
                            d += 2;
                        }
                        (f == 0, f)
                    };
                    if prime {
                        crate::kprintln!("isprime: {} is prime", n);
                    } else if factor != 0 {
                        crate::kprintln!("isprime: {} is not prime (divisible by {})", n, factor);
                    } else {
                        crate::kprintln!("isprime: {} is not prime", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: isprime <n>"),
            }
        }
        "ispalprime" => {
            // Classify whether n is a PALINDROMIC PRIME: prime AND a decimal
            // palindrome (2, 3, 5, 7, 11, 101, 131, 151, 313 IS; 13 is prime but
            // 31 != 13 NOT; 22 is a palindrome but not prime NOT). Combines a
            // trial-division primality test with a digit-reverse palindrome check
            // (u128 reverse, no overflow). Distinct from isprime (primality only)
            // and revnum (reverse + palindrome only).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    // Primality by trial division (overflow-safe d <= n/d bound).
                    let prime = if n < 2 {
                        false
                    } else if n < 4 {
                        true
                    } else if n % 2 == 0 {
                        false
                    } else {
                        let mut d = 3u64;
                        let mut p = true;
                        while d <= n / d {
                            if n % d == 0 {
                                p = false;
                                break;
                            }
                            d += 2;
                        }
                        p
                    };
                    // Decimal palindrome: reverse the digits in u128, compare to n.
                    let mut r: u128 = 0;
                    let mut m = n;
                    while m > 0 {
                        r = r * 10 + (m % 10) as u128;
                        m /= 10;
                    }
                    let palindrome = r == n as u128;
                    if prime && palindrome {
                        crate::kprintln!("ispalprime: {} IS a palindromic prime", n);
                    } else if !prime {
                        crate::kprintln!("ispalprime: {} is NOT (not prime)", n);
                    } else {
                        crate::kprintln!("ispalprime: {} is NOT (prime but not a palindrome)", n);
                    }
                }
                Err(_) => crate::kprintln!("usage: ispalprime <n>"),
            }
        }
        "emirp" => {
            // Classify whether n is an EMIRP: a prime whose decimal digit-reversal
            // is a DIFFERENT prime (13<->31, 17<->71, 79<->97). Palindromic primes
            // (11, 101, 131) are NOT emirps because the reversal equals n, and the
            // single-digit primes (2, 3, 5, 7) reverse to themselves. Distinct from
            // ispalprime (a prime that reads the SAME reversed) and revnum (reverse
            // + palindrome check WITHOUT primality). Primality by trial division with
            // the overflow-safe d <= n/d bound; the reverse fits u64 for n <= 1e7.
            fn is_prime(x: u64) -> bool {
                if x < 2 {
                    return false;
                }
                if x < 4 {
                    return true;
                }
                if x % 2 == 0 {
                    return false;
                }
                let mut d = 3u64;
                while d <= x / d {
                    if x % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n <= 10_000_000 => {
                    let mut r = 0u64;
                    let mut m = n;
                    while m > 0 {
                        r = r * 10 + m % 10;
                        m /= 10;
                    }
                    if is_prime(n) && r != n && is_prime(r) {
                        crate::kprintln!(
                            "emirp: {} IS an emirp (reverse {} is a distinct prime)",
                            n,
                            r
                        );
                    } else if is_prime(n) && r == n {
                        crate::kprintln!("emirp: {} is NOT an emirp (palindromic prime)", n);
                    } else {
                        crate::kprintln!("emirp: {} is NOT an emirp", n);
                    }
                }
                Ok(_) => crate::kprintln!("emirp: n too large (max 10000000)"),
                Err(_) => crate::kprintln!("usage: emirp <n>"),
            }
        }
        "iscircularprime" => cmd_iscircularprime(line, cmd, rest),
        "istwinprime" => cmd_istwinprime(line, cmd, rest),
        "iscousinprime" => cmd_iscousinprime(line, cmd, rest),
        "issexyprime" => cmd_issexyprime(line, cmd, rest),
        "issafeprime" => {
            // Classify whether n is a SAFE PRIME: n is prime AND (n-1)/2 is also prime.
            // 5 -> (5-1)/2 = 2 prime, so IS; 7->3, 11->5, 23->11 are all safe primes.
            // 13 is NOT ((13-1)/2 = 6 is composite). The matching (n-1)/2 is a Sophie
            // Germain prime. Distinct from istwinprime (n+-2 prime), emirp, and
            // iscircularprime. Trial division with the overflow-safe d <= n/d bound; n
            // capped at 1e12. The short-circuit on is_prime(n) avoids an (n-1) underflow.
            fn is_prime(x: u64) -> bool {
                if x < 2 {
                    return false;
                }
                if x < 4 {
                    return true;
                }
                if x % 2 == 0 {
                    return false;
                }
                let mut d = 3u64;
                while d <= x / d {
                    if x % d == 0 {
                        return false;
                    }
                    d += 2;
                }
                true
            }
            match rest.trim().parse::<u64>() {
                Ok(n) if n <= 1_000_000_000_000 => {
                    if is_prime(n) && is_prime((n - 1) / 2) {
                        crate::kprintln!(
                            "issafeprime: {} IS a safe prime ((n-1)/2 = {} is prime)",
                            n,
                            (n - 1) / 2
                        );
                    } else {
                        crate::kprintln!("issafeprime: {} is NOT a safe prime", n);
                    }
                }
                Ok(_) => crate::kprintln!("issafeprime: n too large (max 1000000000000)"),
                Err(_) => crate::kprintln!("usage: issafeprime <n>"),
            }
        }
        "mairinfo" => {
            // Decode MAIR_EL1: 8 one-byte memory-attribute encodings (Attr0..7).
            fn attr_name(b: u8) -> &'static str {
                match b {
                    0x00 => "Device-nGnRnE",
                    0x04 => "Device-nGnRE",
                    0x08 => "Device-nGRE",
                    0x0c => "Device-GRE",
                    0x44 => "Normal-NonCacheable",
                    0xff => "Normal-WriteBack-RW-Alloc",
                    _ if b & 0xf0 == 0 => "Device (other)",
                    _ => "Normal (other)",
                }
            }
            let mair: u64;
            // SAFETY: MAIR_EL1 is readable at EL1.
            unsafe { #[cfg(target_arch = "aarch64")]
                core::arch::asm!("mrs {}, mair_el1", out(reg) mair); #[cfg(not(target_arch = "aarch64"))] { mair = crate::shell_pc::sysreg("mair_el1"); }
            }
            crate::kprintln!("mairinfo: MAIR_EL1 = 0x{:016x}", mair);
            for i in 0..8 {
                let b = ((mair >> (i * 8)) & 0xff) as u8;
                if b != 0 || i == 0 {
                    crate::kprintln!("mairinfo: Attr{} = 0x{:02x} ({})", i, b, attr_name(b));
                }
            }
        }
        "tail" => cmd_tail(line, cmd, rest),
        "ncr" => cmd_ncr(line, cmd, rest),
        "npr" => cmd_npr(line, cmd, rest),
        "pochhammer" => cmd_pochhammer(line, cmd, rest),
        "head" => cmd_head(line, cmd, rest),
        "headlines" => cmd_headlines(line, cmd, rest),
        "rangelines" => cmd_rangelines(line, cmd, rest),
        "rangewords" => cmd_rangewords(line, cmd, rest),
        "unwrap" => cmd_unwrap(line, cmd, rest),
        "joinlines" => cmd_joinlines(line, cmd, rest),
        "taillines" => cmd_taillines(line, cmd, rest),
        "tailwords" => cmd_tailwords(line, cmd, rest),
        "headwords" => cmd_headwords(line, cmd, rest),
        "oddlines" => cmd_oddlines(line, cmd, rest),
        "evenlines" => cmd_evenlines(line, cmd, rest),
        "rwords" => cmd_rwords(line, cmd, rest),
        "rotwords" => cmd_rotwords(line, cmd, rest),
        "capitalizeword" => cmd_capitalizeword(line, cmd, rest),
        "lowerword" => cmd_lowerword(line, cmd, rest),
        "upperword" => cmd_upperword(line, cmd, rest),
        "reverseword" => cmd_reverseword(line, cmd, rest),
        "reveach" => cmd_reveach(line, cmd, rest),
        "revmiddle" => cmd_revmiddle(line, cmd, rest),
        "sortwordchars" => cmd_sortwordchars(line, cmd, rest),
        "swappairs" => cmd_swappairs(line, cmd, rest),
        "deleteword" => cmd_deleteword(line, cmd, rest),
        "setword" => cmd_setword(line, cmd, rest),
        "insertword" => cmd_insertword(line, cmd, rest),
        "swapchar" => cmd_swapchar(line, cmd, rest),
        "swapwords" => cmd_swapwords(line, cmd, rest),
        "swapwordpairs" => cmd_swapwordpairs(line, cmd, rest),
        "swapends" => cmd_swapends(line, cmd, rest),
        "swapfirstlast" => cmd_swapfirstlast(line, cmd, rest),
        "rotate" => cmd_rotate(line, cmd, rest),
        "longestword" => cmd_longestword(line, cmd, rest),
        "maxword" => cmd_maxword(line, cmd, rest),
        "shortestword" => cmd_shortestword(line, cmd, rest),
        "minword" => cmd_minword(line, cmd, rest),
        "sumto" => {
            // nth triangular number T(n) = n(n+1)/2 (u128 avoids overflow).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let t: u128 = (n as u128) * (n as u128 + 1) / 2;
                    crate::kprintln!("sumto: 1+2+...+{} = {}", n, t);
                }
                Err(_) => crate::kprintln!("usage: sumto <n>"),
            }
        }
        "sumrange" => cmd_sumrange(line, cmd, rest),
        "sumsq" => {
            // Sum of the first n squares = n(n+1)(2n+1)/6 (u128, overflow-guarded).
            // Distinct from sumto (the linear 1+2+...+n triangular sum).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let n = n as u128;
                    let s = n
                        .checked_mul(n + 1)
                        .and_then(|x| x.checked_mul(2 * n + 1))
                        .map(|x| x / 6);
                    match s {
                        Some(sum) => {
                            crate::kprintln!("sumsq: sum of first {} squares = {}", n, sum)
                        }
                        None => crate::kprintln!("sumsq: overflows u128 (n too large)"),
                    }
                }
                Err(_) => crate::kprintln!("usage: sumsq <n>"),
            }
        }
        "sumcubes" => {
            // Sum of the first n cubes = (n(n+1)/2)^2 = T(n)^2 (the square of the
            // nth triangular number), u128 overflow-guarded. Completes the power-sum
            // trio with sumto (linear) and sumsq (squares).
            match rest.trim().parse::<u64>() {
                Ok(n) => {
                    let n = n as u128;
                    let t = n.checked_mul(n + 1).map(|x| x / 2); // nth triangular number
                    let s = t.and_then(|tt| tt.checked_mul(tt)); // T(n)^2
                    match s {
                        Some(sum) => {
                            crate::kprintln!("sumcubes: sum of first {} cubes = {}", n, sum)
                        }
                        None => crate::kprintln!("sumcubes: overflows u128 (n too large)"),
                    }
                }
                Err(_) => crate::kprintln!("usage: sumcubes <n>"),
            }
        }
        "mean" => cmd_mean(line, cmd, rest),
        "clamp" => cmd_clamp(line, cmd, rest),
        "minmax" => cmd_minmax(line, cmd, rest),
        "wrap" => cmd_wrap(line, cmd, rest),
        "center" => cmd_center(line, cmd, rest),
        "indent" => cmd_indent(line, cmd, rest),
        "rjust" => cmd_rjust(line, cmd, rest),
        "ljust" => cmd_ljust(line, cmd, rest),
        "zfill" => cmd_zfill(line, cmd, rest),
        "df" => cmd_df(line, cmd, rest),
        "reboot" => {
            // Clean machine reset via the ARM PSCI firmware interface.
            crate::kprintln!("[reboot] resetting via PSCI SYSTEM_RESET...");
            crate::psci::reboot();
        }
        "poweroff" => {
            // Clean shutdown via PSCI SYSTEM_OFF (QEMU exits).
            crate::kprintln!("[poweroff] halting via PSCI SYSTEM_OFF...");
            crate::psci::poweroff();
        }
        "gpio" => cmd_gpio(line, cmd, rest),
        "stat" => cmd_stat(line, cmd, rest),
        "date" => {
            // Read the PL031 RTC (seconds since the Unix epoch) and format it as
            // civil UTC time. On QEMU virt the RTC tracks the host wall clock.
            let secs =
                unsafe { core::ptr::read_volatile(crate::board::RTC_BASE as *const u32) } as u64;
            let (y, mo, d, hh, mm, ss) = epoch_to_utc(secs);
            crate::kprintln!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC  (epoch {})",
                y, mo, d, hh, mm, ss, secs
            );
        }
        "rtc" => {
            // Userspace driver: an EL0 process reads the PL031 clock through its
            // granted MMIO capability and serves the time to a client over IPC.
            crate::kprintln!("[rtc] launching userspace RTC driver + client (see [P3]/[P4])...");
            crate::process::run_rtc_demo();
            crate::kprintln!("[rtc] done: a sandboxed EL0 driver read the clock and served it over IPC");
        }
        "rustapp" => {
            // Run a real compiled-Rust program at EL0 (not an asm stub).
            crate::kprintln!("[rustapp] running a compiled-Rust EL0 app (see [P3])...");
            crate::process::run_rustapp();
            crate::kprintln!("[rustapp] done: ordinary Rust executed at EL0 via syscalls");
        }
        "niavm" => {
            // Run the Nia mini WASM interpreter as an EL0 process; it executes a
            // WASM module at EL0 and its host print becomes a syscall.
            crate::kprintln!("[niavm] Nia interpreter at EL0 runs WASM using print+putc+time via syscalls (see [P3])...");
            crate::process::run_niavm();
            crate::kprintln!("[niavm] done: the WASM runtime ran as a userspace process, host calls = syscalls");
        }
        "appget" => {
            // Install a Nia app from the network: appget <ip> <port> <name> <sha256> (appget.rs).
            crate::appget::run(rest, cpu_getc);
        }
        "dira" => crate::dira::run(rest), // the web browser: dira <address> (dira/mod.rs)
        "nia" => {
            // Nia WASM runtime. `nia` runs the built-in demos; `nia <file.wasm>`
            // loads and runs a WebAssembly module from the filesystem.
            if rest.is_empty() {
                crate::nia::run_demo();
            } else if rest == "--stackprobe" {
                crate::nia::stack_probe_on(); // a test switch (desk-cell-app), until the next boot
            } else {
                crate::nia::run_file(rest);
            }
        }
        "store" => {
            // AfriStore: interactive graphical catalog — press a number to launch.
            crate::store::open();
            crate::kprintln!("[store] closed.");
        }
        "ls" => cmd_ls(line, cmd, rest),
        "cat" => match vfs::read_str(rest) {
            Some(s) => crate::kprintln!("{}", s),
            None => crate::kprintln!("cat: no such file: {}", rest),
        },
        "hmac" => cmd_hmac(line, cmd, rest),
        "crc32" => {
            // CRC-32 (IEEE) checksum of a file.
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: crc32 <file>");
            } else {
                match vfs::read_raw(name) {
                    Some(d) => crate::kprintln!("crc32 {}: {:#010x}", name, crc32(d)),
                    None => crate::kprintln!("crc32: no such file: {}", name),
                }
            }
        }
        "fletcher16" => {
            // Fletcher-16 checksum of a VFS file (distinct from crc32's IEEE CRC-32).
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: fletcher16 <file>");
            } else {
                match vfs::read_raw(name) {
                    None => crate::kprintln!("fletcher16: {} not found", name),
                    Some(bytes) => {
                        let mut s1: u32 = 0;
                        let mut s2: u32 = 0;
                        for &b in bytes {
                            s1 = (s1 + b as u32) % 255;
                            s2 = (s2 + s1) % 255;
                        }
                        let sum = (s2 << 8) | s1;
                        crate::kprintln!(
                            "fletcher16: {} = 0x{:04x} ({} bytes)",
                            name,
                            sum,
                            bytes.len()
                        );
                    }
                }
            }
        }
        "append" => cmd_append(line, cmd, rest),
        "wc" => cmd_wc(line, cmd, rest),
        "base64" => cmd_base64(line, cmd, rest),
        "base32" => cmd_base32(line, cmd, rest),
        "find" => {
            // List filenames containing a substring (complements grep's content search).
            let pat = rest.trim();
            if pat.is_empty() {
                crate::kprintln!("usage: find <substr>");
            } else {
                let mut matches = 0;
                for i in 0..vfs::MAX_FILES {
                    if vfs::is_used(i) {
                        let name = vfs::name_of(i);
                        if name.contains(pat) {
                            crate::kprintln!("{:>6}  {}", vfs::size_of(i), name);
                            matches += 1;
                        }
                    }
                }
                if matches == 0 {
                    crate::kprintln!("find: no files matching '{}'", pat);
                }
            }
        }
        "grep" => cmd_grep(line, cmd, rest),
        "grepi" => cmd_grepi(line, cmd, rest),
        "grepw" => cmd_grepw(line, cmd, rest),
        "grepv" => cmd_grepv(line, cmd, rest),
        "grepn" => cmd_grepn(line, cmd, rest),
        "grepc" => cmd_grepc(line, cmd, rest),
        "scrabblef" => cmd_scrabblef(line, cmd, rest),
        "charcount" => cmd_charcount(line, cmd, rest),
        "wordstartcount" => cmd_wordstartcount(line, cmd, rest),
        "wordendcount" => cmd_wordendcount(line, cmd, rest),
        "countchar" => cmd_countchar(line, cmd, rest),
        "countword" => cmd_countword(line, cmd, rest),
        "indexof" => {
            // Print the 1-based BYTE position of the FIRST occurrence of <sub> in the inline
            // text, or "not found" if absent. "indexof lo hello world" -> 4, "indexof world
            // hello world" -> 7, "indexof z hello world" -> not found. The locator companion
            // of countsubstr (which COUNTS non-overlapping occurrences); distinct from charat
            // (the char AT a position) and find/grep (which search FILES/LINES).
            let (needle, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if needle.is_empty() || text.is_empty() {
                crate::kprintln!("usage: indexof <sub> <text>");
            } else {
                match text.find(needle) {
                    Some(i) => crate::kprintln!("indexof: {}", i + 1),
                    None => crate::kprintln!("indexof: not found"),
                }
            }
        }
        "lastindexof" => {
            // Print the 1-based BYTE position of the LAST occurrence of <sub> in the inline
            // text, or "not found" -- the mirror of indexof (which finds the FIRST). "lastindexof
            // o hello world" -> 8, "lastindexof l hello" -> 4, "lastindexof z hello" -> not found.
            // Distinct from indexof (first occurrence) and countsubstr (count).
            let (needle, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if needle.is_empty() || text.is_empty() {
                crate::kprintln!("usage: lastindexof <sub> <text>");
            } else {
                match text.rfind(needle) {
                    Some(i) => crate::kprintln!("lastindexof: {}", i + 1),
                    None => crate::kprintln!("lastindexof: not found"),
                }
            }
        }
        "before" => {
            // Print everything BEFORE the first occurrence of <sep> in the text (the head of a
            // str.partition); if <sep> is absent the whole text is printed. "before = key=value"
            // -> "key", "before : a:b:c" -> "a", "before o hello" -> "hell". The <sep> is the
            // first token and the haystack is the remainder. Distinct from indexof (POSITION),
            // substr (position+length), cut (field N of a line), and stripprefix (anchored trim).
            let (sep, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if sep.is_empty() || text.is_empty() {
                crate::kprintln!("usage: before <sep> <text>");
            } else {
                let out = match text.find(sep) {
                    Some(i) => &text[..i],
                    None => text,
                };
                crate::kprintln!("before: {}", out);
            }
        }
        "after" => {
            // Print everything AFTER the first occurrence of <sep> in the text (the tail of a
            // str.partition) -- the mirror of `before`; if <sep> is absent the result is empty.
            // "after = key=value" -> "value", "after : a:b:c" -> "b:c" (only the FIRST sep
            // splits), "after - 2024-01-15" -> "01-15". The <sep> is the first token and the
            // haystack is the remainder. Distinct from indexof/substr/cut and stripsuffix.
            let (sep, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if sep.is_empty() || text.is_empty() {
                crate::kprintln!("usage: after <sep> <text>");
            } else {
                let out = match text.find(sep) {
                    Some(i) => &text[i + sep.len()..],
                    None => "",
                };
                crate::kprintln!("after: {}", out);
            }
        }
        "afterlast" => {
            // Print everything AFTER the LAST occurrence of <sep> in the text (the tail of an
            // rpartition) -- the rfind-based mirror of `after` (which uses the FIRST sep). Handy
            // for extracting a file extension: "afterlast . file.tar.gz" -> "gz", "afterlast /
            // /a/b/c" -> "c". An absent separator yields the empty string. Distinct from after
            // (splits on the FIRST sep) and nthfield (an indexed field).
            let (sep, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if sep.is_empty() || text.is_empty() {
                crate::kprintln!("usage: afterlast <sep> <text>");
            } else {
                let out = match text.rfind(sep) {
                    Some(i) => &text[i + sep.len()..],
                    None => "",
                };
                crate::kprintln!("afterlast: {}", out);
            }
        }
        "beforelast" => {
            // Print everything BEFORE the LAST occurrence of <sep> in the text (the head of an
            // rpartition) -- the rfind-based mirror of `before` (which uses the FIRST sep); if
            // <sep> is absent the whole text is printed. "beforelast . file.tar.gz" -> "file.tar"
            // (strip a file extension), "beforelast / /a/b/c" -> "/a/b". With afterlast it splits
            // on the LAST separator. Distinct from before (FIRST sep) and afterlast (the TAIL).
            let (sep, text) = match rest.find(' ') {
                Some(i) => (&rest[..i], &rest[i + 1..]),
                None => (rest, ""),
            };
            if sep.is_empty() || text.is_empty() {
                crate::kprintln!("usage: beforelast <sep> <text>");
            } else {
                let out = match text.rfind(sep) {
                    Some(i) => &text[..i],
                    None => text,
                };
                crate::kprintln!("beforelast: {}", out);
            }
        }
        "between" => cmd_between(line, cmd, rest),
        "countsubstr" => cmd_countsubstr(line, cmd, rest),
        "wordcountf" => cmd_wordcountf(line, cmd, rest),
        "countdistinct" => cmd_countdistinct(line, cmd, rest),
        "sortf" => cmd_sortf(line, cmd, rest),
        "hexdump" => cmd_hexdump(line, cmd, rest),
        "hash" => cmd_hash(line, cmd, rest),
        "write" => {
            let (name, content) = match rest.find(' ') {
                Some(i) => (&rest[..i], rest[i + 1..].trim()),
                None => (rest, ""),
            };
            if name.is_empty() {
                crate::kprintln!("usage: write <file> <text>");
            } else if vfs::write(name, content.as_bytes()) {
                crate::kprintln!("wrote {} bytes to {}", content.len(), name);
            } else {
                crate::kprintln!("write: failed (name too long or filesystem full)");
            }
        }
        "touch" => {
            // Create an empty file, or re-stamp an existing file's mtime (2.102:
            // now stamps the RTC mtime, where before it only created the slot).
            let name = rest.trim();
            if name.is_empty() {
                crate::kprintln!("usage: touch <file>");
            } else if vfs::touch(name) {
                crate::kprintln!("touched {}", name);
            } else {
                crate::kprintln!("touch: failed (name too long or filesystem full)");
            }
        }
        "rm" => {
            if vfs::remove(rest) {
                crate::kprintln!("removed {}", rest);
            } else {
                crate::kprintln!("rm: no such file: {}", rest);
            }
        }
        "cp" => {
            let (src, dst) = match rest.find(' ') {
                Some(i) => (&rest[..i], rest[i + 1..].trim()),
                None => (rest, ""),
            };
            if src.is_empty() || dst.is_empty() {
                crate::kprintln!("usage: cp <src> <dst>");
            } else {
                match vfs::read_raw(src) {
                    None => crate::kprintln!("cp: no such file: {}", src),
                    Some(data) => {
                        let n = data.len();
                        if vfs::write(dst, data) {
                            crate::kprintln!("copied {} -> {} ({} bytes)", src, dst, n);
                        } else {
                            crate::kprintln!("cp: write failed (name too long or filesystem full)");
                        }
                    }
                }
            }
        }
        "mv" => {
            let (src, dst) = match rest.find(' ') {
                Some(i) => (&rest[..i], rest[i + 1..].trim()),
                None => (rest, ""),
            };
            if src.is_empty() || dst.is_empty() {
                crate::kprintln!("usage: mv <src> <dst>");
            } else {
                match vfs::read_raw(src) {
                    None => crate::kprintln!("mv: no such file: {}", src),
                    Some(data) => {
                        if vfs::write(dst, data) {
                            vfs::remove(src);
                            crate::kprintln!("moved {} -> {}", src, dst);
                        } else {
                            crate::kprintln!("mv: write failed (name too long or filesystem full)");
                        }
                    }
                }
            }
        }
        "devices" => {
            crate::kprintln!("NAME               CLASS      MMIO          STATUS");
            for i in 0..asili::count() {
                if let Some(d) = asili::get(i) {
                    crate::kprintln!(
                        "{:<18} {:<10} {:#012x}  {}",
                        d.name,
                        d.class.as_str(),
                        d.mmio,
                        if d.ok { "OK" } else { "FAIL" }
                    );
                }
            }
        }
        "about" => {
            crate::kprintln!("AfriSovera OS — a clean-room, capability-secured microkernel OS.");
            crate::kprintln!("Mansa kernel (Rust) | EL0 userspace | WASM-native | open source.");
            crate::kprintln!("You are typing into a kernel running on emulated ARM64 hardware.");
        }
        "heap" if rest == "grow" || rest.starts_with("grow ") => {
            // 2.740: the arena taking frames from `frame::`, live. Default 1536 KiB —
            // deliberately larger than the 1 MiB `.bss` reservation, so a served request
            // is growth by elimination rather than by taking the counters' word for it.
            let kib = rest[4..].trim().parse::<usize>().unwrap_or(1536);
            heap::grow_demo(kib);
        }
        "heap" if rest == "test" => {
            // The arena's refusal paths against synthetic requests (2.734). The live
            // kernel allocates a few KiB into a mebibyte, so every one of them is
            // otherwise code that has never run.
            heap::selftest();
        }
        "heap" => {
            // The third memory ceiling, reported where the other two are (2.734).
            heap::describe();
        }
        "mem" => cmd_mem(line, cmd, rest),
        "uptime" => {
            let t = sched::ticks();
            crate::kprintln!("{} ticks  (~{} ms, ~{} s)", t, t * 10, t / 100);
        }
        "sleep" => {
            // Block for a bounded number of milliseconds using the timer.
            let ms = rest.trim().parse::<u64>().unwrap_or(100).clamp(1, 5000);
            let ticks = crate::timer::sleep_ms(ms);
            crate::kprintln!("[sleep] slept {} ticks (~{} ms)", ticks, ticks * 10);
        }
        "rand" => {
            // Random u32 values from the hardware-seeded PRNG.
            let n = rest.trim().parse::<usize>().unwrap_or(1).clamp(1, 16);
            crate::kprint!("[rand]");
            for _ in 0..n {
                crate::kprint!(" {:08x}", crate::rng::next_u32());
            }
            crate::kprintln!();
        }
        "random" => {
            // Capability-gated RNG demo: the same WASM app with CAP_RANDOM
            // granted (it draws values) then denied (the kernel refuses).
            crate::nia::rand_demo();
        }
        #[cfg(not(feature = "pc"))]
        "mesh" => {
            // Sovereign Mesh Phase 0 (AFR-0009, kernel/src/mesh/): `mesh id`, `mesh forget`, `mesh selftest`,
            // `mesh listen [port] [secs]`, `mesh send <ip>[:port] <text>`. Port defaults to 4401.
            let r = rest.trim();
            if r == "id" {
                crate::mesh::show_id();
            } else if r == "forget" {
                if crate::mesh::forget() {
                    crate::kprintln!("[mesh] identity forgotten (secret key wiped); the next mesh step makes a new one");
                } else {
                    crate::kprintln!("[mesh] no identity to forget");
                }
            } else if r == "selftest" {
                crate::mesh::selftest();
            } else if r == "listen" || r.starts_with("listen ") {
                let mut it = r["listen".len()..].split_whitespace();
                let port = it.next().and_then(|p| p.parse::<u16>().ok()).filter(|&p| p != 0).unwrap_or(crate::mesh::PORT);
                let secs = it.next().and_then(|p| p.parse::<u64>().ok()).filter(|&t| t > 0 && t <= 120).unwrap_or(30);
                crate::mesh::listen(port, secs * 1000);
            } else if let Some(a) = r.strip_prefix("send ") {
                let (dest, text) = match a.find(' ') {
                    Some(i) => (a[..i].trim(), a[i + 1..].trim_start()),
                    None => (a.trim(), ""),
                };
                let target = match dest.rfind(':') {
                    Some(i) => parse_ipv4(&dest[..i]).zip(dest[i + 1..].parse::<u16>().ok().filter(|&p| p != 0)),
                    None => parse_ipv4(dest).map(|ip| (ip, crate::mesh::PORT)),
                };
                match target {
                    Some((ip, port)) if !text.is_empty() => {
                        crate::mesh::send(ip, port, text.as_bytes(), 20_000);
                    }
                    _ => crate::kprintln!("[mesh] usage: mesh send <a.b.c.d>[:port] <text>"),
                }
            } else {
                crate::kprintln!("[mesh] usage: mesh id | mesh forget | mesh selftest | mesh listen [port] [secs] | mesh send <ip>[:port] <text>");
            }
        }
        #[cfg(not(feature = "pc"))]
        "stack" => {
            // F7: `stack` = the boot and secondary stacks: size, deepest use since boot (paint high-water),
            // free. `stack guards` = the MMU's answer for each guard. `stack overflow-test` = deliberately
            // run the boot stack out (halts the machine when the guard works).
            match rest.trim() {
                "guards" => {
                    crate::stackwater::check_guards();
                }
                "overflow-test" => crate::stackwater::overflow_test(),
                _ => crate::stackwater::report(),
            }
        }
        "rng" => cmd_rng(line, cmd, rest),
        "entropy" => {
            // Read raw bytes from the virtio-rng hardware entropy device.
            if !crate::virtio::rng_present() {
                crate::kprintln!("[entropy] no virtio-rng device (boot with -device virtio-rng-device)");
            } else {
                let mut b = [0u8; 16];
                let n = crate::virtio::virtio_entropy(&mut b);
                if n == 0 {
                    crate::kprintln!("[entropy] virtio-rng read failed");
                } else {
                    const HEX: &[u8; 16] = b"0123456789abcdef";
                    let mut line = [0u8; 48];
                    let mut p = 0;
                    for &byte in b[..n].iter() {
                        line[p] = HEX[(byte >> 4) as usize];
                        line[p + 1] = HEX[(byte & 0xf) as usize];
                        line[p + 2] = b' ';
                        p += 3;
                    }
                    crate::kprintln!(
                        "[entropy] virtio-rng gave {} bytes: {}",
                        n,
                        core::str::from_utf8(&line[..p]).unwrap_or("?")
                    );
                }
            }
        }
        "dice" => {
            // A WASM dice app rolling 1..6 via the capability-gated RNG.
            let n = rest.trim().parse::<usize>().unwrap_or(3);
            crate::nia::dice_demo(n);
        }
        "wasmfuzz" => {
            // Feed malformed modules to the WASM parser; the kernel must survive.
            crate::nia::fuzz();
        }
        "wasmbench" => {
            // Time the Nia bench apps the old way and the new way (claude/nia-fast).
            crate::nia::bench();
        }
        "wasmops" => {
            // Run a WASM module exercising the shift/rotate opcodes.
            crate::nia::ops_demo();
        }
        "wasmi64" => {
            // Run a WASM module exercising 64-bit integer ops (i64.mul/add).
            crate::nia::i64_demo();
        }
        "wasmi64mem" => {
            // Run a WASM module round-tripping an i64 through linear memory.
            crate::nia::i64mem_demo();
        }
        "wasmoob" => {
            // A full-capability module calling speak with a pointer of -1: TRAP, never a panic (nia-addr).
            crate::nia::oob_demo();
        }
        "wasmi64loc" => {
            // Run a WASM module holding a >32-bit value in an i64 local.
            crate::nia::i64loc_demo();
        }
        "wasmf64" => {
            // Run a WASM module exercising IEEE-754 f64 arithmetic.
            crate::nia::f64_demo();
        }
        "wasmf32" => {
            // Run a WASM module exercising IEEE-754 f32 arithmetic.
            crate::nia::f32_demo();
        }
        "wasmmemsz" => {
            // Run a WASM module that reads its linear-memory size (memory.size).
            crate::nia::memsz_demo();
        }
        "wasmbrtable" => {
            // Run a WASM module exercising br_table (indexed multi-way branch).
            crate::nia::brtable_demo();
        }
        "wasmglob" => {
            // Run a WASM module with a mutable global (global.get/global.set).
            crate::nia::glob_demo();
        }
        "bench" => cmd_bench(line, cmd, rest),
        "irqstat" => {
            // One-line interrupt summary (timer ticks + PL011 RX interrupts +
            // console idle-wfi count, which proves the shell sleeps between keys).
            crate::kprintln!(
                "irqs: timer {} ticks, uart rx {}, idle-wfi {}, rx-drops {}, rx-overruns {}",
                sched::ticks(),
                crate::uart::rx_irq_count(),
                crate::uart::idle_wfi_count(),
                crate::uart::rx_drop_count(),
                crate::uart::rx_overrun_count()
            );
        }
        "pmap" if rest == "test" => {
            // The Normal window against every board in the table (2.733). This machine
            // boots with one board, so "the map follows each board's declared DRAM" is
            // otherwise arithmetic nobody ever runs.
            crate::paging::selftest();
        }
        "pmap" => cmd_pmap(line, cmd, rest),
        "sysinfo" => cmd_sysinfo(line, cmd, rest),
        "echo" => {
            crate::kprintln!("{}", rest);
        }
        "fbcon" if rest.trim() == "time" => {
            crate::framebuffer::con_time(); // afriui.md section 18
        }
        "fbcon" if rest.trim() == "blit" => {
            crate::framebuffer::blit_time(); // afriui.md section 19
        }
        "fbcon" if rest.trim() == "wm" => {
            crate::tonga::wm_time(); // afriui.md section 20
        }
        "clear" => {
            crate::desk::hide();
            crate::framebuffer::console_reset();
        }
        "caps" => {
            crate::kprintln!("Capabilities: authority is an unforgeable token the kernel grants.");
            crate::kprintln!("No root, no ambient power. An app can ONLY do what its caps allow.");
            crate::kprintln!("Phase 0 proved it: P1 held a SEND cap, P2 a RECV cap; an un-capable");
            crate::kprintln!("SEND was refused by the kernel. That is the no-gatekeeper foundation.");
        }
        _ => {
            crate::kprintln!("unknown command: '{}'  (try 'help')", cmd);
        }
    }
}
