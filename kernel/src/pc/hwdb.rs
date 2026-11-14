//! The hardware database (12 Nov 2026): each driver's match table, and the names of hardware AfriSovera
//! knows but does not drive yet, for `hwscan` and `wifi`.
//!
//! Asili v0 convention (AFR-0002 section 4.1, architect's review of DRIVER_PLAN.md, 12 Nov 2026): every
//! bus driver exports `pub const BINDS: &[Part]`, and its probe decides from that table and nothing else
//! (`pci_part`). `hwscan` reads the same tables, so the inventory can never disagree with what the drivers
//! really match. The shape is the v1 manifest's `binds` (AFR-0002 section 5.1), so a v1 driver package's
//! manifest is a direct translation.
//!
//! The Wi-Fi tables moved here from wifi.rs unchanged (`wifi` prints exactly what it printed before).

use crate::pci::cfg_read32 as cfg32;

/// One thing a driver binds to. Plain data, as the v1 manifest's `binds`.
#[derive(Clone, Copy)]
#[allow(dead_code)] // the USB and ACPI forms are declared for the standard; the xHCI walk still decides USB
pub enum Bind {
    /// A PCI vendor and device id; `mask` picks the device-id bits that must be equal (0xFFFF for one
    /// device), `min_rev` is the lowest revision taken (0 for any).
    PciId { vendor: u16, device: u16, mask: u16, min_rev: u8 },
    /// A PCI class code (base class, subclass, programming interface: 24 bits) under `mask`.
    PciClass { class: u32, mask: u32 },
    /// A USB interface's class, subclass and protocol; `mask` bit 0 class, bit 1 subclass, bit 2 protocol.
    UsbClass { class: u8, sub: u8, proto: u8, mask: u8 },
    /// A USB vendor and product id (0xFFFF product: any product of that vendor).
    UsbId { vendor: u16, product: u16 },
    /// An ACPI _HID or _CID.
    Acpi(&'static str),
}

/// A match and the name the driver gives what it matched.
#[derive(Clone, Copy)]
pub struct Part {
    pub bind: Bind,
    pub name: &'static str,
}

/// One PCI device id.
pub const fn pci(vendor: u16, device: u16, name: &'static str) -> Part {
    Part { bind: Bind::PciId { vendor, device, mask: 0xFFFF, min_rev: 0 }, name }
}
/// One PCI device id from revision `min_rev` on.
pub const fn pci_rev(vendor: u16, device: u16, min_rev: u8, name: &'static str) -> Part {
    Part { bind: Bind::PciId { vendor, device, mask: 0xFFFF, min_rev }, name }
}
/// A block of PCI device ids (`device` under `mask`).
pub const fn pci_ids(vendor: u16, device: u16, mask: u16, name: &'static str) -> Part {
    Part { bind: Bind::PciId { vendor, device, mask, min_rev: 0 }, name }
}
/// A PCI class code under a mask.
pub const fn pci_class(class: u32, mask: u32, name: &'static str) -> Part {
    Part { bind: Bind::PciClass { class, mask }, name }
}

/// Does `bind` take the PCI function with this vendor:device (`id`, configuration dword 0) and
/// class/revision dword (`cr`, configuration dword 8)?
fn pci_hit(bind: &Bind, id: u32, cr: u32) -> bool {
    let (v, d) = ((id & 0xFFFF) as u16, (id >> 16) as u16);
    match *bind {
        Bind::PciId { vendor, device, mask, min_rev } => {
            v == vendor && d & mask == device & mask && (cr & 0xFF) as u8 >= min_rev
        }
        Bind::PciClass { class, mask } => (cr >> 8) & mask == class & mask,
        _ => false,
    }
}

/// The part of `parts` that takes PCI function b:d.f (`id` its configuration dword 0), if any. Every
/// PCI driver's probe decides with this, from its own BINDS.
pub fn pci_part(parts: &'static [Part], b: u8, d: u8, f: u8, id: u32) -> Option<&'static Part> {
    let cr = cfg32(b, d, f, 8);
    parts.iter().find(|p| pci_hit(&p.bind, id, cr))
}

/// The part of `parts` that names this vendor:device by id alone (no class or revision known: an id
/// typed in from Windows' Device Manager).
pub fn id_part(parts: &'static [Part], v: u16, d: u16) -> Option<&'static Part> {
    parts.iter().find(|p| matches!(p.bind, Bind::PciId { vendor, device, mask, .. } if v == vendor && d & mask == device & mask))
}

/// The part of `parts` that takes this PCI class code (24 bits) by class alone.
pub fn class_part(parts: &'static [Part], class: u32) -> Option<&'static Part> {
    parts.iter().find(|p| matches!(p.bind, Bind::PciClass { class: c, mask } if class & mask == c & mask))
}

/// How a registered driver finds its device.
#[derive(Clone, Copy)]
pub enum How {
    /// From its own BINDS, on the PCI buses (`pci_part`).
    Pci(&'static [Part]),
    /// Inside the xHCI driver's descriptor walk, as the device is enumerated (the USB function drivers:
    /// not yet moved onto BINDS, so they give no `class` verdict).
    UsbWalk,
    /// At a fixed place the CPU or the firmware gives (legacy ports, the firmware's framebuffer, the CPU's
    /// own timer and interrupt controller, memory): no bus id.
    Platform,
}

/// Every driver the PC kernel can register (`devices`), and how it finds its device. `hwscan` checks
/// that every registered driver is here.
pub const DRIVERS: &[(&str, How)] = &[
    ("nvme", How::Pci(crate::nvme::BINDS)),
    ("ahci-sata", How::Pci(crate::ahci::BINDS)),
    ("ide-disk", How::Pci(crate::ide::BINDS)),
    ("intel-e1000", How::Pci(crate::e1000::BINDS)),
    ("realtek-nic", How::Pci(crate::rtl8169::BINDS)),
    ("sd-card", How::Pci(crate::sdhci::BINDS)),
    ("intel-hda", How::Pci(crate::hda::BINDS)),
    ("usb-host (xhci)", How::Pci(crate::xhci::BINDS)),
    ("virtio", How::Pci(VIRTIO_BINDS)),
    ("usb-storage", How::UsbWalk),
    ("usb-ethernet", How::UsbWalk),
    ("usb-keyboard", How::UsbWalk),
    ("usb-mouse", How::UsbWalk),
    ("usb-tablet", How::UsbWalk),
    ("usb-audio", How::UsbWalk),
    ("usb-serial", How::UsbWalk),
    ("ps2-keyboard", How::Platform),
    ("ps2-mouse", How::Platform),
    ("16550-uart", How::Platform),
    ("fw-cfg", How::Platform),
    ("local-apic", How::Platform),
    ("tsc-timer", How::Platform),
    ("cmos-rtc", How::Platform),
    ("rdrand", How::Platform),
    ("gop-display", How::Platform),
    ("ramfs", How::Platform),
];

/// The virtio driver's match table. Here and not in pc/virtio_pci.rs because that file is a private module
/// of the shared virtio.rs (an ARM file this branch does not touch); its probe decides from this table.
pub const VIRTIO_BINDS: &[Part] = &[pci_ids(0x1AF4, 0x1000, 0xFF80, "virtio (transitional 0x1000-0x103f, modern 0x1040-0x107f)")];

/// The PCI functions the virtio driver took (it records them as it probes), for `hwscan`.
static mut VIRTIO_AT: [(u8, u8, u8); 8] = [(0, 0, 0); 8];
static mut NVIRTIO: usize = 0;
pub fn virtio_took(b: u8, d: u8, f: u8) {
    // SAFETY: boot, single core.
    unsafe {
        if NVIRTIO < 8 {
            VIRTIO_AT[NVIRTIO] = (b, d, f);
            NVIRTIO += 1;
        }
    }
}
pub fn virtio_at() -> ([(u8, u8, u8); 8], usize) {
    // SAFETY: plain copies of boot-time state.
    unsafe { (VIRTIO_AT, NVIRTIO) }
}

/// The DRIVERS row of a registered driver's name (every `virtio-*` name is the virtio driver's).
pub fn how_of(name: &str) -> Option<How> {
    let key = if name.starts_with("virtio-") { "virtio" } else { name };
    DRIVERS.iter().find(|r| r.0 == key).map(|r| r.1)
}

// ---------------------------------------------------------------------------------------------------
// Firmware a chip needs before any driver can run it.

#[derive(Clone, Copy, PartialEq)]
pub enum Fw {
    None,
    Open,
    Closed,
}

pub fn fw_line(f: Fw) -> &'static str {
    match f {
        Fw::None => "firmware: none needed",
        Fw::Open => "firmware: open source, may ship with AfriSovera",
        Fw::Closed => "firmware: a closed file, from this PC's own copy or a copy whose licence allows it",
    }
}

// ---------------------------------------------------------------------------------------------------
// Wi-Fi (moved from wifi.rs, 12 Nov 2026; unchanged).

/// The Linux driver family a Wi-Fi chip needs, and what that means for AfriSovera.
#[derive(Clone, Copy)]
pub enum Fam {
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

pub fn fam_line(f: Fam) -> &'static str {
    match f {
        Fam::Iwl => "Linux driver iwlwifi; needs Intel's closed firmware file loaded first",
        Fam::Rtw88 => "Linux driver rtw88; needs Realtek's firmware file loaded first",
        Fam::Rtw89 => "Linux driver rtw89; needs Realtek's firmware file loaded first",
        Fam::Rtlwifi => "Linux driver rtlwifi; needs Realtek's firmware file loaded first",
        Fam::Mt76 => "Linux driver mt76 (mt7921e); needs MediaTek's firmware files loaded first",
        Fam::Ath9k => "Linux driver ath9k; needs NO firmware file; the easiest chip to write a first Wi-Fi driver for",
        Fam::Ath10k => "Linux driver ath10k; needs Qualcomm's firmware files loaded first",
        Fam::Ath11k => "Linux driver ath11k; needs Qualcomm's firmware files loaded first",
        Fam::Brcm => "Broadcom; needs Broadcom's firmware (often only a closed driver exists)",
    }
}

pub fn fam_fw(f: Fam) -> Fw {
    match f {
        Fam::Ath9k => Fw::None,
        _ => Fw::Closed,
    }
}

/// Known Wi-Fi chips: (vendor, device, name, family).
pub const CHIPS: &[(u16, u16, &str, Fam)] = &[
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
pub enum UFam {
    AthHtc,
    Rt2800,
    Mt7601,
    Mt76u,
    Rtl8xxxu,
    Rtw88u,
}

pub fn ufam_line(f: UFam) -> &'static str {
    match f {
        UFam::AthHtc => "Linux driver ath9k_htc; its firmware file is OPEN source (free to ship); the easiest USB stick to write a first Wi-Fi driver for",
        UFam::Rt2800 => "Linux driver rt2800usb; needs Ralink's firmware file (free to redistribute)",
        UFam::Mt7601 => "Linux driver mt7601u; needs MediaTek's firmware file loaded first",
        UFam::Mt76u => "Linux driver mt76 (USB); needs MediaTek's firmware files loaded first",
        UFam::Rtl8xxxu => "Linux driver rtl8xxxu; needs Realtek's firmware file loaded first",
        UFam::Rtw88u => "Linux driver rtw88 (USB); needs Realtek's firmware file loaded first",
    }
}

pub fn ufam_fw(f: UFam) -> Fw {
    match f {
        UFam::AthHtc => Fw::Open,
        _ => Fw::Closed,
    }
}

/// Known USB Wi-Fi sticks: (vendor, product, name, family).
pub const STICKS: &[(u16, u16, &str, UFam)] = &[
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

pub fn stick(v: u16, p: u16) -> Option<(&'static str, UFam)> {
    STICKS.iter().find(|c| c.0 == v && c.1 == p).map(|c| (c.2, c.3))
}

pub fn chip(v: u16, d: u16) -> Option<(&'static str, Fam)> {
    CHIPS.iter().find(|c| c.0 == v && c.1 == d).map(|c| (c.2, c.3))
}

// ---------------------------------------------------------------------------------------------------
// PCI class codes in words, and the ones with nothing to drive.

/// A PCI class (base class << 8 | subclass) in words.
pub fn pci_class_name(c: u32) -> &'static str {
    match c {
        0x0100 => "SCSI storage",
        0x0101 => "IDE storage",
        0x0104 => "RAID storage",
        0x0106 => "SATA storage",
        0x0107 => "SAS storage",
        0x0108 => "NVMe storage",
        0x0180 => "other storage",
        0x0200 => "Ethernet",
        0x0280 => "network (Wi-Fi or other)",
        0x0300 => "VGA display",
        0x0302 => "3D display",
        0x0380 => "display",
        0x0400 => "video",
        0x0401 => "audio (old)",
        0x0403 => "HD Audio",
        0x0480 => "multimedia",
        0x0500 => "memory controller",
        0x0580 => "memory controller",
        0x0600 => "host bridge",
        0x0601 => "ISA/LPC bridge",
        0x0604 => "PCI bridge",
        0x0680 => "bridge",
        0x0700 => "serial port",
        0x0780 => "communication (e.g. Intel ME)",
        0x0800 => "interrupt controller",
        0x0805 => "SD host controller",
        0x0880 => "system peripheral",
        0x0C03 => "USB controller",
        0x0C05 => "SMBus",
        0x0C80 => "serial bus",
        0x0D11 => "Bluetooth",
        0x0D80 => "wireless",
        0x1080 => "encryption",
        0x1180 => "signal processing (thermal)",
        0x1200 => "processing accelerator",
        0x1300 => "instrumentation",
        _ => "other",
    }
}

/// Chipset parts that have nothing for an OS to drive (bridges the firmware set up, SMBus, thermal and
/// management functions, memory controllers).
pub fn pci_none_needed(c: u32) -> bool {
    matches!(c, 0x0500 | 0x0580 | 0x0600 | 0x0601 | 0x0604 | 0x0680 | 0x0800 | 0x0880 | 0x0C05 | 0x1180)
}

// ---------------------------------------------------------------------------------------------------
// ACPI device ids (_HID/_CID) in words, and what AfriSovera does with each.

/// What a driver says about an ACPI device it owns (`AcpiV::Asks`, 14 Nov 2026, architect: hwdb asks the owning
/// driver, it holds no fixed answer). The same hook serves every ACPI device a driver claims: the TPM driver for
/// MSFT0101 first, then battery.rs for the battery, AC adapter, lid and embedded controller.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Claim {
    /// Driven by the driver of this name.
    Driven(&'static str),
    /// The driver knows the device but leaves it off, for this reason (a start method it does not drive, ...).
    Off(&'static str),
    /// No driver has it.
    Missing,
}

/// A hook's answer as hwscan words it: (kind, detail) with kind "driven by", "off" or "missing".
pub fn claim_words(c: Claim) -> (&'static str, &'static str) {
    match c {
        Claim::Driven(d) => ("driven by", d),
        Claim::Off(why) => ("off", why),
        Claim::Missing => ("missing", "no driver in AfriSovera yet"),
    }
}

#[derive(Clone, Copy)]
pub enum AcpiV {
    /// A driver of ours that finds this device at its fixed place (the registered name).
    Ours(&'static str),
    /// Nothing to drive (the firmware's own bookkeeping, or handled another way).
    NoneNeeded,
    /// Not driven yet.
    Missing,
    /// The owning driver says (its hook is asked each time hwscan runs).
    Asks(fn() -> Claim),
}

/// Known ACPI ids: (id, name, verdict). Ids are matched in full.
pub const ACPI_IDS: &[(&str, &str, AcpiV)] = &[
    ("PNP0303", "PS/2 keyboard", AcpiV::Ours("ps2-keyboard")),
    ("PNP030B", "PS/2 keyboard", AcpiV::Ours("ps2-keyboard")),
    ("PNP0320", "PS/2 keyboard (Japanese)", AcpiV::Ours("ps2-keyboard")),
    ("PNP0F03", "PS/2 mouse", AcpiV::Ours("ps2-mouse")),
    ("PNP0F0E", "PS/2 mouse", AcpiV::Ours("ps2-mouse")),
    ("PNP0F13", "PS/2 mouse or touchpad", AcpiV::Ours("ps2-mouse")),
    ("PNP0B00", "real-time clock", AcpiV::Ours("cmos-rtc")),
    ("PNP0B01", "real-time clock", AcpiV::Ours("cmos-rtc")),
    ("PNP0B02", "real-time clock", AcpiV::Ours("cmos-rtc")),
    ("PNP0501", "serial port (16550)", AcpiV::Ours("16550-uart")),
    ("QEMU0002", "QEMU firmware configuration", AcpiV::Ours("fw-cfg")),
    ("PNP0A03", "PCI root bridge", AcpiV::NoneNeeded),
    ("PNP0A08", "PCI Express root bridge", AcpiV::NoneNeeded),
    ("PNP0C01", "system board", AcpiV::NoneNeeded),
    ("PNP0C02", "motherboard resources", AcpiV::NoneNeeded),
    ("PNP0C04", "maths coprocessor", AcpiV::NoneNeeded),
    ("PNP0C0F", "PCI interrupt link", AcpiV::NoneNeeded),
    ("PNP0000", "interrupt controller (8259)", AcpiV::NoneNeeded),
    ("PNP0100", "system timer (8254)", AcpiV::NoneNeeded),
    ("PNP0200", "DMA controller (8237)", AcpiV::NoneNeeded),
    ("PNP0800", "PC speaker", AcpiV::NoneNeeded),
    ("PNP0103", "HPET timer (the CPU's own timer is used)", AcpiV::NoneNeeded),
    ("PNP0C80", "memory device", AcpiV::NoneNeeded),
    ("PNP0A05", "generic container", AcpiV::NoneNeeded),
    ("PNP0A06", "generic container", AcpiV::NoneNeeded),
    ("ACPI0004", "module device", AcpiV::NoneNeeded),
    ("ACPI0006", "GPE block", AcpiV::NoneNeeded),
    ("ACPI0007", "processor", AcpiV::NoneNeeded),
    ("ACPI0010", "processor container", AcpiV::NoneNeeded),
    ("ACPI0013", "generic event device", AcpiV::NoneNeeded),
    ("INT0800", "firmware hub", AcpiV::NoneNeeded),
    ("INT3400", "Intel thermal framework (DPTF)", AcpiV::NoneNeeded),
    ("INT3403", "Intel thermal sensor (DPTF)", AcpiV::NoneNeeded),
    ("PNP0C0C", "power button (ACPI control method)", AcpiV::Missing),
    ("PNP0C0E", "sleep button", AcpiV::Missing),
    ("PNP0C0D", "laptop lid", AcpiV::Missing),
    ("PNP0C0A", "battery", AcpiV::Missing),
    ("ACPI0003", "AC adapter", AcpiV::Missing),
    ("PNP0C09", "embedded controller", AcpiV::Missing),
    ("PNP0C14", "WMI (the maker's own controls)", AcpiV::Missing),
    ("ACPI0008", "ambient light sensor", AcpiV::Missing),
    ("PNP0C50", "HID over I2C (touchpad, touchscreen or keyboard)", AcpiV::Missing),
    ("ACPI0C50", "HID over I2C (touchpad, touchscreen or keyboard)", AcpiV::Missing),
    ("MSFT0101", "TPM 2.0 security chip", AcpiV::Missing),
    ("MSFT0001", "keyboard (Microsoft id)", AcpiV::Ours("ps2-keyboard")),
    ("PNP0700", "floppy controller", AcpiV::Missing),
    ("PNP0400", "printer port", AcpiV::Missing),
    ("PNP0401", "printer port (ECP)", AcpiV::Missing),
    ("INT33C2", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("INT33C3", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("INT3432", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("INT3433", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("80860F41", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("808622C1", "Intel I2C controller (LPSS)", AcpiV::Missing),
    ("AMDI0010", "AMD I2C controller", AcpiV::Missing),
    ("INT33D5", "Intel HID event filter (hotkeys)", AcpiV::Missing),
    ("INT33D6", "Intel virtual buttons", AcpiV::Missing),
    ("BCM2E7B", "Broadcom Bluetooth (serial)", AcpiV::Missing),
];

impl AcpiV {
    /// The hook's answer (`Asks`), or the fixed verdict said the same way.
    pub fn ask(self) -> Claim {
        match self {
            AcpiV::Ours(d) => Claim::Driven(d),
            AcpiV::NoneNeeded => Claim::Off("none needed"),
            AcpiV::Missing => Claim::Missing,
            AcpiV::Asks(f) => f(),
        }
    }
}

pub fn acpi_id(id: &[u8]) -> Option<(&'static str, &'static str, AcpiV)> {
    ACPI_IDS.iter().find(|e| e.0.as_bytes() == id).copied()
}
