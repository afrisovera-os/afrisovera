//! Wi-Fi as the PC's network card (W4b of the Wi-Fi plan, 14 Nov 2026): the station (wlan/mlme.rs) on a radio,
//! seen by the TCP/IP stack through nic.rs exactly like a wired card, and the commands
//! `wifi scan`, `wifi connect <name>`, `wifi status` and `wifi disconnect`.
//!
//! The radio is `crate::air`: pc/wifiradio.rs (the Intel 7265 driver goes there in W5) or, in the test image only,
//! the pretend radio of pc/wifisim.rs (pc/main.rs picks the file; nothing here depends on which). Without a radio
//! the commands say there is no Wi-Fi radio AfriSovera drives yet.
//!
//! The passphrase (security engineer's and architect's rules, 14 Nov 2026):
//! - asked for only after `wifi connect`, never taken on the command line, and typed only at this PC's own keyboard
//!   or serial line: a command run for the field report (the network) or inside `more` or `>` is refused;
//! - never echoed, never kept in the shell's history (it is read here, not by the line editor), never printed;
//! - wiped (volatile writes and a compiler fence, wlan/crypto.rs `wipe`) as soon as the key is made from it.
//! Keys are made only after the Wi-Fi crypto passed its published vectors in this boot (`wifi selftest`, run first
//! when it has not been).
//!
//! The station is shared with the timer interrupt (virtio_pci.rs `emu_poll` calls nic.rs). One lock (`BUSY`) guards
//! it: a command holds it with interrupts on, and the interrupt's poll skips a tick when it is taken. The interrupt
//! moves only a connected station's traffic; joining and the handshake run from the command, unmasked. Nothing slow
//! (making keys, DHCP) runs while the lock is held. `wifi status` prints the longest masked stretch (the architect's
//! 20 ms rule) and the longest step a command ran.

use crate::wlan::crypto;
use crate::wlan::frame::{Mac, Security};
use crate::wlan::mlme::{Ended, Event, NoConnect, Phase, Station};
use crate::wlan::supp::Fail;

use crate::air::Air;

struct Net {
    st: Station,
    air: Air,
    /// the latest event, until a command takes it
    event: Option<Event>,
    /// why the last connection ended, for `wifi status`
    ended: Option<Ended>,
}

static mut NET: Option<Net> = None;

/// The longest stretch with interrupts off under `with`, in microseconds (the architect's 20 ms masked rule;
/// `wifi status` prints it and pc-wifi-sim grades it).
static MAX_OFF_US: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Which step that longest stretch was (written by the holder of BUSY, read by `wifi status` holding it).
static mut MAX_OFF_WHAT: &str = "-";
/// The longest step run from a command, with interrupts on (for the evidence; not masked time).
static MAX_ON_US: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static mut MAX_ON_WHAT: &str = "-";
/// The Wi-Fi state's lock. A command takes it with interrupts ON (nothing is masked while it holds it, so the slow
/// steps of a handshake never mask interrupts); the timer interrupt's network poll finds it taken and skips that tick.
/// The interrupt runs to its end before the command resumes, so a command never finds it taken.
static BUSY: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Milliseconds since power-on.
fn now_ms() -> u64 {
    let hz = crate::timer::hz().max(1) as u128;
    (crate::timer::cycles() as u128 * 1000 / hz) as u64
}

/// Run `f` on the Wi-Fi state with interrupts off (the timer interrupt's network poll uses it too).
fn with<R>(f: impl FnOnce(&mut Net) -> R) -> Option<R> {
    with_as("a command", f)
}

/// `with`, naming the step for the masked-time record.
fn with_as<R>(what: &'static str, f: impl FnOnce(&mut Net) -> R) -> Option<R> {
    use core::sync::atomic::Ordering;
    // interrupts already off means the timer interrupt's poll (or a caller that masked them): masked time
    let masked = !crate::cpu::interrupts_on();
    if BUSY.swap(true, Ordering::Acquire) {
        return None; // the interrupt's poll while a command holds the state: skip this tick
    }
    let t0 = crate::timer::cycles();
    // SAFETY: BUSY is held, so no other user (a command or the timer interrupt, both on this core) touches NET until
    // it is released below; NET is set once at start-up (init) before any interrupt can reach it.
    let r = unsafe { (*core::ptr::addr_of_mut!(NET)).as_mut().map(f) };
    let us = (crate::timer::cycles().wrapping_sub(t0) as u128 * 1_000_000 / crate::timer::hz().max(1) as u128) as u64;
    // SAFETY (both writes): BUSY is still held
    if masked {
        if us > MAX_OFF_US.fetch_max(us, Ordering::Relaxed) {
            unsafe { *core::ptr::addr_of_mut!(MAX_OFF_WHAT) = what };
        }
    } else if us > MAX_ON_US.fetch_max(us, Ordering::Relaxed) {
        unsafe { *core::ptr::addr_of_mut!(MAX_ON_WHAT) = what };
    }
    BUSY.store(false, Ordering::Release);
    r
}

/// Is there a Wi-Fi radio? Called once from nic.rs at start-up; true makes Wi-Fi the PC's network card.
pub fn init() -> bool {
    crate::wlan::mlme::set_secret_rng(|b| crate::rng::fill_secret(b).is_ok());
    let Some(air) = Air::probe() else { return false };
    let mac = crate::wlan::mlme::Radio::mac(&air);
    // SAFETY: start-up, one core, before the emulated network device exists
    unsafe {
        *core::ptr::addr_of_mut!(NET) = Some(Net { st: Station::new(mac), air, event: None, ended: None });
    }
    true
}

/// Does Wi-Fi come before a cable? (Only in the test image, where the cable is the test access point's uplink.)
pub const PREFERRED: bool = crate::air::PREFERRED;

fn record(n: &mut Net, e: Option<Event>) {
    if let Some(e) = e {
        if let Event::Ended(why) = e {
            n.ended = Some(why);
        }
        n.event = Some(e);
    }
}

/// Move everything along: the radio's received frames into the station, the station's timers.
/// One step per interrupts-off stretch (the architect's 20 ms masked rule): the radio's own work, then each received
/// frame on its own, then the station's timers.
fn pump() {
    // the timer interrupt (interrupts off) moves only a connected station's traffic; joining and the handshake are
    // moved by the command's own loop, with interrupts on (the architect's 20 ms masked rule)
    if !crate::cpu::interrupts_on() && with_as("the poll's check", |n| n.st.phase != Phase::Connected).unwrap_or(true) {
        return;
    }
    let now = now_ms();
    with_as("the radio's tick", |n| n.air.tick(now));
    for _ in 0..16 {
        let mut buf = [0u8; 2400];
        let more = with_as("one received frame", |n| {
            let Some(len) = n.air.pop(&mut buf) else { return false };
            let e = n.st.on_frame(&mut n.air, &buf[..len], -40, 6, now);
            buf[..len].fill(0);
            record(n, e);
            true
        });
        if more != Some(true) {
            break;
        }
    }
    with_as("the station's timers", |n| {
        let e = n.st.poll(&mut n.air, now);
        record(n, e);
    });
}

// ---- nic.rs: the card the TCP/IP stack sees ----------------------------------------------------------------------

pub fn mac() -> [u8; 6] {
    with(|n| n.st.own()).unwrap_or([0; 6])
}

pub fn send(frame: &[u8]) -> bool {
    pump();
    with_as("sending a frame", |n| n.st.send_eth(&mut n.air, frame)).unwrap_or(false)
}

pub fn rx_ready() -> bool {
    pump();
    with(|n| n.st.rx_ready()).unwrap_or(false)
}

pub fn recv(dst: &mut [u8]) -> Option<usize> {
    with(|n| n.st.recv_eth(dst)).flatten()
}

/// Joined and keys in: worth starting DHCP.
pub fn link_up() -> bool {
    with(|n| n.st.phase == Phase::Connected).unwrap_or(false)
}

pub fn describe() -> &'static str {
    crate::air::DESCRIBE
}

pub fn short() -> &'static str {
    crate::air::SHORT
}

// ---- the commands ------------------------------------------------------------------------------------------------

fn present() -> bool {
    // SAFETY: a read of whether NET was set at start-up
    let yes = unsafe { (*core::ptr::addr_of!(NET)).is_some() };
    if !yes {
        crate::kprintln!("wifi: no Wi-Fi radio AfriSovera drives on this PC yet (the Intel 7265 driver is the next step); `wifi` lists the chips");
    }
    yes
}

fn security_text(s: Security) -> &'static str {
    match s {
        Security::Open => "open (no password)",
        Security::Wpa2Psk { sha256: false } => "WPA2-Personal",
        Security::Wpa2Psk { sha256: true } => "WPA2-Personal (SHA-256 keys, not yet)",
        Security::Wpa3Only => "WPA3 only (not yet)",
        Security::Enterprise => "enterprise (not supported)",
        Security::Old => "old WEP or TKIP (refused: broken)",
        Security::Unknown => "unknown (refused)",
    }
}

fn ssid_text(b: &[u8]) -> &str {
    match core::str::from_utf8(b) {
        Ok(s) if !b.is_empty() && b.iter().all(|&c| c >= 0x20 && c != 0x7F) => s,
        _ if b.is_empty() || b.iter().all(|&c| c == 0) => "(hidden)",
        _ => "(a name that is not plain text)",
    }
}

fn mac_text(m: &Mac) -> [u8; 17] {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut o = [b':'; 17];
    for (i, b) in m.iter().enumerate() {
        o[i * 3] = H[(b >> 4) as usize];
        o[i * 3 + 1] = H[(b & 15) as usize];
    }
    o
}

fn ended_text(e: &Ended) -> &'static str {
    match e {
        Ended::AuthRefused(_) => "the network refused to let this PC in",
        Ended::AssocRefused(_) => "the network refused to take this PC",
        Ended::NoAnswer => "the network did not answer",
        Ended::Handshake(Fail::Timeout) => "the key handshake did not finish: most often a wrong passphrase",
        Ended::Handshake(Fail::NoRandom) => "no secret random numbers on this PC, so no keys can be made",
        Ended::Handshake(Fail::RsnMismatch) => "the network's security in the handshake is not what it announces (refused: a possible attack)",
        Ended::Handshake(Fail::NotCcmp) => "the network offered a cipher other than CCMP (refused)",
        Ended::Handshake(_) => "the network sent keys that did not check out (refused)",
        Ended::Deauth(_) => "the network sent this PC away",
        Ended::BeaconLoss => "the network went quiet (out of range or switched off)",
        Ended::ByUser => "disconnected",
    }
}

fn wait_ms(ms: u64) {
    let end = now_ms() + ms;
    while now_ms() < end {
        pump();
        core::hint::spin_loop();
    }
}

/// `wifi scan`: listen to every network in range and list them, strongest first.
pub fn scan() {
    if !present() {
        return;
    }
    let ok = with(|n| n.st.scan(&mut n.air, now_ms())).unwrap_or(false);
    if !ok {
        crate::kprintln!("wifi scan: the radio would not scan");
        return;
    }
    wait_ms(300);
    with(|n| {
        let mut out = [None; crate::wlan::bss::MAX_BSS];
        let k = n.st.table.sorted(&mut out);
        crate::kprintln!("wifi scan: {} network(s) heard", k);
        for e in out[..k].iter().flatten() {
            let m = mac_text(&e.info.bssid);
            crate::kprintln!(
                "  {:<24} {} ch {:>2} {:>4} dBm  {}",
                ssid_text(e.info.ssid()),
                core::str::from_utf8(&m).unwrap_or("?"),
                e.info.channel,
                e.signal,
                security_text(e.info.security)
            );
        }
    });
}

/// Read a passphrase from this PC's keyboard or serial line without showing it. None if cancelled (Esc, Ctrl-C).
fn read_secret(buf: &mut [u8; 64]) -> Option<usize> {
    let mut n = 0usize;
    loop {
        let c = loop {
            if let Some(c) = crate::uart::getc() {
                break c;
            }
            crate::desk::idle();
            crate::uart::note_idle_wfi();
            crate::cpu::wfi();
        };
        match c {
            b'\r' | b'\n' => {
                crate::kprintln!("");
                return Some(n);
            }
            3 | 27 => {
                crypto::wipe(buf);
                crate::kprintln!(" (cancelled)");
                return None;
            }
            8 | 127 => n = n.saturating_sub(1),
            0x20..=0x7E if n < 64 => {
                buf[n] = c;
                n += 1;
            }
            _ => {}
        }
    }
}

fn no_connect(ssid: &str, e: NoConnect) {
    match e {
        NoConnect::NotFound => crate::kprintln!("wifi connect: no network called '{}' was heard; `wifi scan` lists them", ssid),
        NoConnect::NeedsPassphrase => crate::kprintln!("wifi connect: '{}' needs a passphrase", ssid),
        NoConnect::NotProtected => crate::kprintln!("wifi connect: '{}' is open; no passphrase is used", ssid),
        NoConnect::Unsupported(s) => crate::kprintln!("wifi connect: '{}' uses {}; AfriSovera does not join it", ssid, security_text(s)),
        NoConnect::Radio => crate::kprintln!("wifi connect: the radio would not join '{}'", ssid),
    }
}

/// `wifi connect <name>`: join the network, asking for its passphrase if it has one, then get an address.
pub fn connect(ssid: &str) {
    if ssid.is_empty() || ssid.len() > 32 {
        crate::kprintln!("wifi connect: give the network's name, as `wifi scan` shows it (up to 32 letters)");
        return;
    }
    if crate::report::capturing() {
        crate::kprintln!("wifi connect: refused from the network; the passphrase is typed only at this PC's keyboard or serial line");
        return;
    }
    if crate::shell::capturing() {
        crate::shell::end_catch(); // the refusal goes to the screen, not into the file
        crate::kprintln!("wifi connect: it asks for a passphrase, so it cannot run inside more or >");
        return;
    }
    if !present() {
        return;
    }
    let name = ssid.as_bytes();
    // the network must have been heard; listen once if it has not
    let heard = |n: &mut Net| {
        let mut out = [None; crate::wlan::bss::MAX_BSS];
        let k = n.st.table.sorted(&mut out);
        out[..k].iter().flatten().any(|e| e.info.ssid() == name)
    };
    if !with(heard).unwrap_or(false) {
        with(|n| n.st.scan(&mut n.air, now_ms()));
        wait_ms(300);
    }
    // an open network joins now; a protected one says it needs a passphrase; anything else is refused here
    // old events are cleared in the same breath as the connect: the timer interrupt may run the whole exchange
    // before this command looks again (seen in pc-wifi-sim: an end was cleared before it was read)
    let r = with(|n| {
        n.event = None;
        n.st.connect(&mut n.air, name, None, now_ms())
    })
    .unwrap_or(Err(NoConnect::Radio));
    match r {
        Ok(()) => {}
        Err(NoConnect::NeedsPassphrase) => {
            if !crypto::kat_passed() {
                crate::kprintln!("wifi connect: the Wi-Fi crypto has not been checked in this boot; checking it first");
                crate::wifi::selftest();
                if !crypto::kat_passed() {
                    crate::kprintln!("wifi connect: the Wi-Fi crypto failed its check; no keys are made this boot");
                    return;
                }
            }
            crate::kprint!("passphrase for '{}' (it is not shown; Enter ends, Esc cancels): ", ssid);
            let mut buf = [0u8; 64];
            let Some(len) = read_secret(&mut buf) else { return };
            crate::kprintln!("wifi connect: making keys (a few seconds)");
            // 8 to 63 characters are the passphrase, 64 hex digits the key itself; the typed bytes are wiped either way
            let pmk = crypto::pmk_from_typed(&mut buf[..len], name);
            crypto::wipe(&mut buf);
            let Ok(pmk) = pmk else {
                crate::kprintln!("wifi connect: a WPA2 passphrase is 8 to 63 letters, digits or signs (or the 64-digit hex key); nothing was sent");
                return;
            };
            let mut k = pmk.0;
            drop(pmk);
            let r = with(|n| {
                n.event = None;
                n.st.connect(&mut n.air, name, Some(k), now_ms())
            })
            .unwrap_or(Err(NoConnect::Radio));
            crypto::wipe(&mut k);
            if let Err(e) = r {
                no_connect(ssid, e);
                return;
            }
        }
        Err(e) => {
            no_connect(ssid, e);
            return;
        }
    }
    crate::kprintln!("wifi connect: joining '{}'", ssid);
    let end = now_ms() + 12_000;
    let mut ev = None;
    while now_ms() < end && ev.is_none() {
        pump();
        ev = with(|n| n.event.take()).flatten();
        core::hint::spin_loop();
    }
    match ev {
        Some(Event::Connected) => {
            let weaker = with(|n| n.st.weaker_ignored).unwrap_or(0);
            let sec = with(|n| n.st.bss().map(|b| b.security)).flatten();
            crate::kprintln!("wifi connect: connected to '{}' ({})", ssid, sec.map_or("?", security_text));
            if weaker > 0 {
                crate::kprintln!("wifi connect: {} weaker-security network(s) with the same name were ignored", weaker);
            }
            dhcp();
        }
        Some(Event::Ended(e)) => crate::kprintln!("wifi connect: not connected: {}", ended_text(&e)),
        None => {
            with(|n| {
                let _ = n.st.disconnect(&mut n.air);
            });
            crate::kprintln!("wifi connect: not connected: no answer within 12 seconds");
        }
    }
}

/// DHCP over the new link, with the timer interrupt on so the network device moves (as at start-up).
fn dhcp() {
    let was_on = crate::cpu::interrupts_on();
    if !was_on {
        crate::cpu::lapic_init(0x2_0000);
        crate::cpu::sti();
    }
    crate::virtio::net_dhcp();
    if !was_on {
        crate::cpu::cli();
    }
}

/// `wifi status`: the connection, its signal, and the counters (dropped frames are counted, never shown).
pub fn status() {
    if !present() {
        return;
    }
    with(|n| {
        let c = n.st.counters;
        match (n.st.phase, n.st.bss()) {
            (Phase::Idle, _) | (_, None) => crate::kprintln!("wifi status: not connected"),
            (p, Some(b)) => {
                let m = mac_text(&b.bssid);
                crate::kprintln!(
                    "wifi status: {} '{}' {} channel {} signal {} dBm, {}",
                    match p {
                        Phase::Connected => "connected to",
                        _ => "joining",
                    },
                    ssid_text(b.ssid()),
                    core::str::from_utf8(&m).unwrap_or("?"),
                    b.channel,
                    n.st.signal,
                    security_text(b.security)
                );
            }
        }
        crate::kprintln!(
            "  frames in {} out {} (send failures {}); dropped: unprotected {} bad MIC {} replays {} not ours {} malformed {} queue full {}",
            c.rx_frames, c.tx_frames, c.tx_fail, c.rx_plain_dropped, c.rx_mic_fail, c.rx_replay, c.rx_not_ours, c.rx_bad, c.rx_queue_full
        );
        if let Some(e) = &n.ended {
            crate::kprintln!("  last ended: {}", ended_text(e));
        }
        n.air.status_line();
        let us = MAX_OFF_US.load(core::sync::atomic::Ordering::Relaxed);
        // SAFETY: read holding BUSY (inside `with`), the only time they are written
        let (what, won) = unsafe { (*core::ptr::addr_of!(MAX_OFF_WHAT), *core::ptr::addr_of!(MAX_ON_WHAT)) };
        crate::kprintln!("  longest stretch with interrupts off: {}.{:03} ms ({})", us / 1000, us % 1000, what);
        let on = MAX_ON_US.load(core::sync::atomic::Ordering::Relaxed);
        crate::kprintln!("  longest step with interrupts on: {}.{:03} ms ({})", on / 1000, on % 1000, won);
    });
    let ip = crate::virtio::dhcp_ip();
    if ip != [0; 4] {
        crate::kprintln!("  address {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    }
}

/// `wifi disconnect`: leave the network; its keys are wiped.
pub fn disconnect() {
    if !present() {
        return;
    }
    let e = with(|n| {
        let e = n.st.disconnect(&mut n.air);
        record(n, e);
        n.event = None;
        e.is_some()
    })
    .unwrap_or(false);
    crate::kprintln!("{}", if e { "wifi disconnect: disconnected; the keys are wiped" } else { "wifi disconnect: not connected" });
}
