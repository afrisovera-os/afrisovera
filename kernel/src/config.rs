//! Persistent system configuration.
//!
//! A small `afrisovera.conf` file in the VFS holds real key=value state — the
//! hostname, the DHCP-acquired network config, and a boot counter — and is
//! written through to the virtio-blk disk via the VFS's `sync_to_disk`. On the
//! next boot the file is restored from disk (by `vfs::load_from_disk`), so the
//! settings survive a reboot. This ties the storage + VFS + net subsystems into
//! one durable-configuration feature.

//! 2.736: the text is no longer built with `format!`. This runs on **every disk boot**,
//! before the shell exists, and `format!` allocates infallibly — so on a full arena the
//! null went to the language's allocation error handler and took the machine down for a
//! 90-byte string, at the one moment there is nobody at a console to read why. It now
//! renders into a fixed buffer and REFUSES by name, so a full arena costs the config
//! write and the boot continues.

use crate::fmtbuf;

const CONF: &str = "afrisovera.conf";

/// The buffer the config text renders into.
///
/// Exactly the VFS's per-file limit, so the two bounds coincide: anything [`render`]
/// accepts, `vfs::write` stores whole. A smaller buffer would refuse files the VFS could
/// have held; a larger one would let `vfs::write` silently truncate past its own limit,
/// and a truncated config is worse than none — it parses, and it lies.
const CONF_MAX: usize = crate::vfs::MAX_FILE_BYTES;

/// Refused: the VFS would not take the file.
pub const BOUND_VFS: &str = "the VFS has no slot for it (file table full, or the name is too long)";

/// Read the persisted boot counter (0 if there is no saved config yet).
fn saved_boots() -> u32 {
    if let Some(s) = crate::vfs::read_str(CONF) {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("boots=") {
                return v.trim().parse::<u32>().unwrap_or(0);
            }
        }
    }
    0
}

/// The longest language code kept (ISO 639 codes are 2 or 3 letters; room for a regional tag later).
pub const LANG_MAX: usize = 8;

/// Is this a language code the config keeps: 2 to LANG_MAX lowercase ASCII letters. Anything else in the file reads
/// as English, so a damaged line can never reach the desk as text.
fn lang_ok(c: &str) -> bool {
    (2..=LANG_MAX).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase())
}

/// The desk's language as saved (LANGUAGES_PLAN.md): the `lang=` line, or "en" when there is none or it is not a
/// plain code.
pub fn lang() -> &'static str {
    crate::vfs::read_str(CONF).map(|s| lang_in(s).unwrap_or("en")).unwrap_or("en")
}

/// The desk's test language (LANGUAGES_PLAN.md stage 6): never kept (the architect, 15 Nov).
pub const TEST_LANG: &str = "qps";

/// What a config's `lang=` line gives: Ok(the code), or Ok("en") when there is none or it is not a plain code. A line
/// naming the test language is Err(it), refused by name, and the desk reads English.
pub fn lang_in(text: &str) -> Result<&str, &str> {
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("lang=") {
            let v = v.trim();
            if v == TEST_LANG {
                return Err(v);
            }
            if lang_ok(v) {
                return Ok(v);
            }
        }
    }
    Ok("en")
}

/// A `lang=` line naming the test language, if the saved config has one (the desk says so at the first gui).
pub fn lang_refused() -> Option<&'static str> {
    crate::vfs::read_str(CONF).and_then(|s| lang_in(s).err())
}

/// The person's clock this boot, once they choose (Settings, `desk clock`): 0 none yet (the file says), 1 12-hour,
/// 2 24-hour. Kept here so the next config write carries it.
static CLOCK: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// What a `clock24=` line says (LANGUAGES_PLAN.md stage 5b, the architect's Q3): Some(true) for `1`, Some(false) for
/// `0`, None when there is no line. Anything else is Err(the value): strict like `lang=`, and the clock then follows
/// the language.
pub fn clock24_in(text: &str) -> Result<Option<bool>, &str> {
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("clock24=") {
            return match v.trim() {
                "1" => Ok(Some(true)),
                "0" => Ok(Some(false)),
                other => Err(other),
            };
        }
    }
    Ok(None)
}

/// The person's clock: Some(true) 24-hour, Some(false) 12-hour, None when they have not chosen (or the saved line
/// is damaged): the desk then uses the language's usual clock.
pub fn clock24() -> Option<bool> {
    match CLOCK.load(core::sync::atomic::Ordering::Relaxed) {
        1 => Some(false),
        2 => Some(true),
        _ => crate::vfs::read_str(CONF).and_then(|s| clock24_in(s).ok().flatten()),
    }
}

/// A saved `clock24=` value that is not 0 or 1, if there is one (the desk says so at the first gui).
pub fn clock24_damaged() -> Option<&'static str> {
    crate::vfs::read_str(CONF).and_then(|s| clock24_in(s).err())
}

/// The person chose a clock (Settings, `desk clock 24|12`): keep it, like the language, through the same
/// allocation-free path. From now on it beats the language's default.
pub fn set_clock24(on: bool) -> Result<(), &'static str> {
    CLOCK.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
    persist(saved_boots(), lang())
}

/// Render the configuration text into a caller-owned buffer — **pure**, and the whole
/// of what used to be a `format!`.
///
/// Takes the four inputs rather than reading them, for the reason `heap::place` and
/// `frame::plan` do: the live kernel only ever renders one config, so the boundary that
/// decides whether this path can fail is unreachable from a shell unless the arithmetic
/// can be asked directly. [`selftest`] asks it.
pub fn render<'a>(
    buf: &'a mut [u8],
    ip: [u8; 4],
    gw: [u8; 4],
    dns: [u8; 4],
    boots: u32,
    lang: &str,
    clock24: Option<bool>,
) -> Result<&'a str, &'static str> {
    let lang = if lang_ok(lang) { lang } else { "en" };
    // the clock's line only once the person has chosen; until then the file is exactly what it was
    let clock = match clock24 {
        Some(true) => "clock24=1\n",
        Some(false) => "clock24=0\n",
        None => "",
    };
    fmtbuf::render(
        buf,
        format_args!(
            "hostname=afrisovera\nip={}.{}.{}.{}\ngw={}.{}.{}.{}\ndns={}.{}.{}.{}\nboots={}\nlang={}\n{}",
            ip[0], ip[1], ip[2], ip[3],
            gw[0], gw[1], gw[2], gw[3],
            dns[0], dns[1], dns[2], dns[3],
            boots, lang, clock
        ),
    )
}

/// Render the current system state and write it to the config file, syncing to
/// disk when a virtio-blk device is present — or refuse **by name**.
///
/// Allocation-free end to end: [`render`] fills a stack buffer and `vfs::write` copies
/// into a static table. A full heap can no longer cost the boot here, because this path
/// no longer asks the heap for anything.
fn persist(boots: u32, lang: &str) -> Result<(), &'static str> {
    let mut buf = [0u8; CONF_MAX];
    let conf = render(
        &mut buf,
        crate::virtio::dhcp_ip(),
        crate::virtio::dhcp_gw(),
        crate::virtio::dhcp_dns(),
        boots,
        lang,
        clock24(),
    )?;
    // Checked rather than dropped: `vfs::write` has always returned whether it took the
    // file, and a config that was never written is exactly the state the next boot would
    // silently misread as "no config saved yet" and start the boot count over.
    if !crate::vfs::write(CONF, conf.as_bytes()) {
        return Err(BOUND_VFS);
    }
    if crate::virtio::blk_present() {
        crate::vfs::sync_to_disk();
    }
    Ok(())
}

/// At boot (after `vfs::load_from_disk`): if there is a disk to persist to, bump
/// the boot counter, save the config, and report. Skipped entirely when no
/// virtio-blk device is present, so diskless boots are unaffected.
pub fn boot_apply() {
    if !crate::virtio::blk_present() {
        return;
    }
    let prev = saved_boots();
    let boots = prev + 1;
    // The refusal costs the config write, not the boot (2.736). Reported and stepped
    // over: an operator who cannot see why the boot count stopped moving has a machine
    // that quietly forgets every reboot.
    if let Err(why) = persist(boots, lang()) {
        crate::kprintln!("[cfg] NOT saved — {}; booting on without persisting", why);
        return;
    }
    let ip = crate::virtio::dhcp_ip();
    if prev > 0 {
        crate::kprintln!(
            "[cfg] loaded from disk; hostname=afrisovera ip={}.{}.{}.{} boots={}",
            ip[0], ip[1], ip[2], ip[3], boots
        );
    } else {
        crate::kprintln!("[cfg] initialized; hostname=afrisovera boots={}", boots);
    }
}

/// The persisted boot count (0 if no config has been saved yet).
pub fn boots() -> u32 {
    saved_boots()
}

/// Shell `config`: print the saved configuration.
pub fn show() {
    match crate::vfs::read_str(CONF) {
        Some(s) => {
            crate::kprintln!("[cfg] {}:", CONF);
            for line in s.lines() {
                if !line.is_empty() {
                    crate::kprintln!("  {}", line);
                }
            }
        }
        None => crate::kprintln!("[cfg] no configuration saved"),
    }
}

fn case_ok(pass: &mut u32, total: &mut u32, ok: bool, what: &str) {
    *total += 1;
    if ok {
        *pass += 1;
        crate::kprintln!("  [ok]   {}", what);
    } else {
        crate::kprintln!("  [FAIL] {}", what);
    }
}

/// Put the allocation-free config render through the buffers the live kernel never
/// gives it (`config test`).
///
/// The live machine renders one config, ~90 bytes, into a 512-byte buffer, so the
/// refusal path this milestone exists to create has never executed and never will on a
/// healthy boot — the same reason `heap test` exists. What is proved here is that the
/// path is *correct when it does*, and — the claim that actually matters — that it
/// CANNOT fire in production, because the widest config the kernel can ever produce
/// still fits.
pub fn selftest() {
    let (mut pass, mut total) = (0u32, 0u32);
    const IP: [u8; 4] = [10, 0, 2, 15];
    const GW: [u8; 4] = [10, 0, 2, 2];
    const DNS: [u8; 4] = [10, 0, 2, 3];
    const EXPECT: &str =
        "hostname=afrisovera\nip=10.0.2.15\ngw=10.0.2.2\ndns=10.0.2.3\nboots=7\nlang=en\n";

    crate::kprintln!("config: allocation-free render against synthetic buffers");

    let mut buf = [0u8; CONF_MAX];
    // The baseline: the fixed buffer produces exactly the bytes `format!` used to.
    // Byte-for-byte, because the next boot re-reads and parses this text.
    let exact = match render(&mut buf, IP, GW, DNS, 7, "en", None) {
        Ok(s) => {
            let n = s.len();
            case_ok(&mut pass, &mut total, s == EXPECT,
                "the rendered config is byte-identical to what format! built before it");
            n
        }
        Err(_) => {
            case_ok(&mut pass, &mut total, false,
                "the rendered config is byte-identical to what format! built before it");
            0
        }
    };

    // The boundary, on both sides — the whole reason the check is a refusal and not a
    // truncation. One byte short must lose the WHOLE write, not the last character:
    // "boots=7\n" cut to "boots=" parses on the next boot, and lies.
    let fits = matches!(render(&mut buf[..exact], IP, GW, DNS, 7, "en", None), Ok(s) if s == EXPECT);
    case_ok(&mut pass, &mut total, exact > 0 && fits,
        "a buffer of exactly the rendered length is served, to the last byte");
    case_ok(&mut pass, &mut total,
        exact > 0 && render(&mut buf[..exact - 1], IP, GW, DNS, 7, "en", None) == Err(fmtbuf::BOUND_BUF),
        "one byte short is REFUSED by name — never written short, never half a key=value");
    // ...and every smaller buffer likewise: no length below the fit may report success.
    let mut all_refuse = exact > 0;
    for n in 0..exact {
        all_refuse &= render(&mut buf[..n], IP, GW, DNS, 7, "en", None).is_err();
    }
    case_ok(&mut pass, &mut total, all_refuse,
        "EVERY buffer below the fit refuses; none reports a partial render as a success");

    // The claim that keeps the refusal theoretical: the widest config this kernel can
    // ever produce is the all-255 address triple with a saturated boot counter, and it
    // still fits the buffer with room to spare. If this case ever fails, the boot path
    // has acquired a bound it can actually hit.
    let mut wide = [0u8; CONF_MAX];
    let widest = render(&mut wide, [255; 4], [255; 4], [255; 4], u32::MAX, "abcdefgh", Some(true))
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    case_ok(&mut pass, &mut total, widest <= CONF_MAX,
        "the WIDEST config the kernel can produce (255.255.255.255 x3, u32::MAX boots, an 8-letter language, a clock24 line) fits");

    // the clock's line (stage 5b): written only once chosen, read back as written, anything else refused by name
    let mut cb = [0u8; CONF_MAX];
    let chosen = render(&mut cb, IP, GW, DNS, 7, "en", Some(false)).unwrap_or("");
    case_ok(&mut pass, &mut total,
        chosen.ends_with("lang=en\nclock24=0\n") && clock24_in(chosen) == Ok(Some(false))
            && clock24_in(EXPECT) == Ok(None) && clock24_in("clock24=1\n") == Ok(Some(true)),
        "a clock24 line is written only once chosen, and reads back as written");
    case_ok(&mut pass, &mut total,
        clock24_in("clock24=yes\n") == Err("yes") && clock24_in("clock24=\n") == Err("")
            && clock24_in("clock24=10\n") == Err("10"),
        "a clock24 line that is not 0 or 1 is refused by name, and the clock follows the language");

    // stage 6: the test language is never kept, so a config naming it reads as English (the architect, 15 Nov)
    case_ok(&mut pass, &mut total,
        lang_in("hostname=afrisovera
lang=qps
") == Err("qps") && lang_in(EXPECT) == Ok("en")
            && lang_in("lang=tw
") == Ok("tw") && lang_in("lang=NOT A CODE
") == Ok("en"),
        "a config naming the test language qps is refused by name and reads as English");
    // And what fits the buffer fits the file: `vfs::write` silently truncates past its
    // own per-file limit, so the buffer must not be the larger of the two.
    case_ok(&mut pass, &mut total, CONF_MAX <= crate::vfs::MAX_FILE_BYTES,
        "the buffer is no larger than the VFS file limit, so nothing rendered is truncated");

    // The writer underneath, on the two properties the panic path depends on: it stops
    // at the buffer's end, and it stops on a character boundary. A buffer that filled
    // mid-character would hand the console an invalid string at the worst moment.
    let mut small = [0u8; 2];
    let (text, dropped) = fmtbuf::render_lossy(&mut small, format_args!("aéb"));
    case_ok(&mut pass, &mut total, text == "a" && dropped == 3,
        "a lossy render stops on a CHAR boundary, keeping the prefix and counting the rest");
    let mut none = [0u8; 0];
    let (empty, lost) = fmtbuf::render_lossy(&mut none, format_args!("boots=7"));
    case_ok(&mut pass, &mut total, empty.is_empty() && lost == 7,
        "a zero-length buffer writes nothing and overruns nothing, rather than faulting");
    let (whole, none_lost) = fmtbuf::render_lossy(&mut buf, format_args!("boots={}", 7u32));
    case_ok(&mut pass, &mut total, whole == "boots=7" && none_lost == 0,
        "text that fits is rendered whole, with nothing reported dropped");

    crate::kprintln!(
        "config: {}/{} render checks held ({} B buffer, {} B rendered, {} B widest possible)",
        pass, total, CONF_MAX, exact, widest
    );
}

/// Shell `config save`: persist the current state (keeping the boot count).
pub fn save() {
    if let Err(why) = persist(saved_boots(), lang()) {
        crate::kprintln!("[cfg] NOT saved to {} — {}", CONF, why);
        return;
    }
    if crate::virtio::blk_present() {
        crate::kprintln!("[cfg] saved to {} on disk", CONF);
    } else {
        crate::kprintln!("[cfg] saved to {} (RAM only; no virtio-blk to persist)", CONF);
    }
}

/// The desk's language changed (Settings, `desk lang <code>`): keep it, with the boot count as it is, through the
/// same allocation-free path as the boot. With a disk it is synced and survives a reboot; without one it lasts
/// until power off. Refused by name, never half written.
pub fn set_lang(code: &str) -> Result<(), &'static str> {
    if code == TEST_LANG {
        return Err("a test language is never kept");
    }
    if !lang_ok(code) {
        return Err("not a language code (2 to 8 lowercase letters)");
    }
    persist(saved_boots(), code)
}
