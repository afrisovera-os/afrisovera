//! Bluetooth slice 1 (14 Nov 2026; BLUETOOTH_PLAN.md, ARCHITECT_REVIEWS 14 Nov "Bluetooth slice 1 plan"): the
//! HCI transport over USB, an Intel legacy controller from ROM (or patched when an exact firmware row matches),
//! and a scan the owner types. Read only towards other devices: the PC is never discoverable or connectable,
//! nothing connects, nothing pairs, no key is kept.
//!
//! - xhci.rs configures the controller's interrupt-IN endpoint (events) and sends commands as class control
//!   requests; every event packet comes here (`packet`, timer interrupt) and is put together and read by
//!   btparse.rs, which fuzz-host fuzzes (`bt_event`).
//! - Nothing is sent at boot. `bt` starts the controller: HCI_Reset and read commands only (the version, the
//!   address, the buffer sizes), and for an Intel legacy chip Intel Read Version and, only if fwdb lists a patch
//!   file for exactly that version and this device, the patch.
//! - `bt scan [seconds] [active]`: a passive LE scan by default (the radio only listens); `active` also sends LE
//!   scan requests and a classic inquiry. At most 30 s, then everything is switched off again.
//! - Our own BD_ADDR is shown with its maker half only. Remote devices' names go to the screen only (not the
//!   serial line, not the report, not a file); the report gets only the count. Under the test rig (fw_cfg
//!   opt/afrisovera/bt-names-serial) they go to the serial line too, which is what the suite reads.
//! - Slice 1 never changes the Bluetooth and Wi-Fi coexistence settings (they share one card).

use core::ptr::{addr_of, addr_of_mut};
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

use crate::btparse::{self, Asm, Event, Rec, Table};

static mut ASM: Asm = Asm::new();
static mut TABLE: Table = Table::new();
/// The opcode a command waits for (0: none), its whole answer, and whether it has come.
static WAIT_OP: AtomicU16 = AtomicU16::new(0);
static mut REPLY: ([u8; btparse::EVENT_MAX], usize) = ([0; btparse::EVENT_MAX], 0);
static REPLIED: AtomicBool = AtomicBool::new(false);
static SCANNING: AtomicBool = AtomicBool::new(false);
static EVENTS: AtomicU64 = AtomicU64::new(0);
static BAD: AtomicU64 = AtomicU64::new(0);
static STRAY: AtomicU64 = AtomicU64::new(0);

/// What `bt` found when it started the controller.
#[derive(Clone, Copy)]
struct Info {
    started: bool,
    hci: (u8, u16),
    lmp: (u8, u16),
    maker: u16,
    addr: [u8; 6],
    acl: (u16, u16),
    /// Intel Read Version (hw platform, variant, revision, fw variant, revision, build num, ww, yy, patch num)
    intel: Option<[u8; 9]>,
    /// The firmware line: "ROM ...", or the patch applied.
    fw: &'static str,
    patched: Option<&'static str>,
}
static mut INFO: Info = Info {
    started: false,
    hci: (0, 0),
    lmp: (0, 0),
    maker: 0,
    addr: [0; 6],
    acl: (0, 0),
    intel: None,
    fw: "not started (type `bt`)",
    patched: None,
};

/// The AfriSovera test controller (scripts/ci/qemu-hci-test.patch), which plays an Intel legacy chip.
const TEST_ID: (u16, u16) = (0x46F4, 0xAF02);
/// Intel's legacy controllers (before the bootloader parts), as Linux's btusb lists them.
const INTEL_LEGACY: [u16; 4] = [0x07DC, 0x0A2A, 0x0A2B, 0x0AA7];

/// One event packet from the interrupt endpoint (xhci.rs, timer interrupt).
pub fn packet(pkt: &[u8]) {
    // SAFETY: only the timer interrupt runs this (one core); mainline reads REPLY only after REPLIED (Acquire)
    // and TABLE only while SCANNING is off.
    unsafe {
        let asm = &mut *addr_of_mut!(ASM);
        let before = asm.dropped;
        asm.packet(pkt, &mut |e| {
            EVENTS.fetch_add(1, Ordering::Relaxed);
            match btparse::event(e) {
                Err(_) => {
                    BAD.fetch_add(1, Ordering::Relaxed);
                }
                Ok(ev) => {
                    let op = match ev {
                        Event::Complete { opcode, .. } | Event::Status { opcode, .. } => Some(opcode),
                        _ => None,
                    };
                    let want = WAIT_OP.load(Ordering::Relaxed);
                    if want != 0 && op == Some(want) && !REPLIED.load(Ordering::Relaxed) {
                        let r = &mut *addr_of_mut!(REPLY);
                        r.0[..e.len()].copy_from_slice(e);
                        r.1 = e.len();
                        REPLIED.store(true, Ordering::Release);
                    } else if op.is_some() {
                        STRAY.fetch_add(1, Ordering::Relaxed); // an answer to a command nobody is waiting for
                    } else if SCANNING.load(Ordering::Relaxed) {
                        (*addr_of_mut!(TABLE)).take(&ev);
                    }
                }
            }
        });
        BAD.fetch_add(asm.dropped - before, Ordering::Relaxed);
    }
}

/// The controller went (unplugged): everything forgotten.
pub fn gone() {
    SCANNING.store(false, Ordering::Relaxed);
    WAIT_OP.store(0, Ordering::Relaxed);
    // SAFETY: the controller is gone, so no event packet arrives meanwhile.
    unsafe {
        (*addr_of_mut!(ASM)).reset();
        (*addr_of_mut!(TABLE)).clear();
        INFO.started = false;
        INFO.fw = "not started (type `bt`)";
        INFO.patched = None;
    }
}

/// Send one command and wait for its Command Complete or Command Status (2 s): the whole answer event.
fn cmd(opcode: u16, params: &[u8], out: &mut [u8; btparse::EVENT_MAX]) -> Result<usize, &'static str> {
    if params.len() > 255 {
        return Err("a command too long");
    }
    let mut pkt = [0u8; 258];
    pkt[..2].copy_from_slice(&opcode.to_le_bytes());
    pkt[2] = params.len() as u8;
    pkt[3..3 + params.len()].copy_from_slice(params);
    REPLIED.store(false, Ordering::Relaxed);
    WAIT_OP.store(opcode, Ordering::Release);
    if !crate::xhci::bt_command(&pkt[..3 + params.len()]) {
        WAIT_OP.store(0, Ordering::Relaxed);
        return Err("the controller refused the command on the USB control pipe");
    }
    let t0 = crate::sched::ticks();
    while !REPLIED.load(Ordering::Acquire) {
        if crate::sched::ticks().wrapping_sub(t0) > 200 {
            WAIT_OP.store(0, Ordering::Relaxed);
            return Err("no answer in 2 s");
        }
        crate::cpu::wfi();
    }
    WAIT_OP.store(0, Ordering::Relaxed);
    // SAFETY: REPLIED (Acquire) says the interrupt has finished writing REPLY and will not write it again until
    // the next command clears REPLIED.
    let r = unsafe { &*addr_of!(REPLY) };
    out[..r.1].copy_from_slice(&r.0[..r.1]);
    Ok(r.1)
}

/// `cmd` for a command answered by Command Complete with a status first: the return parameters after the
/// status, or why not.
fn call(opcode: u16, params: &[u8], ret: &mut [u8; btparse::EVENT_MAX]) -> Result<usize, &'static str> {
    let mut e = [0u8; btparse::EVENT_MAX];
    let n = cmd(opcode, params, &mut e)?;
    match btparse::event(&e[..n]) {
        Ok(Event::Complete { ret: r, .. }) if !r.is_empty() => {
            if r[0] != 0 {
                return Err("the controller answered with an error status");
            }
            ret[..r.len() - 1].copy_from_slice(&r[1..]);
            Ok(r.len() - 1)
        }
        Ok(Event::Status { status: 0, .. }) => Ok(0),
        Ok(Event::Status { .. }) => Err("the controller answered with an error status"),
        _ => Err("the controller's answer did not read"),
    }
}

fn hex2(b: u8) -> [u8; 2] {
    const H: &[u8; 16] = b"0123456789abcdef";
    [H[(b >> 4) as usize], H[(b & 15) as usize]]
}

/// Intel's legacy patch file name for a Read Version answer, as Linux's btintel forms it:
/// ibt-hw-<platform>.<variant>.<revision>-fw-<variant>.<revision>.<build>.<ww>.<yy>.bseq (hex, no padding).
fn intel_name(v: &[u8; 9], out: &mut [u8; 64]) -> usize {
    let mut n = 0usize;
    let mut put = |s: &[u8]| {
        for &c in s {
            if n < out.len() {
                out[n] = c;
                n += 1;
            }
        }
    };
    let mut num = |x: u8, put: &mut dyn FnMut(&[u8])| {
        let h = hex2(x);
        if h[0] == b'0' { put(&h[1..]) } else { put(&h) }
    };
    put(b"ibt-hw-");
    for (i, &x) in v[..3].iter().enumerate() {
        num(x, &mut put);
        put(if i < 2 { b"." } else { b"-fw-" });
    }
    for (i, &x) in v[3..8].iter().enumerate() {
        num(x, &mut put);
        if i < 4 {
            put(b".");
        }
    }
    put(b".bseq");
    n
}

/// The patch, sent as the vendor documents it: Manufacturer Mode on, each command of the file sent and its answer
/// compared with the event the file names, then Manufacturer Mode off with "patches activated". Any failure part
/// way leaves Manufacturer Mode with a reset and the patches NOT activated (architect's condition), and the chip
/// runs from ROM. True when every command was answered as the file says.
fn patch(file: &[u8], cmds: &mut usize, fail_at: &mut usize) -> bool {
    let mut ret = [0u8; btparse::EVENT_MAX];
    if call(0xFC11, &[0x01, 0x00], &mut ret).is_err() {
        return false;
    }
    let mut ok = btparse::bseq_check(file).is_ok();
    let (mut at, mut pending) = (0usize, false);
    let mut got = [0u8; btparse::EVENT_MAX];
    let mut got_n = 0usize;
    while ok {
        let Ok(Some((r, next))) = btparse::bseq_next(file, at) else { break };
        match r {
            Rec::Command(c) => {
                *cmds += 1;
                let opcode = u16::from_le_bytes([c[0], c[1]]);
                match cmd(opcode, &c[3..], &mut got) {
                    Ok(n) => got_n = n,
                    Err(_) => {
                        ok = false;
                        *fail_at = *cmds;
                    }
                }
                pending = true;
            }
            Rec::Event(e) => {
                // the first event after a command is the answer it must give, byte for byte
                if core::mem::replace(&mut pending, false) && (got_n != e.len() || got[..got_n] != *e) {
                    ok = false;
                    *fail_at = *cmds;
                }
            }
        }
        at = next;
    }
    // Manufacturer Mode off: 0x02 = reset and activate the patches, 0x01 = reset, patches not activated
    let _ = call(0xFC11, &[0x00, if ok { 0x02 } else { 0x01 }], &mut ret);
    ok
}

/// `bt` without a word: start the controller if it has not been, then say what it is.
fn start() -> bool {
    // SAFETY: shell context; INFO is written only here and in `gone`.
    let info = unsafe { &mut *addr_of_mut!(INFO) };
    if info.started {
        return true;
    }
    if !crate::xhci::bt_present() {
        crate::kprintln!("bt: no Bluetooth controller (USB class E0/01/01) was found; lsusb shows what is there");
        return false;
    }
    let mut r = [0u8; btparse::EVENT_MAX];
    let steps: [(&str, u16); 1] = [("HCI_Reset", 0x0C03)];
    for (name, op) in steps {
        if let Err(why) = call(op, &[], &mut r) {
            crate::kprintln!("bt: {} failed: {}", name, why);
            return false;
        }
    }
    match call(0x1001, &[], &mut r) {
        Ok(n) if n >= 8 => {
            info.hci = (r[0], u16::from_le_bytes([r[1], r[2]]));
            info.lmp = (r[3], u16::from_le_bytes([r[6], r[7]]));
            info.maker = u16::from_le_bytes([r[4], r[5]]);
        }
        Ok(_) => crate::kprintln!("bt: Read Local Version answered too short"),
        Err(why) => crate::kprintln!("bt: Read Local Version failed: {}", why),
    }
    match call(0x1009, &[], &mut r) {
        Ok(n) if n >= 6 => info.addr.copy_from_slice(&r[..6]),
        _ => crate::kprintln!("bt: Read BD_ADDR failed"),
    }
    if let Ok(n) = call(0x1005, &[], &mut r) {
        if n >= 7 {
            info.acl = (u16::from_le_bytes([r[0], r[1]]), u16::from_le_bytes([r[3], r[4]]));
        }
    }
    let (vid, pid) = crate::xhci::bt_id();
    let intel = (vid == 0x8087 && INTEL_LEGACY.contains(&pid)) || (vid, pid) == TEST_ID;
    info.fw = "ROM (the controller's own)";
    if intel {
        match call(0xFC05, &[], &mut r) {
            Ok(n) if n >= 9 => {
                let mut v = [0u8; 9];
                v.copy_from_slice(&r[..9]);
                info.intel = Some(v);
                if v[8] != 0 {
                    info.fw = "already patched (the firmware's patch number is not 0)";
                } else {
                    let mut nb = [0u8; 64];
                    let nl = intel_name(&v, &mut nb);
                    let name = core::str::from_utf8(&nb[..nl]).unwrap_or("?");
                    match crate::fwdb::by_name(name) {
                        None => info.fw = "ROM (no patch file is listed for this exact version)",
                        Some(fw) => {
                            let (mut cmds, mut fail_at) = (0usize, 0usize);
                            let mut from = [0u8; 32];
                            let dev = crate::fwdb::Dev::UsbId { vendor: vid, product: pid };
                            match crate::fwdb::send(fw, dev, |bytes| patch(bytes, &mut cmds, &mut fail_at), &mut from) {
                                Ok(_) => {
                                    info.fw = "patched";
                                    info.patched = Some(fw.name);
                                    crate::kprintln!("bt: patch {}: {} commands, each answered as the file says; patches activated", fw.name, cmds);
                                }
                                Err(why) => {
                                    info.fw = "ROM (the patch was not applied)";
                                    if fail_at > 0 {
                                        crate::kprintln!("bt: patch {}: command {} was not answered as the file says; Manufacturer Mode left with a reset, patches not activated", fw.name, fail_at);
                                    }
                                    crate::kprintln!("bt: patch {}: {}", fw.name, why);
                                }
                            }
                        }
                    }
                }
            }
            _ => crate::kprintln!("bt: Intel Read Version failed"),
        }
    }
    info.started = true;
    true
}

fn maker_name(m: u16) -> &'static str {
    match m {
        2 => "Intel",
        15 => "Broadcom",
        29 => "Qualcomm",
        70 => "MediaTek",
        93 => "Realtek",
        0xFFFF => "test",
        _ => "",
    }
}

/// `bt` (status), `bt scan [seconds] [active]`, `bt stats`.
pub fn command(arg: &str) {
    let mut words = arg.split_whitespace();
    match words.next() {
        None => {
            if !start() {
                return;
            }
            // SAFETY: shell context; read after start().
            let i = unsafe { *addr_of!(INFO) };
            let (vid, pid) = crate::xhci::bt_id();
            let mut m = [0u8; 17];
            btparse::masked(&i.addr, &mut m);
            crate::kprintln!(
                "bt: controller {:04x}:{:04x} ({} {}), HCI {} rev {:#06x}, LMP {} subversion {:#06x}",
                vid, pid, maker_name(i.maker), i.maker, i.hci.0, i.hci.1, i.lmp.0, i.lmp.1
            );
            let addr = if i.addr == [0; 6] { "not read" } else { core::str::from_utf8(&m).unwrap_or("?") };
            crate::kprintln!(
                "bt: address {} (the maker half; the rest is never shown), ACL {} bytes x {}; never discoverable or connectable",
                addr, i.acl.0, i.acl.1
            );
            if let Some(v) = i.intel {
                crate::kprintln!(
                    "bt: Intel version: hw platform {:#04x} variant {:#04x} revision {:#04x}, fw variant {:#04x} revision {:#04x} build {} week {} year {}, patch {}",
                    v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]
                );
            }
            crate::kprintln!("bt: firmware: {}", i.fw);
            if let Some(n) = i.patched {
                crate::kprintln!("bt: patch file: {}", n);
            }
        }
        Some("scan") => scan(words),
        Some("stats") => stats(),
        _ => crate::kprintln!("bt: bt | bt scan [seconds, at most 30] [active] | bt stats"),
    }
}

fn stats() {
    // SAFETY: plain copies of counters the interrupt updates.
    let (asm_ev, asm_drop, n, full, bad) = unsafe {
        let a = &*addr_of!(ASM);
        let t = &*addr_of!(TABLE);
        (a.events, a.dropped, t.n, t.full, t.bad)
    };
    crate::kprintln!(
        "bt stats: {} events, {} dropped (bad length {}), {} unasked answers; last scan: {} devices kept, {} not kept (table full), {} reports that did not fit",
        asm_ev, asm_drop, BAD.load(Ordering::Relaxed), STRAY.load(Ordering::Relaxed), n, full, bad
    );
}

/// Names to the serial line too only under the test rig.
fn names_on_serial() -> bool {
    crate::qemu::fw_cfg_file(b"opt/afrisovera/bt-names-serial")
}

fn scan<'a>(mut words: impl Iterator<Item = &'a str>) {
    let mut secs = 5u64;
    let mut active = false;
    for w in words.by_ref() {
        match w {
            "active" => active = true,
            n => match n.parse::<u64>() {
                Ok(s) if (1..=30).contains(&s) => secs = s,
                _ => {
                    crate::kprintln!("bt scan: seconds from 1 to 30, then `active` if wanted");
                    return;
                }
            },
        }
    }
    if !start() {
        return;
    }
    let mut r = [0u8; btparse::EVENT_MAX];
    // SAFETY: SCANNING is off, so the interrupt does not touch TABLE.
    unsafe { (*addr_of_mut!(TABLE)).clear() };
    // reports can follow the controller's answer at once, so the table takes them from before the scan starts
    SCANNING.store(true, Ordering::Release);
    // LE scan parameters: passive (0) or active (1), interval and window 10 ms, our public address, no filter
    let le = call(0x200B, &[active as u8, 0x10, 0x00, 0x10, 0x00, 0x00, 0x00], &mut r).is_ok()
        && call(0x200C, &[0x01, 0x01], &mut r).is_ok();
    // a classic inquiry transmits, so only with `active`: the general inquiry access code, length in 1.28 s units
    let inq = active && call(0x0401, &[0x33, 0x8B, 0x9E, ((secs * 100 / 128) as u8).clamp(1, 0x30), 0x00], &mut r).is_ok();
    crate::kprintln!(
        "bt scan: {} s, {}{}",
        secs,
        if le { if active { "LE active (scan requests sent)" } else { "LE passive (listening only)" } } else { "no LE scan (the controller refused it)" },
        if active { if inq { " and a classic inquiry" } else { "; the classic inquiry was refused" } } else { "" }
    );
    let t0 = crate::sched::ticks();
    while crate::sched::ticks().wrapping_sub(t0) < secs * 100 {
        crate::cpu::wfi();
    }
    SCANNING.store(false, Ordering::Release);
    if le {
        let _ = call(0x200C, &[0x00, 0x00], &mut r);
    }
    if inq {
        let _ = call(0x0402, &[], &mut r); // Inquiry Cancel (an inquiry already over answers with an error: fine)
    }
    // SAFETY: SCANNING is off again: the interrupt no longer writes TABLE.
    let t = unsafe { &*addr_of!(TABLE) };
    crate::kprintln!("bt scan: {} device(s) seen{}; everything switched off again", t.n, if t.full > 0 { " (more than 32: not all kept)" } else { "" });
    let serial = names_on_serial();
    for d in &t.dev[..t.n] {
        let mut m = [0u8; 17];
        btparse::masked(&d.addr, &mut m);
        let kind = match d.kind {
            0 => "classic",
            1 => "LE public",
            _ => "LE random",
        };
        crate::kprintln!(
            "  {}  {:<9}  RSSI {:>4}  {} {:#08x}",
            core::str::from_utf8(&m).unwrap_or("?"), kind, d.rssi,
            if d.kind == 0 { "class" } else { "appearance" }, d.class
        );
        if d.name_len > 0 {
            let name = core::str::from_utf8(&d.name[..d.name_len]).unwrap_or("?");
            if serial {
                crate::kprintln!("      name: {}", name);
            } else {
                crate::uart::screen_fmt(format_args!("      name: {}\n", name));
            }
        }
    }
    if !serial && t.dev[..t.n].iter().any(|d| d.name_len > 0) {
        crate::kprintln!("bt scan: the names are on the screen only (never in the report or a file)");
    }
}
