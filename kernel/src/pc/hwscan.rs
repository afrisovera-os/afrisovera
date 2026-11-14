//! `hwscan` (12 Nov 2026): the PC's hardware inventory, READ ONLY. The driver engineer's first step
//! (DRIVER_PLAN.md, architect's OK with conditions A1-A5, 12 Nov 2026).
//!
//! It lists every PCI function, every USB device the USB host driver knows, the ACPI tables and the
//! devices the ACPI tables name, and marks each one:
//!   driven       an AfriSovera driver runs it now;
//!   class        a driver's match table (Asili v0 BINDS, pc/hwdb.rs) covers it, but it is not in use now
//!                (another one is, it did not start, or it sits past the buses the drivers walk);
//!   missing      no driver yet; Wi-Fi lines name the chip family and the firmware it would need;
//!   none needed  chipset parts with nothing for an OS to drive.
//! `hwscan <id>` gives the verdict for one id as Windows' Device Manager shows it, so a PC can be
//! checked before AfriSovera is ever started on it.
//!
//! Read only (A5): PCI configuration reads, ACPI table reads, and the USB host driver's own records; no
//! device is enabled, no BAR is sized, no USB command is sent. No serial numbers or MAC addresses are
//! printed (A3): ids and classes only.
//!
//! ACPI (A2): there is no AML interpreter. The DSDT and SSDTs are scanned byte by byte for
//! `Name(_HID, ...)` and `Name(_CID, ...)` (an EISA id or a string, or a package of them for _CID) and
//! `Device(XXXX)` for the name. Every table is at most its readable (mapped) size and at most 1 MiB, at
//! most 64 SSDTs are read, every read is inside the table, a table with a bad checksum is listed and not
//! scanned, and one bad table never stops the rest. A byte scan can see a "_HID" inside other data (a false
//! line), and a device the firmware switches off (_STA) is still listed; `hwscan help` says so.
//! `hwscan selftest` runs the table checks and the scanner on broken tables built in memory.

use crate::hwdb::{self, AcpiV, Claim, How};
use crate::pci::cfg_read32 as cfg32;

const MAX_TABLE: usize = 1 << 20;
const MAX_SSDT: usize = 64;

// ------------------------------------------------------------------------------------------------
// Where ACPI tables may be (architect's MMIO note, 12 Nov 2026): a table address from the RSDT/XSDT or the
// FADT is read only when it lies inside memory the UEFI map calls reserved (type 0), ACPI reclaim (9) or
// ACPI NVS (10), and never past the end of that region. Any other address (device registers, RAM the kernel
// uses) is refused, so a broken firmware pointer cannot make hwscan read a device. Noted from main.rs while
// it walks the map, before ExitBootServices; with no map noted, every address is refused (fail closed).

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AOrd};

const MAX_FW: usize = 32;
static FW_START: [AtomicU64; MAX_FW] = [const { AtomicU64::new(0) }; MAX_FW];
static FW_END: [AtomicU64; MAX_FW] = [const { AtomicU64::new(0) }; MAX_FW];
static FW_N: AtomicUsize = AtomicUsize::new(0);
static FW_OVER: AtomicUsize = AtomicUsize::new(0);

/// One UEFI memory map descriptor (main.rs, before ExitBootServices): keep it if ACPI tables may live there.
pub fn note_region(start: u64, pages: u64, ty: u32) {
    if !matches!(ty, 0 | 9 | 10) || pages == 0 {
        return;
    }
    let Some(end) = pages.checked_mul(4096).and_then(|b| start.checked_add(b)) else { return };
    let n = FW_N.load(AOrd::Relaxed);
    // the map is sorted on most firmware: a region that starts where the last one ended joins it
    if n > 0 && FW_END[n - 1].load(AOrd::Relaxed) == start {
        FW_END[n - 1].store(end, AOrd::Relaxed);
        return;
    }
    if n == MAX_FW {
        FW_OVER.fetch_add(1, AOrd::Relaxed);
        return;
    }
    FW_START[n].store(start, AOrd::Relaxed);
    FW_END[n].store(end, AOrd::Relaxed);
    FW_N.store(n + 1, AOrd::Relaxed);
}

/// The end of the noted ACPI or reserved region that holds address `a`, if one does.
fn fw_end(a: u64) -> Option<u64> {
    (0..FW_N.load(AOrd::Relaxed).min(MAX_FW))
        .map(|i| (FW_START[i].load(AOrd::Relaxed), FW_END[i].load(AOrd::Relaxed)))
        .find(|&(s, e)| a >= s && a < e)
        .map(|(_, e)| e)
}
const MAX_ROOT: usize = 256;

#[derive(Default)]
struct Counts {
    driven: u32,
    class: u32,
    missing: u32,
    none: u32,
}

impl Counts {
    fn total(&self) -> u32 {
        self.driven + self.class + self.missing + self.none
    }
}

/// The driver that took PCI function b:d.f, and the name its BINDS gives that function.
fn took(b: u8, d: u8, f: u8, id: u32) -> Option<(&'static str, &'static str)> {
    let at = (b, d, f);
    let single: [(&'static str, Option<(u8, u8, u8)>, &'static [hwdb::Part]); 6] = [
        ("nvme", crate::nvme::at(), crate::nvme::BINDS),
        ("ahci-sata", crate::ahci::at(), crate::ahci::BINDS),
        ("intel-e1000", crate::e1000::at(), crate::e1000::BINDS),
        ("realtek-nic", crate::rtl8169::at(), crate::rtl8169::BINDS),
        ("sd-card", crate::sdhci::at(), crate::sdhci::BINDS),
        ("intel-hda", crate::hda::at(), crate::hda::BINDS),
    ];
    let name = |parts: &'static [hwdb::Part]| hwdb::pci_part(parts, b, d, f, id).map(|p| p.name).unwrap_or("");
    for (drv, a, parts) in single {
        if a == Some(at) {
            return Some((drv, name(parts)));
        }
    }
    let (ide, n) = crate::ide::controllers();
    if ide[..n].contains(&at) {
        return Some(("ide-disk", name(crate::ide::BINDS)));
    }
    let (x, n) = crate::xhci::controllers();
    if x[..n].contains(&at) {
        return Some(("usb-host (xhci)", name(crate::xhci::BINDS)));
    }
    let (v, n) = hwdb::virtio_at();
    if v[..n].contains(&at) {
        return Some(("virtio", name(hwdb::VIRTIO_BINDS)));
    }
    None
}

/// The first driver whose BINDS take PCI function b:d.f.
fn covers(b: u8, d: u8, f: u8, id: u32) -> Option<(&'static str, &'static str)> {
    for &(drv, how) in hwdb::DRIVERS {
        if let How::Pci(parts) = how {
            if let Some(p) = hwdb::pci_part(parts, b, d, f, id) {
                return Some((drv, p.name));
            }
        }
    }
    None
}

fn scan_pci(c: &mut Counts) {
    crate::kprintln!("hwscan: PCI (every bus, read only)");
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
                let class = cfg32(b, d, f, 0x08) >> 16;
                crate::kprint!("  pci {:02x}:{:02x}.{} {:04x}:{:04x} {}: ", b, d, f, v, dv, hwdb::pci_class_name(class));
                if let Some((drv, part)) = took(b, d, f, id) {
                    c.driven += 1;
                    crate::kprintln!("driven by {} ({})", drv, part);
                } else if let Some((drv, part)) = covers(b, d, f, id) {
                    c.class += 1;
                    crate::kprintln!("class: {} covers it ({}), not in use now", drv, part);
                } else if let Some((name, fam)) = hwdb::chip(v, dv) {
                    c.missing += 1;
                    crate::kprintln!("missing: Wi-Fi {}; {}; {}", name, fam_short(fam), hwdb::fw_line(hwdb::fam_fw(fam)));
                } else if hwdb::pci_none_needed(class) {
                    c.none += 1;
                    crate::kprintln!("none needed (chipset part)");
                } else if class >> 8 == 0x03 {
                    c.missing += 1;
                    crate::kprintln!("missing: no graphics driver; the screen runs on the firmware's framebuffer");
                } else {
                    c.missing += 1;
                    crate::kprintln!("missing: no driver in AfriSovera yet");
                }
            }
        }
    }
}

/// The driver family of a Wi-Fi chip, short (the family's whole line is `wifi`'s).
fn fam_short(f: hwdb::Fam) -> &'static str {
    use hwdb::Fam::*;
    match f {
        Iwl => "family iwlwifi",
        Rtw88 => "family rtw88",
        Rtw89 => "family rtw89",
        Rtlwifi => "family rtlwifi",
        Mt76 => "family mt76",
        Ath9k => "family ath9k",
        Ath10k => "family ath10k",
        Ath11k => "family ath11k",
        Brcm => "Broadcom",
    }
}

fn scan_usb(c: &mut Counts) {
    let (on, others, no) = crate::xhci::others();
    if !on {
        crate::kprintln!("hwscan: USB: no USB controller in use, so no USB device looked at");
        return;
    }
    crate::kprintln!("hwscan: USB (the devices the USB host driver has seen since power-on)");
    let (used, nu) = crate::xhci::used();
    for &(v, p, what) in &used[..nu] {
        c.driven += 1;
        crate::kprintln!("  usb {:04x}:{:04x} {}: driven", v, p, what);
    }
    for &(v, p, dc, ic) in &others[..no] {
        c.missing += 1;
        if let Some((name, fam)) = hwdb::stick(v, p) {
            crate::kprintln!("  usb {:04x}:{:04x} Wi-Fi stick: missing: {}; {}", v, p, name, hwdb::fw_line(hwdb::ufam_fw(fam)));
        } else {
            let cl = if dc != 0 { dc } else { ic };
            crate::kprintln!("  usb {:04x}:{:04x} {}: missing: no driver in AfriSovera yet", v, p, crate::xhci::class_name(cl));
        }
    }
}

// ------------------------------------------------------------------------------------------------
// ACPI tables (A2: every length capped, every read inside the table)

/// Why a table is not scanned.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Bad {
    NoHeader,
    Short,
    Huge,
    PastEnd,
    Checksum,
    NotAcpi,
}

fn bad_line(b: Bad) -> &'static str {
    match b {
        Bad::NoHeader => "its header is not readable: not scanned",
        Bad::Short => "its length is shorter than its header: not scanned",
        Bad::Huge => "its length is over 1 MiB: not scanned",
        Bad::PastEnd => "its length runs past the readable memory: not scanned",
        Bad::Checksum => "bad checksum: not scanned",
        Bad::NotAcpi => "it is not in ACPI or reserved memory (UEFI memory map): not read",
    }
}

fn u32_at(t: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(t.get(o..o.checked_add(4)?)?.try_into().ok()?))
}
fn u64_at(t: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(t.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// The table in `avail` (the bytes that may be read from its start), checked: header readable, length
/// at least the header, at most 1 MiB and inside `avail`, checksum zero.
fn table_of(avail: &[u8]) -> Result<&[u8], Bad> {
    let len = u32_at(avail, 4).filter(|_| avail.len() >= 36).ok_or(Bad::NoHeader)? as usize;
    if len < 36 {
        return Err(Bad::Short);
    }
    if len > MAX_TABLE {
        return Err(Bad::Huge);
    }
    let t = avail.get(..len).ok_or(Bad::PastEnd)?;
    if t.iter().fold(0u8, |s, &x| s.wrapping_add(x)) != 0 {
        return Err(Bad::Checksum);
    }
    Ok(t)
}

/// The bytes at physical address `a` that the kernel's 1:1 map lets us read, at most 1 MiB and never past the
/// end of the ACPI or reserved region that holds `a`; an address in no such region is refused (NotAcpi).
fn readable(a: u64) -> Result<&'static [u8], Bad> {
    if a == 0 {
        return Ok(&[]);
    }
    let cap = fw_end(a).ok_or(Bad::NotAcpi)?.saturating_sub(a).min(MAX_TABLE as u64);
    // a firmware pointer can be anything: every add is checked, and the walk stops before the top page of the
    // address space (a slice may not reach the end of it)
    let mut n = 0u64;
    while n < cap {
        let Some(page) = a.checked_add(n).map(|p| p & !0xFFF) else { break };
        if crate::mmu::translate(page) != Some(page) {
            break;
        }
        let Some(next) = page.checked_add(0x1000) else { break };
        n = next - a;
    }
    let n = n.min(cap) as usize;
    // SAFETY: every page of [a, a + n) is present in the kernel's 1:1 map (checked above) and inside one
    // region the UEFI map calls reserved or ACPI memory, which the kernel never reuses; it is only read.
    Ok(unsafe { core::slice::from_raw_parts(a as *const u8, n) })
}

/// The ACPI global lock's word (FACS offset 16) for battery.rs (AML C1): the FADT's FACS must lie in ACPI or
/// reserved memory in the kernel's 1:1 map, say "FACS" and be at least 64 bytes long; else why not.
pub fn facs_lock() -> Result<u64, &'static str> {
    let a = crate::acpi::facs();
    if a == 0 {
        return Err("the FADT names no FACS");
    }
    let t = readable(a).map_err(|_| "the FACS is not in ACPI memory")?;
    if t.len() < 64 || &t[..4] != b"FACS" || u32_at(t, 4).unwrap_or(0) < 64 || a % 8 != 0 {
        return Err("the FACS is not a FACS");
    }
    Ok(a + 16)
}

/// The DSDT and then every SSDT the RSDT or XSDT lists (at most MAX_SSDT), each checked as `hwscan` checks it
/// (inside ACPI or reserved memory, length, checksum, signature), handed to `f`: for battery.rs's structural AML
/// reader (13 Nov 2026). Returns whether the DSDT was read, and how many SSDTs were.
pub fn aml_tables(f: &mut dyn FnMut(&'static [u8])) -> (bool, usize) {
    let Some((root, wide)) = crate::acpi::root() else { return (false, 0) };
    let Ok(rt) = readable(root).and_then(table_of) else { return (false, 0) };
    let step = if wide { 8 } else { 4 };
    let mut dsdt = 0u64;
    let mut ssdt = [0u64; MAX_SSDT];
    let mut nssdt = 0;
    for e in 0..((rt.len() - 36) / step).min(MAX_ROOT) {
        let o = 36 + e * step;
        let a = if wide { u64_at(rt, o) } else { u32_at(rt, o).map(|v| v as u64) }.unwrap_or(0);
        let Ok(t) = readable(a).and_then(table_of) else { continue };
        if &sig(t) == b"FACP" && dsdt == 0 {
            let x = if t.len() >= 148 { u64_at(t, 140).unwrap_or(0) } else { 0 };
            dsdt = if x != 0 { x } else { u32_at(t, 40).unwrap_or(0) as u64 };
        } else if &sig(t) == b"SSDT" && nssdt < MAX_SSDT {
            ssdt[nssdt] = a;
            nssdt += 1;
        }
    }
    let mut got = (false, 0);
    for (k, &a) in core::iter::once(&dsdt).chain(ssdt[..nssdt].iter()).enumerate() {
        let want: &[u8; 4] = if k == 0 { b"DSDT" } else { b"SSDT" };
        match readable(a).and_then(table_of) {
            Ok(t) if &sig(t) == want => {
                f(t);
                if k == 0 {
                    got.0 = true;
                } else {
                    got.1 += 1;
                }
            }
            _ => {}
        }
    }
    got
}

// ---- `acpisave` (AML rung C2, ARCHITECT_REVIEWS 14 Nov, "ACPI AML options", Q2) ----
// The firmware's own AML (the DSDT and SSDTs) and the FADT, saved to the USB stick AfriSovera started from through
// report save's one door (esp::acpi_save), so the team can read what a real laptop's _BST, _PSR and _LID do before
// any interpreter rung that writes is planned. A FIXED list: only DSDT, SSDT and FACP are ever saved. MSDM (it holds
// the Windows product key), SLIC and every other table are never read past their signature. The tables are the
// laptop maker's code: they live in the team's notes folder, never in git, a public test fixture or a cloud job.

/// The only tables `acpisave` saves.
const SAVE_SIGS: [&[u8; 4]; 3] = [b"DSDT", b"SSDT", b"FACP"];
/// The most bytes saved in all (a laptop's DSDT and SSDTs are a few hundred KiB).
const SAVE_CAP: usize = 1 << 20;

/// `acpisave`: DSDT, every SSDT and the FACP, each checked as hwscan checks a table (in ACPI or reserved memory,
/// its length, its checksum), put one after another after a title line, and written as \AFREPORT\<n>.BIN.
pub fn acpi_save() {
    let Some((root, wide)) = crate::acpi::root() else {
        crate::kprintln!("acpisave: the firmware gave no ACPI root table; nothing saved");
        return;
    };
    let Ok(rt) = readable(root).and_then(table_of) else {
        crate::kprintln!("acpisave: the ACPI root table did not check out; nothing saved");
        return;
    };
    let mut out: alloc::vec::Vec<u8> = alloc::vec::Vec::new();
    out.extend_from_slice(b"AFRISOVERA ACPI TABLES 1\n");
    let (mut dsdt_at, mut ssdts, mut facp, mut left_out, mut bad, mut too_big) = (0u64, 0u32, 0u32, 0u32, 0u32, 0u32);
    let mut put = |t: &[u8], out: &mut alloc::vec::Vec<u8>| -> bool {
        if out.len() + t.len() > SAVE_CAP {
            too_big += 1;
            return false;
        }
        out.extend_from_slice(t);
        true
    };
    let step = if wide { 8 } else { 4 };
    for e in 0..((rt.len() - 36) / step).min(MAX_ROOT) {
        let o = 36 + e * step;
        let a = if wide { u64_at(rt, o) } else { u32_at(rt, o).map(|v| v as u64) }.unwrap_or(0);
        // the signature first: a table not on the list is never read further (no checksum walk over it)
        let Ok(raw) = readable(a) else {
            bad += 1;
            continue;
        };
        let s = sig(raw);
        if !SAVE_SIGS.contains(&&s) || &s == b"DSDT" {
            left_out += 1;
            continue;
        }
        let Ok(t) = table_of(raw) else {
            bad += 1;
            continue;
        };
        if &s == b"FACP" {
            if dsdt_at == 0 {
                let x = if t.len() >= 148 { u64_at(t, 140).unwrap_or(0) } else { 0 };
                dsdt_at = if x != 0 { x } else { u32_at(t, 40).unwrap_or(0) as u64 };
            }
            if put(t, &mut out) {
                facp += 1;
            }
        } else if put(t, &mut out) {
            ssdts += 1;
        }
    }
    let mut dsdt_len = 0usize;
    match readable(dsdt_at) {
        Ok(raw) if dsdt_at != 0 && &sig(raw) == b"DSDT" => match table_of(raw) {
            Ok(t) => {
                if put(t, &mut out) {
                    dsdt_len = t.len();
                }
            }
            Err(_) => bad += 1,
        },
        _ => bad += 1,
    }
    crate::kprintln!(
        "acpisave: DSDT {}, {} SSDT(s), {} FACP, {} bytes in all; {} other table(s) left out by the fixed list (never saved: MSDM, SLIC and the rest); {} that did not check out",
        if dsdt_len > 0 { "kept" } else { "missing" }, ssdts, facp, out.len(), left_out, bad
    );
    if too_big > 0 {
        crate::kprintln!("acpisave: {} table(s) left out: past the 1 MiB this saves in all", too_big);
    }
    if dsdt_len == 0 && ssdts == 0 {
        crate::kprintln!("acpisave: no DSDT or SSDT to save; nothing written");
        return;
    }
    crate::kprintln!("acpisave: these are the laptop maker's tables: keep them in the team's notes folder, never in git or a cloud job");
    crate::esp::acpi_save(&out);
}

fn sig(t: &[u8]) -> [u8; 4] {
    let mut s = [b'?'; 4];
    for (o, &c) in s.iter_mut().zip(t.iter().take(4)) {
        *o = if c.is_ascii_graphic() { c } else { b'?' };
    }
    s
}

fn txt(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("?")
}

/// ACPI ids found by the scan: (id, its length, times seen, the device it was first seen under).
const MAX_IDS: usize = 96;
struct Ids {
    id: [[u8; 9]; MAX_IDS],
    len: [u8; MAX_IDS],
    count: [u16; MAX_IDS],
    dev: [[u8; 4]; MAX_IDS],
    n: usize,
    cut: u32,    // a _HID or _CID whose value runs past the table's end
    method: u32, // a _HID given by a method (not read without an AML interpreter)
    full: u32,   // ids not kept because the list was full
}

impl Ids {
    const fn new() -> Self {
        Ids { id: [[0; 9]; MAX_IDS], len: [0; MAX_IDS], count: [0; MAX_IDS], dev: [[b'?'; 4]; MAX_IDS], n: 0, cut: 0, method: 0, full: 0 }
    }
    fn add(&mut self, id: &[u8], dev: [u8; 4]) {
        if let Some(k) = (0..self.n).find(|&k| &self.id[k][..self.len[k] as usize] == id) {
            self.count[k] = self.count[k].saturating_add(1);
            return;
        }
        if self.n == MAX_IDS || id.len() > 9 || id.is_empty() {
            self.full += 1;
            return;
        }
        self.id[self.n][..id.len()].copy_from_slice(id);
        self.len[self.n] = id.len() as u8;
        self.count[self.n] = 1;
        self.dev[self.n] = dev;
        self.n += 1;
    }
}

/// An AML PkgLength at `i`: (the length, the bytes it takes).
fn pkg_len(t: &[u8], i: usize) -> Option<(usize, usize)> {
    let lead = *t.get(i)?;
    let extra = (lead >> 6) as usize;
    if extra == 0 {
        return Some(((lead & 0x3F) as usize, 1));
    }
    let mut len = (lead & 0x0F) as usize;
    for k in 0..extra {
        len |= (*t.get(i + 1 + k)? as usize) << (4 + 8 * k);
    }
    Some((len, 1 + extra))
}

/// The last 4-character segment of an AML NameString at `i`.
fn name_seg(t: &[u8], mut i: usize) -> Option<[u8; 4]> {
    while matches!(t.get(i), Some(b'\\') | Some(b'^')) {
        i += 1;
    }
    let segs = match *t.get(i)? {
        0x2E => {
            i += 1;
            2
        }
        0x2F => {
            i += 2;
            *t.get(i - 1)? as usize
        }
        0x00 => return None,
        _ => 1,
    };
    let last = i.checked_add(4 * segs.checked_sub(1)?)?;
    let s: [u8; 4] = t.get(last..last + 4)?.try_into().ok()?;
    s.iter().all(|&c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_').then_some(s)
}

/// An EISA id (`PNP0303` from 0x41D00303 as it sits in memory).
fn eisa(b: [u8; 4]) -> [u8; 7] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let l = |v: u8| b'@' + (v & 0x1F);
    [
        l(b[0] >> 2),
        l((b[0] & 3) << 3 | b[1] >> 5),
        l(b[1]),
        HEX[(b[2] >> 4) as usize],
        HEX[(b[2] & 15) as usize],
        HEX[(b[3] >> 4) as usize],
        HEX[(b[3] & 15) as usize],
    ]
}

/// One id value (EisaId or string) at `j`, inside `t[..end]`: Some(Some(id)) read, Some(None) not an id
/// form, None cut off at the end.
fn id_value(t: &[u8], j: usize, end: usize, out: &mut [u8; 9]) -> Option<Option<usize>> {
    let t = t.get(..end)?;
    match *t.get(j)? {
        0x0C => {
            let b: [u8; 4] = t.get(j + 1..j + 5)?.try_into().ok()?;
            out[..7].copy_from_slice(&eisa(b));
            Some(Some(7))
        }
        0x0D => {
            let mut n = 0;
            loop {
                let c = *t.get(j + 1 + n)?;
                if c == 0 {
                    break;
                }
                if n == 9 || !(c.is_ascii_alphanumeric() || c == b'_') {
                    return Some(None); // not an id string
                }
                out[n] = c;
                n += 1;
            }
            Some(if n >= 4 { Some(n) } else { None })
        }
        _ => Some(None),
    }
}

/// Scan one table's AML (after its 36-byte header) for Device names and _HID/_CID ids.
fn scan_aml(t: &[u8], ids: &mut Ids) {
    let mut dev = [b'?'; 4];
    let mut i = 36;
    while i < t.len() {
        let op = t[i];
        if op == 0x5B && t.get(i + 1) == Some(&0x82) {
            // Device(Name): ExtOpPrefix DeviceOp PkgLength NameString
            if let Some((_, n)) = pkg_len(t, i + 2) {
                if let Some(s) = name_seg(t, i + 2 + n) {
                    dev = s;
                }
            }
        } else if op == 0x14 {
            // Method(_HID): MethodOp PkgLength NameString
            if let Some((_, n)) = pkg_len(t, i + 1) {
                if t.get(i + 1 + n..i + 5 + n) == Some(b"_HID") {
                    ids.method += 1;
                }
            }
        } else if op == 0x08 {
            let what = t.get(i + 1..i + 5);
            if what == Some(b"_HID") || what == Some(b"_CID") {
                let j = i + 5;
                let mut id = [0u8; 9];
                if t.get(j) == Some(&0x12) && what == Some(b"_CID") {
                    // Package(n) { id, id, ... }: PackageOp PkgLength NumElements elements
                    match pkg_len(t, j + 1) {
                        Some((plen, n)) => {
                            let end = (j + 1).saturating_add(plen).min(t.len());
                            let mut k = j + 1 + n + 1; // after NumElements
                            for _ in 0..8 {
                                match id_value(t, k, end, &mut id) {
                                    Some(Some(len)) => {
                                        ids.add(&id[..len], dev);
                                        k += if t[k] == 0x0C { 5 } else { len + 2 };
                                    }
                                    Some(None) => break,
                                    None => {
                                        if end == t.len() && k < t.len() {
                                            ids.cut += 1;
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                        None => ids.cut += 1,
                    }
                } else {
                    match id_value(t, j, t.len(), &mut id) {
                        Some(Some(len)) => ids.add(&id[..len], dev),
                        Some(None) => {}
                        None => ids.cut += 1,
                    }
                }
            }
        }
        i += 1;
    }
}

/// The ACPI tables and the devices they name.
fn scan_acpi(c: &mut Counts) {
    let over = FW_OVER.load(AOrd::Relaxed);
    if over > 0 {
        crate::kprintln!("hwscan: ACPI: {} reserved or ACPI memory region(s) past the first {} not noted; tables there are not read", over, MAX_FW);
    }
    let Some((root, wide)) = crate::acpi::root() else {
        crate::kprintln!("hwscan: ACPI: the firmware gave no usable RSDT or XSDT");
        return;
    };
    let rt = match readable(root).and_then(table_of) {
        Ok(t) => t,
        Err(b) => {
            crate::kprintln!("hwscan: ACPI: the {}: {}", if wide { "XSDT" } else { "RSDT" }, bad_line(b));
            return;
        }
    };
    let step = if wide { 8 } else { 4 };
    let entries = ((rt.len() - 36) / step).min(MAX_ROOT);
    crate::kprint!("hwscan: ACPI tables ({}):", if wide { "XSDT" } else { "RSDT" });
    let mut dsdt = 0u64;
    let mut ssdt = [0u64; MAX_SSDT];
    let (mut nssdt, mut over) = (0usize, 0u32);
    let mut bad = [(0u64, [0u8; 4], Bad::NoHeader); 8];
    let mut nbad = 0;
    for e in 0..entries {
        let o = 36 + e * step;
        let a = if wide { u64_at(rt, o) } else { u32_at(rt, o).map(|v| v as u64) }.unwrap_or(0);
        let avail = readable(a);
        let s = sig(avail.unwrap_or(&[]));
        crate::kprint!(" {}", txt(&s));
        match avail.and_then(table_of) {
            Ok(t) => {
                if &s == b"FACP" {
                    let x = if t.len() >= 148 { u64_at(t, 140).unwrap_or(0) } else { 0 };
                    dsdt = if x != 0 { x } else { u32_at(t, 40).unwrap_or(0) as u64 };
                } else if &s == b"SSDT" {
                    if nssdt < MAX_SSDT {
                        ssdt[nssdt] = a;
                        nssdt += 1;
                    } else {
                        over += 1;
                    }
                }
            }
            Err(b) => {
                if nbad < bad.len() {
                    bad[nbad] = (a, s, b);
                    nbad += 1;
                }
            }
        }
    }
    crate::kprintln!("");
    for &(a, s, b) in &bad[..nbad] {
        crate::kprintln!("  acpi table {} at {:#x}: {}", txt(&s), a, bad_line(b));
    }
    if over > 0 {
        crate::kprintln!("  acpi: {} SSDT(s) past the first {} not read", over, MAX_SSDT);
    }
    let mut ids = Ids::new();
    let mut scanned = 0;
    for (k, &a) in core::iter::once(&dsdt).chain(ssdt[..nssdt].iter()).enumerate() {
        if a == 0 {
            continue;
        }
        let name = if k == 0 { "DSDT" } else { "SSDT" };
        match readable(a).and_then(table_of) {
            Ok(t) if &sig(t) == name.as_bytes() => {
                scan_aml(t, &mut ids);
                scanned += 1;
            }
            Ok(t) => crate::kprintln!("  acpi table at {:#x} should be the {} but is {}: not scanned", a, name, txt(&sig(t))),
            Err(b) => crate::kprintln!("  acpi table {} at {:#x}: {}", name, a, bad_line(b)),
        }
    }
    crate::kprintln!("hwscan: ACPI devices (a byte scan of {} table(s): see hwscan help)", scanned);
    for k in 0..ids.n {
        let id = &ids.id[k][..ids.len[k] as usize];
        crate::kprint!("  acpi {}", txt(id));
        if ids.count[k] > 1 {
            crate::kprint!(" x{}", ids.count[k]);
        }
        crate::kprint!(" ({}): ", txt(&ids.dev[k]));
        match hwdb::acpi_id(id) {
            Some((_, name, AcpiV::Ours(drv))) => {
                if registered(drv) {
                    c.driven += 1;
                    crate::kprintln!("{}: driven by {}", name, drv);
                } else {
                    c.class += 1;
                    crate::kprintln!("{}: class: {} did not find it at its fixed place", name, drv);
                }
            }
            Some((_, name, AcpiV::NoneNeeded)) => {
                c.none += 1;
                crate::kprintln!("{}: none needed", name);
            }
            Some((_, name, AcpiV::Missing)) => {
                c.missing += 1;
                crate::kprintln!("{}: missing: no driver in AfriSovera yet", name);
            }
            Some((_, name, AcpiV::Asks(f))) => {
                let claim = f();
                match claim {
                    Claim::Driven(_) => c.driven += 1,
                    Claim::Off(_) => c.class += 1,
                    Claim::Missing => c.missing += 1,
                }
                let (kind, detail) = hwdb::claim_words(claim);
                crate::kprintln!("{}: {}: {}", name, kind, detail);
            }
            None => {
                c.missing += 1;
                crate::kprintln!("missing: not in AfriSovera's list; tell us this id");
            }
        }
    }
    if ids.method > 0 {
        crate::kprintln!("  acpi: {} device id(s) given by a method, not read (needs an AML interpreter)", ids.method);
    }
    if ids.cut > 0 {
        crate::kprintln!("  acpi: {} device id(s) cut off at a table's end, not read", ids.cut);
    }
    if ids.full > 0 {
        crate::kprintln!("  acpi: {} more id(s) not listed (the list holds {})", ids.full, MAX_IDS);
    }
}

fn registered(name: &str) -> bool {
    (0..crate::asili::count()).any(|i| crate::asili::get(i).is_some_and(|d| d.name == name))
}

/// The registered drivers without a row in the match tables (should be none).
fn unbound(say: bool) -> (u32, u32) {
    let (mut n, mut bad) = (0u32, 0u32);
    for i in 0..crate::asili::count() {
        let Some(d) = crate::asili::get(i) else { continue };
        n += 1;
        match hwdb::how_of(d.name) {
            None => {
                bad += 1;
                crate::kprintln!("hwscan: driver {} is registered but has no row in the match tables (pc/hwdb.rs)", d.name);
            }
            Some(h) if say => crate::kprintln!(
                "  driver {}: {}",
                d.name,
                match h {
                    How::Pci(_) => "PCI, its probe decides from its own BINDS",
                    How::UsbWalk => "USB, decided in the USB host driver's descriptor walk (not yet on BINDS: no class verdict)",
                    How::Platform => "fixed place (CPU, firmware or legacy port): no bus id",
                }
            ),
            Some(_) => {}
        }
    }
    (n, bad)
}

/// `hwscan`: the whole inventory.
pub fn show() {
    crate::kprintln!("hwscan: this PC's hardware (read only, nothing is changed; ids and classes only)");
    let mut c = Counts::default();
    scan_pci(&mut c);
    scan_usb(&mut c);
    scan_acpi(&mut c);
    let (n, bad) = unbound(false);
    crate::kprintln!("hwscan: {} registered driver(s), {} without a match table", n, bad);
    crate::kprintln!(
        "hwscan: {} device(s): {} driven, {} class, {} missing, {} none needed",
        c.total(), c.driven, c.class, c.missing, c.none
    );
    crate::kprintln!(
        "  for the owner: lines marked missing have no AfriSovera driver yet, with the firmware a Wi-Fi chip would need; send this list (report <ip>) so we write the right drivers first"
    );
}

fn help() {
    crate::kprintln!("hwscan             every PCI, USB and ACPI device of this PC, each marked driven, class, missing or none needed");
    crate::kprintln!("hwscan <id>        one device by its id: 8086:2723, PCI\\VEN_8086&DEV_2723&CC_0280, USB\\VID_0CF3&PID_9271, ACPI\\PNP0C50 or ELAN0001");
    crate::kprintln!("hwscan drivers     every registered driver and how it finds its device");
    crate::kprintln!("hwscan selftest    the ACPI table checks and scanner run on broken tables built in memory");
    crate::kprintln!("  read only: nothing is written, no device is switched on, no serial number or MAC address is shown");
    crate::kprintln!("  ACPI devices come from a byte scan of the ACPI tables (no AML interpreter): a line can be false (\"_HID\" inside other data), and a device the firmware switches off is still listed");
}

/// `hwscan <arg>`.
pub fn command(arg: &str) {
    match arg {
        "" => show(),
        "help" => help(),
        "selftest" => selftest(),
        "drivers" => {
            crate::kprintln!("hwscan: registered drivers and how each finds its device");
            let (n, bad) = unbound(true);
            crate::kprintln!("hwscan: {} registered driver(s), {} without a match table", n, bad);
        }
        _ => lookup(arg),
    }
}

fn upper(s: &str, buf: &mut [u8; 96]) -> usize {
    let n = s.len().min(buf.len());
    for (o, c) in buf.iter_mut().zip(s.bytes().take(n)) {
        *o = c.to_ascii_uppercase();
    }
    n
}

/// An ACPI id from `ACPI\PNP0C50`, `ACPI\VEN_ELAN&DEV_0001`, `ACPI\ELAN0001\...` or a bare `ELAN0001`.
fn acpi_arg<'a>(u: &'a str, out: &'a mut [u8; 9]) -> Option<&'a [u8]> {
    let rest = u.strip_prefix("ACPI\\").or_else(|| u.strip_prefix("ACPI:"));
    let body = rest.unwrap_or(u);
    if let (Some(a), Some(b)) = (body.find("VEN_"), body.find("DEV_")) {
        let v = body[a + 4..].split(['&', '\\']).next()?;
        let d = body[b + 4..].split(['&', '\\']).next()?;
        if v.len() + d.len() > 9 {
            return None;
        }
        out[..v.len()].copy_from_slice(v.as_bytes());
        out[v.len()..v.len() + d.len()].copy_from_slice(d.as_bytes());
        return Some(&out[..v.len() + d.len()]);
    }
    let id = body.split(['\\', '&']).next()?;
    if !(7..=8).contains(&id.len()) || !id.bytes().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    out[..id.len()].copy_from_slice(id.as_bytes());
    Some(&out[..id.len()])
}

/// `hwscan <id>`: one device, from an id as Windows' Device Manager shows it.
fn lookup(arg: &str) {
    let mut up = [0u8; 96];
    let n = upper(arg, &mut up);
    let Ok(u) = core::str::from_utf8(&up[..n]) else { return usage() };
    let mut aid = [0u8; 9];
    if u.starts_with("ACPI") || (!u.contains(':') && !u.contains("VEN_") && !u.contains("VID_")) {
        let Some(id) = acpi_arg(u, &mut aid) else { return usage() };
        match hwdb::acpi_id(id) {
            Some((_, name, AcpiV::Ours(drv))) => crate::kprintln!("hwscan: acpi {} is {}: {} drives it", txt(id), name, drv),
            Some((_, name, AcpiV::NoneNeeded)) => crate::kprintln!("hwscan: acpi {} is {}: none needed", txt(id), name),
            Some((_, name, AcpiV::Missing)) => crate::kprintln!("hwscan: acpi {} is {}: missing, no driver in AfriSovera yet", txt(id), name),
            Some((_, name, AcpiV::Asks(f))) => {
                let (kind, detail) = hwdb::claim_words(f());
                crate::kprintln!("hwscan: acpi {} is {}: {}: {}", txt(id), name, kind, detail)
            }
            None => crate::kprintln!("hwscan: acpi {} is not in AfriSovera's list; tell us this id", txt(id)),
        }
        return;
    }
    let Some((v, d, form)) = crate::wifi::parse(arg) else { return usage() };
    if form == Some(true) {
        match hwdb::stick(v, d) {
            Some((name, fam)) => crate::kprintln!("hwscan: usb {:04x}:{:04x} is {} (Wi-Fi stick): missing; {}", v, d, name, hwdb::fw_line(hwdb::ufam_fw(fam))),
            None => crate::kprintln!("hwscan: usb {:04x}:{:04x}: USB devices are matched by their class when plugged in; plug it in and run hwscan", v, d),
        }
        return;
    }
    // the class, when Device Manager's id carries it (CC_0C0330 or CC_0C03)
    let cc = u.find("CC_").and_then(|a| {
        let h = &u[a + 3..];
        let k = h.bytes().take_while(|c| c.is_ascii_hexdigit()).count();
        match k {
            k if k >= 6 => u32::from_str_radix(&h[..6], 16).ok(),
            4 | 5 => u32::from_str_radix(&h[..4], 16).ok().map(|c| c << 8),
            _ => None,
        }
    });
    for &(drv, how) in hwdb::DRIVERS {
        if let How::Pci(parts) = how {
            if let Some(p) = hwdb::id_part(parts, v, d) {
                return crate::kprintln!("hwscan: pci {:04x}:{:04x} is {}: {} drives it", v, d, p.name, drv);
            }
        }
    }
    if let Some(class) = cc {
        for &(drv, how) in hwdb::DRIVERS {
            if let How::Pci(parts) = how {
                if let Some(p) = hwdb::class_part(parts, class) {
                    return crate::kprintln!("hwscan: pci {:04x}:{:04x} is a {}: the class driver {} covers it ({})", v, d, hwdb::pci_class_name(class >> 8), drv, p.name);
                }
            }
        }
    }
    if let Some((name, fam)) = hwdb::chip(v, d) {
        return crate::kprintln!("hwscan: pci {:04x}:{:04x} is {} (Wi-Fi): missing; {}; {}", v, d, name, fam_short(fam), hwdb::fw_line(hwdb::fam_fw(fam)));
    }
    match cc {
        Some(class) if hwdb::pci_none_needed(class >> 8) => crate::kprintln!("hwscan: pci {:04x}:{:04x} is a {}: none needed (chipset part)", v, d, hwdb::pci_class_name(class >> 8)),
        Some(class) => crate::kprintln!("hwscan: pci {:04x}:{:04x} is a {}: missing, no driver in AfriSovera yet", v, d, hwdb::pci_class_name(class >> 8)),
        None => crate::kprintln!("hwscan: pci {:04x}:{:04x} is not in AfriSovera's lists; give its class too (Device Manager's Compatible Ids show CC_xxxx) or tell us this id", v, d),
    }
}

fn usage() {
    crate::kprintln!("hwscan: give an id as 8086:2723, PCI\\VEN_8086&DEV_2723, USB\\VID_0CF3&PID_9271 or ACPI\\PNP0C50 (hwscan help)");
}

// ------------------------------------------------------------------------------------------------
// selftest (A2): broken tables built in memory must each end in a line, not a fault.

/// A table in `buf` with signature `s`, header length `len`, body `body`; checksum made right.
fn build(buf: &mut [u8; 128], s: &[u8; 4], len: u32, body: &[u8]) -> usize {
    buf.fill(0);
    buf[..4].copy_from_slice(s);
    buf[4..8].copy_from_slice(&len.to_le_bytes());
    let end = (36 + body.len()).min(buf.len());
    buf[36..end].copy_from_slice(&body[..end - 36]);
    let n = (len as usize).min(buf.len());
    let sum = buf[..n].iter().fold(0u8, |s, &x| s.wrapping_add(x));
    buf[9] = buf[9].wrapping_sub(sum);
    end
}

fn selftest() {
    // Device(KBD_) { Name(_HID, EisaId("PNP0303")) Name(_CID, Package(2) { "PNP0C50", EisaId("PNP0F13") }) }
    const GOOD: &[u8] = &[
        0x5B, 0x82, 0x20, b'K', b'B', b'D', b'_', //
        0x08, b'_', b'H', b'I', b'D', 0x0C, 0x41, 0xD0, 0x03, 0x03, //
        0x08, b'_', b'C', b'I', b'D', 0x12, 0x10, 0x02, 0x0D, b'P', b'N', b'P', b'0', b'C', b'5', b'0', 0x00, 0x0C, 0x41, 0xD0, 0x0F, 0x13,
    ];
    let mut buf = [0u8; 128];
    let (mut fails, mut cases) = (0, 0);
    let mut case = |name: &str, want: &str, got: &str| {
        let ok = want == got;
        cases += 1;
        if !ok {
            fails += 1;
        }
        crate::kprintln!("hwscan selftest: {}: {} ({})", name, got, if ok { "as expected" } else { "NOT as expected" });
    };
    let scan = |t: &[u8]| -> Ids {
        let mut ids = Ids::new();
        scan_aml(t, &mut ids);
        ids
    };
    // 1. a good table: two ids under KBD_, one from a package
    let n = build(&mut buf, b"DSDT", (36 + GOOD.len()) as u32, GOOD);
    let r = match table_of(&buf[..n]) {
        Ok(t) => {
            let ids = scan(t);
            let all: [&[u8]; 3] = [b"PNP0303", b"PNP0C50", b"PNP0F13"];
            if ids.n == 3 && (0..3).all(|k| &ids.id[k][..ids.len[k] as usize] == all[k] && &ids.dev[k] == b"KBD_") { "3 ids under KBD_" } else { "wrong ids" }
        }
        Err(b) => bad_line(b),
    };
    case("good DSDT", "3 ids under KBD_", r);
    // 2. truncated: the header says 100 bytes, 50 can be read
    build(&mut buf, b"DSDT", 100, GOOD);
    case("truncated DSDT", bad_line(Bad::PastEnd), table_of(&buf[..50]).err().map_or("scanned", bad_line));
    // 3. a length of 0xFFFFFFFF
    build(&mut buf, b"SSDT", 0xFFFF_FFFF, &[]);
    case("length 0xFFFFFFFF", bad_line(Bad::Huge), table_of(&buf).err().map_or("scanned", bad_line));
    // 4. a length shorter than the header
    build(&mut buf, b"SSDT", 20, &[]);
    case("length 20", bad_line(Bad::Short), table_of(&buf).err().map_or("scanned", bad_line));
    // 5. a header that cannot be read at all
    case("10 readable bytes", bad_line(Bad::NoHeader), table_of(&buf[..10]).err().map_or("scanned", bad_line));
    // 6. a bad checksum
    let n = build(&mut buf, b"DSDT", (36 + GOOD.len()) as u32, GOOD);
    buf[40] ^= 0x55;
    case("bad checksum", bad_line(Bad::Checksum), table_of(&buf[..n]).err().map_or("scanned", bad_line));
    // 7. _HID's NameOp at the very end, then its EisaId cut, then a string with no end
    let tails: [(&str, &[u8]); 4] = [
        ("NameOp at the last byte", &[0x08]),
        ("_HID with no value", &[0x08, b'_', b'H', b'I', b'D']),
        ("EisaId cut off", &[0x08, b'_', b'H', b'I', b'D', 0x0C, 0x41, 0xD0]),
        ("string with no end", &[0x08, b'_', b'H', b'I', b'D', 0x0D, b'E', b'L', b'A', b'N']),
    ];
    for (name, body) in tails {
        let n = build(&mut buf, b"SSDT", (36 + body.len()) as u32, body);
        let r = match table_of(&buf[..n]) {
            Ok(t) => {
                let ids = scan(t);
                match (ids.n, ids.cut) {
                    (0, 0) => "no id, nothing cut",
                    (0, 1) => "no id, 1 cut off",
                    _ => "wrong",
                }
            }
            Err(b) => bad_line(b),
        };
        case(name, if body.len() <= 1 { "no id, nothing cut" } else { "no id, 1 cut off" }, r);
    }
    // 8. a package that claims more elements and bytes than the table has
    const PKG: &[u8] = &[0x08, b'_', b'C', b'I', b'D', 0x12, 0x3F, 0x09, 0x0C, 0x41, 0xD0, 0x0C];
    let n = build(&mut buf, b"SSDT", (36 + PKG.len()) as u32, PKG);
    let r = match table_of(&buf[..n]) {
        Ok(t) => match scan(t) {
            ids if ids.n == 0 && ids.cut == 1 => "no id, 1 cut off",
            _ => "wrong",
        },
        Err(b) => bad_line(b),
    };
    case("package past the end", "no id, 1 cut off", r);
    // 9. a firmware pointer near the top of the address space (architect's review: no overflow, no fault)
    case("pointer 0xFFFFFFFFFFFFF000", bad_line(Bad::NotAcpi), readable(0xFFFF_FFFF_FFFF_F000).and_then(table_of).err().map_or("scanned", bad_line));
    // 9b. a pointer into the kernel's own memory (loader data, not ACPI): refused before any byte is read
    let own = core::ptr::addr_of!(FW_N) as u64;
    case("pointer into kernel memory", bad_line(Bad::NotAcpi), readable(own).and_then(table_of).err().map_or("scanned", bad_line));
    // 10. a Device name whose PkgLength runs off the end
    let n = build(&mut buf, b"SSDT", 39, &[0x5B, 0x82, 0xC0]);
    let r = match table_of(&buf[..n]) {
        Ok(t) => if scan(t).n == 0 { "no id" } else { "wrong" },
        Err(b) => bad_line(b),
    };
    case("Device cut off", "no id", r);
    // 11. the driver hook (hwdb AcpiV::Asks): each answer a driver may give, as hwscan words it
    fn driven() -> Claim {
        Claim::Driven("test-driver")
    }
    fn off() -> Claim {
        Claim::Off("start method 8, not driven")
    }
    fn none() -> Claim {
        Claim::Missing
    }
    for (name, f, want) in [
        ("hook: driven", driven as fn() -> Claim, "driven by test-driver"),
        ("hook: off", off as fn() -> Claim, "off start method 8, not driven"),
        ("hook: missing", none as fn() -> Claim, "missing no driver in AfriSovera yet"),
    ] {
        let (kind, detail) = hwdb::claim_words(AcpiV::Asks(f).ask());
        let mut got = [0u8; 64];
        let (k, d) = (kind.as_bytes(), detail.as_bytes());
        let n = (k.len() + 1 + d.len()).min(got.len());
        got[..k.len()].copy_from_slice(k);
        got[k.len()] = b' ';
        got[k.len() + 1..n].copy_from_slice(&d[..n - k.len() - 1]);
        case(name, want, core::str::from_utf8(&got[..n]).unwrap_or("?"));
    }
    if fails == 0 {
        crate::kprintln!("hwscan selftest: all {} cases as expected, every one ended in a line", cases);
    } else {
        crate::kprintln!("hwscan selftest: {} case(s) NOT as expected", fails);
    }
}
