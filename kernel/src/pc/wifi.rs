//! `wifi` (PC track step 3, Wi-Fi groundwork, 7 Nov 2026): which network chips this PC has, READ ONLY.
//!
//! Wi-Fi is the last step-3 driver and the biggest (a full 802.11 + WPA2 stack, and most chips need a
//! closed firmware file loaded first). Before writing one we need to know WHICH chip the owner's PCs
//! have. `wifi` walks every PCI bus (configuration mechanism #1, the same ports the other PC drivers
//! use; only reads), lists every network controller (PCI class 0x02) with its vendor:device id, names
//! the known Wi-Fi chips and the Linux driver family each needs, and says for each wired chip whether
//! AfriSovera drives it. `wifi <id>` names a chip from its id as Windows' Device Manager shows it
//! (Hardware Ids, e.g. PCI\VEN_8086&DEV_2723) or as `8086:2723`, without booting AfriSovera on that PC.
//! Nothing is written anywhere; no device is enabled or touched beyond its id and class registers.
//! USB Wi-Fi sticks (7 Nov 2026): the devices the xHCI driver found at power-on (or plugged in since)
//! and does not use are named from their vendor:product, and `wifi` also takes Device Manager's
//! USB\VID_0BDA&PID_8179 form. A stick is the cheap way to give a PC a Wi-Fi chip we choose.

// PCI configuration space (read only here) through the shared pc/pci.rs (review R1-8).
use crate::pci::cfg_read32 as cfg32;
// The chip and stick tables and their family lines live in pc/hwdb.rs since 12 Nov 2026 (hwscan reads them
// too); this file prints exactly what it printed before.
use crate::hwdb::{chip, fam_line, stick, ufam_line};

/// A wired chip AfriSovera drives: the Intel e1000/e1000e/igb parts (e1000.rs), the Realtek
/// r8169 family and RTL8139C+ (rtl8169.rs), or virtio-net.
fn driven(v: u16, d: u16) -> Option<&'static str> {
    // the drivers' own match tables (Asili v0 BINDS), by id alone
    if let Some(p) = crate::hwdb::id_part(crate::e1000::BINDS, v, d) {
        return Some(p.name);
    }
    if let Some(p) = crate::hwdb::id_part(crate::rtl8169::BINDS, v, d) {
        return Some(p.name);
    }
    if v == 0x1AF4 && (d == 0x1000 || d == 0x1041) {
        return Some("virtio-net");
    }
    None
}

const OWNER: &str = "  for the owner: AfriSovera has no Wi-Fi driver yet; connect with a network cable (an Intel or Realtek Ethernet port) or a USB Ethernet adapter; the chip named above tells us which Wi-Fi driver to write first";

/// `wifi`: every network controller on the PCI buses, named; read only.
pub fn show() {
    crate::kprintln!("wifi: network chips on this PC's PCI buses (read only, nothing is changed)");
    let (mut funcs, mut nets, mut wifis) = (0u32, 0u32, 0u32);
    for b in 0..=255u8 {
        for d in 0..32u8 {
            let id0 = cfg32(b, d, 0, 0);
            if id0 & 0xFFFF == 0xFFFF {
                continue;
            }
            let multi = cfg32(b, d, 0, 0x0C) & 0x0080_0000 != 0;
            for f in 0..if multi { 8u8 } else { 1 } {
                let id = if f == 0 { id0 } else { cfg32(b, d, f, 0) };
                let (v, dv) = ((id & 0xFFFF) as u16, (id >> 16) as u16);
                if v == 0xFFFF {
                    continue;
                }
                funcs += 1;
                let class = cfg32(b, d, f, 0x08) >> 16; // base class << 8 | subclass
                let known = chip(v, dv);
                if class >> 8 != 0x02 && known.is_none() {
                    continue;
                }
                nets += 1;
                if let Some((name, fam)) = known {
                    wifis += 1;
                    crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Wi-Fi: {}; {}", b, d, f, v, dv, name, fam_line(fam));
                } else if class == 0x0280 {
                    wifis += 1;
                    crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Wi-Fi or other network chip, not in AfriSovera's list; tell us this id", b, d, f, v, dv);
                } else if class == 0x0200 {
                    // an RTL8139 before the C+ (revision 0x20) has no descriptor rings: not driven
                    let old8139 = v == 0x10EC && dv == 0x8139 && cfg32(b, d, f, 0x08) & 0xFF < 0x20;
                    match if old8139 { None } else { driven(v, dv) } {
                        Some(n) => crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Ethernet (cable): {}; AfriSovera drives it", b, d, f, v, dv, n),
                        None => crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Ethernet (cable): no driver in AfriSovera yet", b, d, f, v, dv),
                    }
                } else {
                    crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} network controller (class {:04x}): no driver in AfriSovera", b, d, f, v, dv, class);
                }
            }
        }
    }
    crate::kprintln!("wifi: {} PCI function(s) looked at, {} network chip(s), {} Wi-Fi", funcs, nets, wifis);
    let (usb, list, n) = crate::xhci::others();
    let mut sticks = 0u32;
    if !usb {
        crate::kprintln!("wifi: no USB controller in use, so no USB Wi-Fi stick looked for");
    } else {
        crate::kprintln!("wifi: USB devices AfriSovera does not use: {}", n);
        for &(v, p, dc, ic) in &list[..n] {
            match stick(v, p) {
                Some((name, fam)) => {
                    sticks += 1;
                    crate::kprintln!("  usb {:04x}:{:04x} Wi-Fi stick: {}; {}", v, p, name, ufam_line(fam));
                }
                None => crate::kprintln!(
                    "  usb {:04x}:{:04x} (class {:02x}/{:02x}) not a Wi-Fi stick in AfriSovera's list; tell us this id if it is one",
                    v, p, dc, ic
                ),
            }
        }
    }
    if wifis == 0 && sticks == 0 {
        crate::kprintln!("wifi: no Wi-Fi chip on PCI and no USB Wi-Fi stick AfriSovera knows");
    }
    crate::kprintln!("{}", OWNER);
}

/// `wifi selftest` (W1, 14 Nov 2026): every published test vector of the Wi-Fi crypto (kernel/src/wlan/crypto_kat.rs,
/// parsed from the RFCs and IEEE 802.11 annexes by scripts/wifi-kat-fetch.sh), one line per family with its time.
/// Wi-Fi may make keys only after this has passed in this boot (`wlan::crypto::kat_passed`). Nothing is sent.
pub fn selftest() {
    use crate::wlan::crypto::{run_kind, set_kat_result, Kind, KINDS};
    crate::kprintln!("wifi selftest: the Wi-Fi crypto against its published test vectors (nothing is sent or changed)");
    let hz = crate::timer::hz().max(1);
    let (mut total, mut bad) = (0usize, 0usize);
    for k in KINDS {
        if matches!(k, Kind::Pbkdf2Sha1 | Kind::Psk) {
            crate::kprintln!("  {}: making keys (a few seconds)", k.name());
        }
        let t0 = crate::timer::cycles();
        let r = run_kind(k);
        let ms = crate::timer::cycles().wrapping_sub(t0).saturating_mul(1000) / hz;
        match r {
            Ok(n) => {
                total += n;
                if k == Kind::Psk && n > 0 {
                    crate::kprintln!("  {}: {} of {} PASS in {} ms ({} ms for one passphrase)", k.name(), n, n, ms, ms / n as u64);
                } else {
                    crate::kprintln!("  {}: {} of {} PASS in {} ms", k.name(), n, n, ms);
                }
            }
            Err(i) => {
                bad += 1;
                crate::kprintln!("  {}: vector {} FAILED", k.name(), i + 1);
            }
        }
    }
    set_kat_result(bad == 0);
    if bad == 0 {
        crate::kprintln!("wifi selftest: {} of {} vectors PASS; Wi-Fi may make keys", total, total);
    } else {
        crate::kprintln!("wifi selftest: FAILED in {} famil{}; Wi-Fi will not make keys this boot", bad, if bad == 1 { "y" } else { "ies" });
    }
}

pub(crate) fn hex4(s: &str) -> Option<u16> {
    let h = s.get(..4)?;
    if h.bytes().all(|c| c.is_ascii_hexdigit()) {
        u16::from_str_radix(h, 16).ok()
    } else {
        None
    }
}

/// The (vendor, device) in `8086:2723`, Device Manager's `PCI\VEN_8086&DEV_2723&SUBSYS_...` or
/// `USB\VID_0BDA&PID_8179&REV_0000`, and which form: Some(false) PCI, Some(true) USB, None plain.
pub(crate) fn parse(s: &str) -> Option<(u16, u16, Option<bool>)> {
    let mut up = [0u8; 96];
    let n = s.len().min(up.len());
    for (o, c) in up.iter_mut().zip(s.bytes().take(n)) {
        *o = c.to_ascii_uppercase();
    }
    let u = core::str::from_utf8(&up[..n]).ok()?;
    if let (Some(a), Some(b)) = (u.find("VEN_"), u.find("DEV_")) {
        return Some((hex4(&u[a + 4..])?, hex4(&u[b + 4..])?, Some(false)));
    }
    if let (Some(a), Some(b)) = (u.find("VID_"), u.find("PID_")) {
        return Some((hex4(&u[a + 4..])?, hex4(&u[b + 4..])?, Some(true)));
    }
    let (a, b) = u.split_once(':')?;
    if a.len() != 4 || b.len() != 4 {
        return None;
    }
    Some((hex4(a)?, hex4(b)?, None))
}

/// `wifi <id>`: name a chip from its PCI id, e.g. read from Windows' Device Manager on that PC.
pub fn lookup(arg: &str) {
    let Some((v, d, form)) = parse(arg) else {
        crate::kprintln!("wifi: give the chip's id as 8086:2723, or as Windows' Device Manager shows it (PCI\\VEN_8086&DEV_2723 or USB\\VID_0BDA&PID_8179)");
        return;
    };
    if form == Some(true) {
        match stick(v, d) {
            Some((name, fam)) => crate::kprintln!("wifi: usb {:04x}:{:04x} is {} (USB Wi-Fi stick); {}", v, d, name, ufam_line(fam)),
            None => crate::kprintln!("wifi: usb {:04x}:{:04x} is not in AfriSovera's list of USB Wi-Fi sticks; tell us this id", v, d),
        }
        return;
    }
    let usb_stick = if form.is_none() { stick(v, d) } else { None };
    if let Some((name, fam)) = chip(v, d) {
        crate::kprintln!("wifi: {:04x}:{:04x} is {} (Wi-Fi); {}", v, d, name, fam_line(fam));
    } else if let Some(n) = driven(v, d) {
        crate::kprintln!("wifi: {:04x}:{:04x} is {}, a wired Ethernet chip AfriSovera drives (a network cable works)", v, d, n);
    } else if let Some((name, fam)) = usb_stick {
        crate::kprintln!("wifi: usb {:04x}:{:04x} is {} (USB Wi-Fi stick); {}", v, d, name, ufam_line(fam));
    } else {
        crate::kprintln!("wifi: {:04x}:{:04x} is not in AfriSovera's list of network chips; tell us this id", v, d);
    }
}
