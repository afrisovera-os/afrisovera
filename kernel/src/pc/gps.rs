//! A GPS receiver on a USB serial cable, and the shell's `sensors` (sensors S1: SENSORS_PLAN.md, the architect's OK
//! TO BUILD and the security engineer's rules of 21 Nov 2026).
//!
//! xhci.rs hands every byte from a USB serial cable to `rx`. nmea.rs's `Link` decides whether the cable is a person's
//! console or a GPS: until a good GGA, RMC, GSA or GSV sentence arrives the bytes go on to the shell as before; from
//! then on the cable is a GPS, its sentences build the fix, and the console is no longer mirrored to it.
//!
//! Where a reading may go (the security engineer, 21 Nov 2026), until the sensor grants of S2 exist:
//! - only to the `sensors` line on the screen, typed by the person;
//! - never to COM1 or the USB serial console, never to a saved `report`, `acpisave` or any file. Under the test rig
//!   only (QEMU, fw_cfg `opt/afrisovera/sensors-serial`) the position line goes to COM1 too, so a suite can grade it;
//!   without it COM1 gets a line saying the position is on the screen only;
//! - no app, Nia or Daraja path reads it;
//! - error lines carry counts only, never a sentence's bytes or a position.

use crate::nmea::{Fix, Kind, Link};
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicBool, Ordering};

static LOCK: AtomicBool = AtomicBool::new(false);
static GPS: AtomicBool = AtomicBool::new(false);
static mut LINK: Link = Link::new();

/// Run `f` on the link with the lock held and interrupts masked on this core (the cable's poll and the shell meet
/// here; either core may run either).
fn with<R>(f: impl FnOnce(&mut Link) -> R) -> R {
    let flags: u64;
    // SAFETY: save RFLAGS and mask interrupts on this core while the lock is held, as uart.rs's prints do.
    unsafe { core::arch::asm!("pushfq; pop {}; cli", out(reg) flags) };
    while LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        core::hint::spin_loop();
    }
    // SAFETY: LINK is touched only here, under LOCK.
    let r = f(unsafe { &mut *addr_of_mut!(LINK) });
    LOCK.store(false, Ordering::Release);
    if flags & 0x200 != 0 {
        crate::cpu::sti();
    }
    r
}

/// A new USB serial cable: a console until it shows otherwise.
pub fn reset() {
    with(|l| l.reset());
    GPS.store(false, Ordering::Release);
}

/// True once the cable has shown itself a GPS: xhci.rs then stops mirroring the console to it.
pub fn active() -> bool {
    GPS.load(Ordering::Acquire)
}

/// One byte from the USB serial cable.
pub fn rx(b: u8) {
    // at most a held line and the byte that ended it go to the shell at once
    let mut out = [0u8; crate::nmea::BODY + 1];
    let mut n = 0;
    let became = with(|l| {
        l.byte(b, &mut |c| {
            if n < out.len() {
                out[n] = c;
                n += 1;
            }
        })
    });
    for &c in &out[..n] {
        crate::uart::inject(c);
    }
    if became {
        GPS.store(true, Ordering::Release);
        crate::kprintln!(
            "[gps] the USB serial cable speaks NMEA 0183: a GPS receiver. Its lines no longer reach the shell and the console is no longer mirrored to it; `sensors` shows the fix"
        );
    }
}

/// Under the test rig only: the position line to COM1 as well as the screen.
fn position_on_serial() -> bool {
    crate::qemu::fw_cfg_file(b"opt/afrisovera/sensors-serial")
}

fn deg(v: i32, pos: char, neg: char) -> (u32, u32, char) {
    let a = v.unsigned_abs();
    (a / 10_000_000, a % 10_000_000, if v < 0 { neg } else { pos })
}

/// The shell's `sensors`: the sensors this PC reads, and the GPS fix (reads only).
pub fn command(_rest: &str) {
    crate::kprintln!("sensors: the sensors this PC reads (reads only; a position is shown on the screen only)");
    if !active() {
        crate::kprintln!(
            "sensors: no GPS (a GPS receiver on a USB serial cable is read once it sends NMEA 0183 sentences; none has)"
        );
        return;
    }
    let (fix, c): (Fix, _) = with(|l| (l.fix, l.counts));
    crate::kprintln!(
        "sensors: GPS on the USB serial cable: NMEA 0183, {} sentences read, {} with a bad checksum, {} too long, {} malformed, {} of other kinds",
        c.good, c.checksum, c.too_long, c.malformed, c.other
    );
    let view = match fix.in_view {
        Some(v) => v as u32,
        None => 0,
    };
    let used = fix.sats_used.map_or(0, |s| s as u32);
    let (pos, acc) = match (fix.kind, fix.pos, fix.accuracy_mm()) {
        (Kind::None, _, _) | (_, None, _) => {
            crate::kprintln!("sensors: GPS fix: none ({} satellites in use, {} in view)", used, view);
            return;
        }
        (_, Some(p), a) => (p, a),
    };
    let kind = if fix.kind == Kind::D3 { "3D" } else { "2D" };
    match (acc, fix.hdop_c) {
        (Some(a), Some(h)) => crate::kprintln!(
            "sensors: GPS fix: {}, {} satellites in use, {} in view, accuracy about {}.{:03} m (estimated from HDOP {}.{:02})",
            kind, used, view, a / 1000, a % 1000, h / 100, h % 100
        ),
        _ => crate::kprintln!(
            "sensors: GPS fix: {}, {} satellites in use, {} in view, accuracy unknown (no HDOP)",
            kind, used, view
        ),
    }
    let line = |w: &mut dyn FnMut(core::fmt::Arguments)| {
        let (la, laf, ns) = deg(pos.lat_e7, 'N', 'S');
        let (lo, lof, ew) = deg(pos.lon_e7, 'E', 'W');
        w(format_args!("sensors: GPS position: {}.{:07} {}, {}.{:07} {}", la, laf, ns, lo, lof, ew));
        if let Some(a) = fix.alt_mm {
            let m = a.unsigned_abs();
            w(format_args!(", altitude {}{}.{:03} m", if a < 0 { "-" } else { "" }, m / 1000, m % 1000));
        }
        if let Some(s) = fix.speed_mms {
            w(format_args!(", speed {}.{:03} m/s", s / 1000, s % 1000));
        }
        if let Some(c) = fix.course_cdeg {
            w(format_args!(", course {}.{:02} degrees", c / 100, c % 100));
        }
        if let Some(t) = fix.t_wall_ms {
            let (d, ms) = ((t / 86_400_000) as i64, t % 86_400_000);
            let (y, mo, da) = civil(d);
            let s = ms / 1000;
            w(format_args!(
                ", time {:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} UTC",
                y, mo, da, s / 3600, s / 60 % 60, s % 60, ms % 1000
            ));
        }
        w(format_args!("\n"));
    };
    if position_on_serial() {
        line(&mut |a| crate::kprint!("{}", a));
    } else {
        line(&mut |a| crate::uart::screen_fmt(a));
        crate::kprintln!("sensors: GPS position: on the screen only");
    }
}

/// The date of a day number since 1970-01-01 (H. Hinnant's civil_from_days).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}
