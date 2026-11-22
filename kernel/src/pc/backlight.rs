//! `backlight` (PC track step 3, 14 Nov 2026): how bright the laptop's own screen is, as the Intel
//! graphics chip's backlight PWM has it. READ ONLY.
//!
//! The owner's HP (field report 14 Nov 2026: Skylake-U, Sunrise Point chipset, Intel HD 520) has no
//! brightness control in AfriSovera. Before anything sets the brightness we read what the firmware
//! left: on Intel laptops since 2013 the panel's backlight is a PWM on the chipset's display side,
//! three dwords in the graphics chip's register BAR (BAR0) at 0xC8250. What they hold depends on the
//! chipset generation (Intel's graphics Programmer's Reference Manuals: BLC_PWM_PCH_CTL1 and CTL2 at 0xC8250 and
//! 0xC8254 up to Union Point; from Cannon Point on, the period and the duty in two registers at 0xC8254 and
//! 0xC8258), told from the ISA bridge's device id at 00:1f.0 (id & 0xFF80, the chipset datasheets' id ranges):
//! - Lynx Point to Union Point (2013-2017): CTL2 holds the period (bits 31:16) and the duty (15:0).
//! - Cannon Point and later (2018 on): CTL2 is the period, the next dword the duty, 32 bits each.
//! In both, CTL1 bit 31 says the PWM is on and bit 29 that it is inverted (duty counts darkness).
//! A chipset not in the list gets its three dwords shown raw, not guessed at.
//!
//! Nothing is written: no register, no PCI command bit. If the firmware left the chip's memory
//! decoding off, the registers are not read at all. Panels whose brightness is set over the
//! DisplayPort AUX channel, or by the firmware (ACPI _BCM), show the PWM off; that is said, not hidden.

use crate::mmu;
use crate::pci;
use core::ptr::read_volatile;

const CTL1: u64 = 0xC8250; // bit 31 PWM on, bit 29 inverted (both layouts)
const CTL2: u64 = 0xC8254; // older layout: period (31:16) and duty (15:0); newer layout: the period
const DUTY: u64 = 0xC8258; // newer layout only: the duty

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Packed, // period and duty in one dword (Lynx Point to Union Point)
    Split,  // period and duty in two dwords (Cannon Point and later)
}

/// The chipset family from the ISA bridge's device id (id & 0xFF80, the ranges in Intel's chipset datasheets).
fn layout(pch: u16) -> Option<(Layout, &'static str)> {
    Some(match pch & 0xFF80 {
        0x8C00 | 0x9C00 => (Layout::Packed, "Lynx Point, 2013"),
        0x8C80 | 0x9C80 => (Layout::Packed, "Wildcat Point, 2014"),
        0xA100 | 0x9D00 => (Layout::Packed, "Sunrise Point, 2015"),
        0xA280 => (Layout::Packed, "Union Point, 2017"),
        0xA300 | 0x9D80 => (Layout::Split, "Cannon Point, 2018"),
        0x0280 | 0x0680 | 0xA380 => (Layout::Split, "Comet Point, 2019"),
        0x3480 => (Layout::Split, "Ice Point, 2019"),
        0xA080 | 0x4380 => (Layout::Split, "Tiger Point, 2020"),
        _ => return None,
    })
}

/// (on, inverted, duty, period) from the three dwords.
fn decode(l: Layout, c1: u32, c2: u32, d: u32) -> (bool, bool, u32, u32) {
    let on = c1 & (1 << 31) != 0;
    let inv = c1 & (1 << 29) != 0;
    match l {
        Layout::Packed => (on, inv, c2 & 0xFFFF, c2 >> 16),
        Layout::Split => (on, inv, d, c2),
    }
}

/// The brightness in percent (rounded), or None when the values make no sense (no period, or a duty
/// longer than the period). An inverted PWM counts the dark part, so the light is period - duty.
fn percent(inv: bool, duty: u32, period: u32) -> Option<u32> {
    if period == 0 || duty > period {
        return None;
    }
    let lit = if inv { period - duty } else { duty } as u64;
    Some(((lit * 100 + period as u64 / 2) / period as u64) as u32)
}

/// The first display controller (PCI class 0x03) on the first buses: (bus, dev, fn, id).
fn display() -> Option<(u8, u8, u8, u32)> {
    let mut idv = 0;
    let at = pci::find_by(8, |b, d, f, id| {
        idv = id;
        pci::cfg_read32(b, d, f, 8) >> 24 == 0x03
    })?;
    Some((at.0, at.1, at.2, idv))
}

/// `backlight`: the Intel graphics backlight PWM, read only.
pub fn show() {
    crate::kprintln!("backlight: the screen's backlight as the graphics chip has it (read only, nothing is changed)");
    let id = pci::cfg_read32(0, 2, 0, 0);
    let class = pci::cfg_read32(0, 2, 0, 8) >> 24;
    if id & 0xFFFF != 0x8086 || class != 0x03 {
        match display() {
            Some((b, d, f, did)) => crate::kprintln!(
                "backlight: no Intel graphics at 00:02.0; the display here is {:04x}:{:04x} at {:02x}:{:02x}.{}; only Intel graphics backlights are read so far",
                did & 0xFFFF,
                did >> 16,
                b,
                d,
                f
            ),
            None => crate::kprintln!("backlight: no display controller on PCI; nothing to read"),
        }
        return;
    }
    let pch = pci::cfg_read32(0, 0x1F, 0, 0);
    let pch_dev = if pch & 0xFFFF == 0x8086 { (pch >> 16) as u16 } else { 0 };
    let fam = layout(pch_dev);
    crate::kprintln!(
        "backlight: Intel graphics 8086:{:04x} at 00:02.0; chipset 8086:{:04x} ({})",
        id >> 16,
        pch_dev,
        fam.map(|(_, n)| n).unwrap_or("not in AfriSovera's list")
    );
    if pci::cfg_read32(0, 2, 0, 4) & 0x2 == 0 {
        crate::kprintln!("backlight: the chip's registers are switched off (PCI memory decoding off); not read");
        return;
    }
    let bar = pci::bar_mem(0, 2, 0, 0);
    if bar == 0 || !mmu::map_mmio(bar + CTL1, 0x10, "backlight") {
        crate::kprintln!("backlight: the chip has no usable register BAR; not read");
        return;
    }
    // SAFETY: three aligned dword reads inside BAR0, just mapped uncached; reads change nothing there.
    let (c1, c2, d) = unsafe {
        (
            read_volatile((bar + CTL1) as *const u32),
            read_volatile((bar + CTL2) as *const u32),
            read_volatile((bar + DUTY) as *const u32),
        )
    };
    let Some((l, _)) = fam else {
        crate::kprintln!(
            "backlight: raw registers C8250={:08x} C8254={:08x} C8258={:08x}; not decoded for this chipset, tell us its id",
            c1,
            c2,
            d
        );
        return;
    };
    let (on, inv, duty, period) = decode(l, c1, c2, d);
    if !on {
        crate::kprintln!(
            "backlight: PWM off; this panel's brightness is set another way (the panel's own DisplayPort control or the firmware), not read yet"
        );
        return;
    }
    match percent(inv, duty, period) {
        Some(p) => crate::kprintln!(
            "backlight: PWM on, duty {} of {}{}: about {}% bright",
            duty,
            period,
            if inv { " (inverted)" } else { "" },
            p
        ),
        None => crate::kprintln!("backlight: PWM on but the values make no sense (duty {}, period {}); not used", duty, period),
    }
}

/// `backlight selftest`: the decoding on known register values, no hardware touched.
pub fn selftest() {
    let mut pass = 0u32;
    let mut fail = 0u32;
    let mut check = |name: &str, ok: bool| {
        if ok {
            pass += 1;
        } else {
            fail += 1;
            crate::kprintln!("backlight selftest: {} FAILED", name);
        }
    };
    let p = Layout::Packed;
    let s = Layout::Split;
    check("packed half", decode(p, 0x8000_0000, 9765 << 16 | 4882, 0) == (true, false, 4882, 9765));
    check("packed half percent", percent(false, 4882, 9765) == Some(50));
    check("packed fifth", percent(false, 1953, 9765) == Some(20));
    check("inverted fifth", decode(p, 0xA000_0000, 0, 0).1 && percent(true, 1953, 9765) == Some(80));
    check("packed off", !decode(p, 0x2000_0000, 9765 << 16 | 4882, 0).0);
    check("split full", decode(s, 0x8000_0000, 30000, 30000) == (true, false, 30000, 30000) && percent(false, 30000, 30000) == Some(100));
    check("split ignores CTL2 duty half", decode(s, 0x8000_0000, 0x0001_0000, 0x8000) == (true, false, 0x8000, 0x0001_0000));
    check("no period", percent(false, 5, 0).is_none());
    check("duty past period", percent(false, 11, 10).is_none());
    check("dark", percent(false, 0, 10) == Some(0));
    check("HP Sunrise Point (9d48)", matches!(layout(0x9D48), Some((Layout::Packed, _))));
    check("Cannon Point (9d84)", matches!(layout(0x9D84), Some((Layout::Split, _))));
    check("QEMU ICH9 (2918) unknown", layout(0x2918).is_none());
    crate::kprintln!("backlight selftest: {} of {} pass", pass, pass + fail);
}
