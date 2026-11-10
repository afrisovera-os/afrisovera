//! The PC's real-time clock: the CMOS RTC (MC146818-compatible) at I/O ports 0x70/0x71,
//! present on every PC chipset and emulated by QEMU (tracking the host's clock in UTC).
//! The PC counterpart of the ARM kernel's PL031 read: seconds since the Unix epoch.

use crate::cpu::{inb, outb};

const INDEX: u16 = 0x70;
const DATA: u16 = 0x71;

fn reg(r: u8) -> u8 {
    // SAFETY: CMOS index/data ports; bit 7 of the index keeps NMIs masked.
    unsafe {
        outb(INDEX, 0x80 | r);
        inb(DATA)
    }
}

fn updating() -> bool {
    reg(0x0A) & 0x80 != 0
}

#[derive(Clone, Copy, PartialEq)]
struct Raw {
    s: u8,
    m: u8,
    h: u8,
    d: u8,
    mo: u8,
    y: u8,
    c: u8,
}

fn raw() -> Raw {
    let mut spins = 0u32;
    while updating() && spins < 100_000 {
        spins += 1;
    }
    Raw { s: reg(0), m: reg(2), h: reg(4), d: reg(7), mo: reg(8), y: reg(9), c: reg(0x32) }
}

/// True when the chip answers like an RTC (a missing one reads back 0xFF everywhere).
pub fn present() -> bool {
    reg(0x0A) != 0xFF || reg(0x0B) != 0xFF
}

/// The status-B register (bit 2: binary mode, bit 1: 24-hour mode).
pub fn status_b() -> u8 {
    reg(0x0B)
}

/// The clock as the chip holds it, decoded (BCD or binary, 12 or 24 hour), plus the raw
/// hour, minute, second and century bytes, for `rtcinfo`.
#[derive(Clone, Copy)]
pub struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
    pub raw: [u8; 4],
}

/// The chip's date and time, read until two reads agree (the chip may tick between the
/// byte reads). None when no RTC answers.
pub fn civil() -> Option<Civil> {
    if !present() {
        return None;
    }
    let mut a = raw();
    for _ in 0..8 {
        let b = raw();
        if a == b {
            break;
        }
        a = b;
    }
    let sb = status_b();
    let bcd = |v: u8| if sb & 0x04 == 0 { (v & 0x0F) + (v >> 4) * 10 } else { v };
    let pm = a.h & 0x80 != 0;
    let mut hour = bcd(a.h & 0x7F) as u64;
    if sb & 0x02 == 0 {
        // 12-hour mode: 12 AM is 0, 12 PM is 12.
        hour %= 12;
        if pm {
            hour += 12;
        }
    }
    let century = match bcd(a.c) {
        c @ 19..=21 => c as i64,
        _ => 20,
    };
    Some(Civil {
        year: century * 100 + bcd(a.y) as i64,
        month: bcd(a.mo).clamp(1, 12) as i64,
        day: bcd(a.d).clamp(1, 31) as i64,
        hour,
        minute: bcd(a.m) as u64,
        second: bcd(a.s) as u64,
        raw: [a.h, a.m, a.s, a.c],
    })
}

/// Seconds since 1970-01-01 00:00:00 for a civil date and time (0 before 1970).
pub fn civil_epoch(year: i64, mo: i64, d: i64, hour: u64, minute: u64, second: u64) -> u64 {
    // days_from_civil (Howard Hinnant), the inverse of shell.rs's epoch_to_utc.
    let y = if mo <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    if days < 0 {
        return 0;
    }
    days as u64 * 86400 + hour * 3600 + minute * 60 + second
}

/// Seconds since 1970-01-01 00:00:00 UTC (the chip is taken to hold UTC). 0 when no RTC
/// answers.
pub fn epoch() -> u64 {
    match civil() {
        Some(c) => civil_epoch(c.year, c.month, c.day, c.hour, c.minute, c.second),
        None => 0,
    }
}
