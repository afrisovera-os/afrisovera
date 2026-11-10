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

/// The Linux driver family a Wi-Fi chip needs, and what that means for AfriSovera.
#[derive(Clone, Copy)]
enum Fam {
    Iwl,
    Rtw88,
    Rtw89,
    Rtlwifi,
    Mt76,
    Ath9k,
    Ath10k,
    Ath11k,
    Brcm,
}

fn fam_line(f: Fam) -> &'static str {
    match f {
        Fam::Iwl => "Linux driver iwlwifi; needs Intel's closed firmware file loaded first",
        Fam::Rtw88 => "Linux driver rtw88; needs Realtek's firmware file loaded first",
        Fam::Rtw89 => "Linux driver rtw89; needs Realtek's firmware file loaded first",
        Fam::Rtlwifi => "Linux driver rtlwifi; needs Realtek's firmware file loaded first",
        Fam::Mt76 => "Linux driver mt76 (mt7921e); needs MediaTek's firmware files loaded first",
        Fam::Ath9k => "Linux driver ath9k; needs NO firmware file -- the easiest chip to write a first Wi-Fi driver for",
        Fam::Ath10k => "Linux driver ath10k; needs Qualcomm's firmware files loaded first",
        Fam::Ath11k => "Linux driver ath11k; needs Qualcomm's firmware files loaded first",
        Fam::Brcm => "Broadcom; needs Broadcom's firmware (often only a closed driver exists)",
    }
}

/// Known Wi-Fi chips: (vendor, device, name, family).
const CHIPS: &[(u16, u16, &str, Fam)] = &[
    // Intel (0x8086): plug-in cards, then the CNVi Wi-Fi built into the chipset (its RF part is a CRF module)
    (0x8086, 0x272B, "Intel Wi-Fi 7 BE200", Fam::Iwl),
    (0x8086, 0x2725, "Intel Wi-Fi 6E AX210/AX211/AX411", Fam::Iwl),
    (0x8086, 0x2723, "Intel Wi-Fi 6 AX200", Fam::Iwl),
    (0x8086, 0x2526, "Intel Wireless-AC 9260", Fam::Iwl),
    (0x8086, 0x24FD, "Intel Wireless 8265", Fam::Iwl),
    (0x8086, 0x24F3, "Intel Wireless 8260", Fam::Iwl),
    (0x8086, 0x24FB, "Intel Wireless 3168", Fam::Iwl),
    (0x8086, 0x3165, "Intel Wireless 3165", Fam::Iwl),
    (0x8086, 0x3166, "Intel Wireless 3165", Fam::Iwl),
    (0x8086, 0x095A, "Intel Wireless 7265", Fam::Iwl),
    (0x8086, 0x095B, "Intel Wireless 7265", Fam::Iwl),
    (0x8086, 0x08B1, "Intel Wireless 7260", Fam::Iwl),
    (0x8086, 0x08B2, "Intel Wireless 7260", Fam::Iwl),
    (0x8086, 0x9DF0, "Intel CNVi Wi-Fi (Cannon Point-LP: 9560/9461/9462)", Fam::Iwl),
    (0x8086, 0xA370, "Intel CNVi Wi-Fi (Cannon Point-H: 9560)", Fam::Iwl),
    (0x8086, 0x31DC, "Intel CNVi Wi-Fi (Gemini Lake: 9560/9461/9462)", Fam::Iwl),
    (0x8086, 0x02F0, "Intel CNVi Wi-Fi (Comet Lake-LP: AX201/9560)", Fam::Iwl),
    (0x8086, 0x06F0, "Intel CNVi Wi-Fi (Comet Lake-H: AX201/9560)", Fam::Iwl),
    (0x8086, 0x34F0, "Intel CNVi Wi-Fi (Ice Lake: AX201/9560)", Fam::Iwl),
    (0x8086, 0x4DF0, "Intel CNVi Wi-Fi (Jasper Lake: AX201/9560)", Fam::Iwl),
    (0x8086, 0xA0F0, "Intel CNVi Wi-Fi (Tiger Lake: AX201)", Fam::Iwl),
    (0x8086, 0x43F0, "Intel CNVi Wi-Fi (Tiger Lake-H: AX201)", Fam::Iwl),
    (0x8086, 0x51F0, "Intel CNVi Wi-Fi (Alder Lake-P: AX211/AX201)", Fam::Iwl),
    (0x8086, 0x51F1, "Intel CNVi Wi-Fi (Alder Lake-P: AX211/AX201)", Fam::Iwl),
    (0x8086, 0x54F0, "Intel CNVi Wi-Fi (Alder Lake-N: AX211/AX201)", Fam::Iwl),
    (0x8086, 0x7A70, "Intel CNVi Wi-Fi (Alder Lake-S: AX211/AX201)", Fam::Iwl),
    (0x8086, 0x7AF0, "Intel CNVi Wi-Fi (Raptor Lake-S: AX211/AX201)", Fam::Iwl),
    (0x8086, 0x7E40, "Intel CNVi Wi-Fi (Meteor Lake: BE201/AX211)", Fam::Iwl),
    // Realtek (0x10EC)
    (0x10EC, 0xC821, "Realtek RTL8821CE", Fam::Rtw88),
    (0x10EC, 0xC822, "Realtek RTL8822CE", Fam::Rtw88),
    (0x10EC, 0xB822, "Realtek RTL8822BE", Fam::Rtw88),
    (0x10EC, 0xD723, "Realtek RTL8723DE", Fam::Rtw88),
    (0x10EC, 0x8852, "Realtek RTL8852AE", Fam::Rtw89),
    (0x10EC, 0xB852, "Realtek RTL8852BE", Fam::Rtw89),
    (0x10EC, 0xC852, "Realtek RTL8852CE", Fam::Rtw89),
    (0x10EC, 0xB723, "Realtek RTL8723BE", Fam::Rtlwifi),
    (0x10EC, 0x8179, "Realtek RTL8188EE", Fam::Rtlwifi),
    (0x10EC, 0x8821, "Realtek RTL8821AE", Fam::Rtlwifi),
    // MediaTek (0x14C3)
    (0x14C3, 0x7961, "MediaTek MT7921 (Wi-Fi 6)", Fam::Mt76),
    (0x14C3, 0x0608, "MediaTek MT7921K / RZ608 (Wi-Fi 6E)", Fam::Mt76),
    (0x14C3, 0x0616, "MediaTek MT7922 / RZ616 (Wi-Fi 6E)", Fam::Mt76),
    // Qualcomm Atheros (0x168C) and Qualcomm (0x17CB)
    (0x168C, 0x0032, "Qualcomm Atheros AR9485", Fam::Ath9k),
    (0x168C, 0x0034, "Qualcomm Atheros AR9462", Fam::Ath9k),
    (0x168C, 0x0036, "Qualcomm Atheros QCA9565 / AR9565", Fam::Ath9k),
    (0x168C, 0x002B, "Qualcomm Atheros AR9285", Fam::Ath9k),
    (0x168C, 0x0030, "Qualcomm Atheros AR93xx", Fam::Ath9k),
    (0x168C, 0x003E, "Qualcomm Atheros QCA6174", Fam::Ath10k),
    (0x168C, 0x0042, "Qualcomm Atheros QCA9377", Fam::Ath10k),
    (0x17CB, 0x1101, "Qualcomm QCA6390", Fam::Ath11k),
    (0x17CB, 0x1103, "Qualcomm WCN6855 / QCNFA765", Fam::Ath11k),
    // Broadcom (0x14E4)
    (0x14E4, 0x43A0, "Broadcom BCM4360", Fam::Brcm),
    (0x14E4, 0x43B1, "Broadcom BCM4352", Fam::Brcm),
    (0x14E4, 0x43A3, "Broadcom BCM4350", Fam::Brcm),
    (0x14E4, 0x4365, "Broadcom BCM43142", Fam::Brcm),
];

/// Linux driver families of USB Wi-Fi sticks.
#[derive(Clone, Copy)]
enum UFam {
    AthHtc,
    Rt2800,
    Mt7601,
    Mt76u,
    Rtl8xxxu,
    Rtw88u,
}

fn ufam_line(f: UFam) -> &'static str {
    match f {
        UFam::AthHtc => "Linux driver ath9k_htc; its firmware file is OPEN source (free to ship) -- the easiest USB stick to write a first Wi-Fi driver for",
        UFam::Rt2800 => "Linux driver rt2800usb; needs Ralink's firmware file (free to redistribute)",
        UFam::Mt7601 => "Linux driver mt7601u; needs MediaTek's firmware file loaded first",
        UFam::Mt76u => "Linux driver mt76 (USB); needs MediaTek's firmware files loaded first",
        UFam::Rtl8xxxu => "Linux driver rtl8xxxu; needs Realtek's firmware file loaded first",
        UFam::Rtw88u => "Linux driver rtw88 (USB); needs Realtek's firmware file loaded first",
    }
}

/// Known USB Wi-Fi sticks: (vendor, product, name, family).
const STICKS: &[(u16, u16, &str, UFam)] = &[
    (0x0CF3, 0x9271, "Qualcomm Atheros AR9271 (e.g. TP-Link TL-WN722N v1, Alfa AWUS036NHA)", UFam::AthHtc),
    (0x0CF3, 0x7015, "Qualcomm Atheros AR7010 (e.g. TP-Link TL-WN821N v3)", UFam::AthHtc),
    (0x0846, 0x9030, "Qualcomm Atheros AR9271 (Netgear WNA1100)", UFam::AthHtc),
    (0x148F, 0x5370, "Ralink RT5370", UFam::Rt2800),
    (0x148F, 0x3070, "Ralink RT3070", UFam::Rt2800),
    (0x148F, 0x7601, "MediaTek MT7601U", UFam::Mt7601),
    (0x0E8D, 0x7612, "MediaTek MT7612U", UFam::Mt76u),
    (0x0E8D, 0x7961, "MediaTek MT7921AU (Wi-Fi 6)", UFam::Mt76u),
    (0x0BDA, 0x8179, "Realtek RTL8188EUS", UFam::Rtl8xxxu),
    (0x0BDA, 0x8176, "Realtek RTL8188CUS", UFam::Rtl8xxxu),
    (0x0BDA, 0x818B, "Realtek RTL8192EU", UFam::Rtl8xxxu),
    (0x2357, 0x010C, "Realtek RTL8188EUS (TP-Link TL-WN722N v2/v3)", UFam::Rtl8xxxu),
    (0x2357, 0x0109, "Realtek RTL8192EU (TP-Link TL-WN823N v2)", UFam::Rtl8xxxu),
    (0x0BDA, 0xC811, "Realtek RTL8811CU", UFam::Rtw88u),
    (0x0BDA, 0xB812, "Realtek RTL88x2BU", UFam::Rtw88u),
];

fn stick(v: u16, p: u16) -> Option<(&'static str, UFam)> {
    STICKS.iter().find(|c| c.0 == v && c.1 == p).map(|c| (c.2, c.3))
}

fn chip(v: u16, d: u16) -> Option<(&'static str, Fam)> {
    CHIPS.iter().find(|c| c.0 == v && c.1 == d).map(|c| (c.2, c.3))
}

/// A wired chip AfriSovera drives: the Intel e1000/e1000e/igb parts (e1000.rs), the Realtek
/// r8169 family and RTL8139C+ (rtl8169.rs), or virtio-net.
fn driven(v: u16, d: u16) -> Option<&'static str> {
    if v == 0x8086 {
        return crate::e1000::PARTS.iter().find(|p| p.0 == d).map(|p| p.1);
    }
    if let Some(p) = crate::rtl8169::PARTS.iter().find(|p| p.0 == v && p.1 == d) {
        return Some(p.2);
    }
    if v == 0x1AF4 && (d == 0x1000 || d == 0x1041) {
        return Some("virtio-net");
    }
    None
}

const OWNER: &str = "  for the owner: AfriSovera has no Wi-Fi driver yet -- connect with a network cable (an Intel or Realtek Ethernet port) or a USB Ethernet adapter; the chip named above tells us which Wi-Fi driver to write first";

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
                    crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Wi-Fi: {} -- {}", b, d, f, v, dv, name, fam_line(fam));
                } else if class == 0x0280 {
                    wifis += 1;
                    crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Wi-Fi or other network chip, not in AfriSovera's list -- tell us this id", b, d, f, v, dv);
                } else if class == 0x0200 {
                    // an RTL8139 before the C+ (revision 0x20) has no descriptor rings: not driven
                    let old8139 = v == 0x10EC && dv == 0x8139 && cfg32(b, d, f, 0x08) & 0xFF < 0x20;
                    match if old8139 { None } else { driven(v, dv) } {
                        Some(n) => crate::kprintln!("  {:02x}:{:02x}.{} {:04x}:{:04x} Ethernet (cable): {} -- AfriSovera drives it", b, d, f, v, dv, n),
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
                    crate::kprintln!("  usb {:04x}:{:04x} Wi-Fi stick: {} -- {}", v, p, name, ufam_line(fam));
                }
                None => crate::kprintln!(
                    "  usb {:04x}:{:04x} (class {:02x}/{:02x}) not a Wi-Fi stick in AfriSovera's list -- tell us this id if it is one",
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

fn hex4(s: &str) -> Option<u16> {
    let h = s.get(..4)?;
    if h.bytes().all(|c| c.is_ascii_hexdigit()) {
        u16::from_str_radix(h, 16).ok()
    } else {
        None
    }
}

/// The (vendor, device) in `8086:2723`, Device Manager's `PCI\VEN_8086&DEV_2723&SUBSYS_...` or
/// `USB\VID_0BDA&PID_8179&REV_0000`, and which form: Some(false) PCI, Some(true) USB, None plain.
fn parse(s: &str) -> Option<(u16, u16, Option<bool>)> {
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
            Some((name, fam)) => crate::kprintln!("wifi: usb {:04x}:{:04x} is {} (USB Wi-Fi stick) -- {}", v, d, name, ufam_line(fam)),
            None => crate::kprintln!("wifi: usb {:04x}:{:04x} is not in AfriSovera's list of USB Wi-Fi sticks -- tell us this id", v, d),
        }
        return;
    }
    let usb_stick = if form.is_none() { stick(v, d) } else { None };
    if let Some((name, fam)) = chip(v, d) {
        crate::kprintln!("wifi: {:04x}:{:04x} is {} (Wi-Fi) -- {}", v, d, name, fam_line(fam));
    } else if let Some(n) = driven(v, d) {
        crate::kprintln!("wifi: {:04x}:{:04x} is {}, a wired Ethernet chip AfriSovera drives (a network cable works)", v, d, n);
    } else if let Some((name, fam)) = usb_stick {
        crate::kprintln!("wifi: usb {:04x}:{:04x} is {} (USB Wi-Fi stick) -- {}", v, d, name, ufam_line(fam));
    } else {
        crate::kprintln!("wifi: {:04x}:{:04x} is not in AfriSovera's list of network chips -- tell us this id", v, d);
    }
}
