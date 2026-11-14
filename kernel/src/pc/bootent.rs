//! `bootentries`: the PC firmware's boot entries, READ ONLY (PC track step 5, 7 Nov 2026).
//!
//! Booting AfriSovera from its own partition next to Windows needs one more firmware boot
//! entry. Before AfriSovera ever adds one, this shows the entries as they are: BootOrder,
//! each Boot#### (active or not, its name, where it points: the disk, the partition, the
//! file), the one the PC was started from this time, and the next free number. The variables
//! were read at power-on (`uefi::keep_boot_entries`); nothing here talks to the firmware and
//! nothing is written.

use crate::uefi;

fn hex4(v: u16) -> [u8; 4] {
    let h = b"0123456789ABCDEF";
    [h[(v >> 12) as usize & 15], h[(v >> 8) as usize & 15], h[(v >> 4) as usize & 15], h[v as usize & 15]]
}

/// A small line buffer (no allocation needed for one entry's text).
struct Line {
    b: [u8; 200],
    n: usize,
}

impl Line {
    fn new() -> Self {
        Line { b: [0; 200], n: 0 }
    }
    fn byte(&mut self, c: u8) {
        if self.n < self.b.len() {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    fn s(&mut self, t: &str) {
        for c in t.bytes() {
            self.byte(c);
        }
    }
    fn num(&mut self, mut v: u64) {
        let mut d = [0u8; 20];
        let mut k = 0;
        loop {
            d[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while k > 0 {
            k -= 1;
            self.byte(d[k]);
        }
    }
    fn hex(&mut self, v: u64, digits: usize) {
        let h = b"0123456789abcdef";
        for i in (0..digits).rev() {
            self.byte(h[(v >> (4 * i)) as usize & 15]);
        }
    }
    fn str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

fn u16le(d: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([d[i], d[i + 1]])
}
fn u32le(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}
fn u64le(d: &[u8], i: usize) -> u64 {
    u32le(d, i) as u64 | (u32le(d, i + 4) as u64) << 32
}

/// A GUID in its usual text form (mixed-endian, as Windows shows it).
fn guid(l: &mut Line, g: &[u8]) {
    l.hex(u32le(g, 0) as u64, 8);
    l.byte(b'-');
    l.hex(u16le(g, 4) as u64, 4);
    l.byte(b'-');
    l.hex(u16le(g, 6) as u64, 4);
    l.byte(b'-');
    for (i, b) in g[8..16].iter().enumerate() {
        if i == 2 {
            l.byte(b'-');
        }
        l.hex(*b as u64, 2);
    }
}

/// One device path, short: where the entry points. The file path seen is also kept (lower-case
/// ASCII) in `file`, to recognise AfriSovera's own file.
fn path(l: &mut Line, p: &[u8], file: &mut [u8; 64], file_n: &mut usize) {
    let mut i = 0;
    let mut first = true;
    while i + 4 <= p.len() {
        let (ty, sub, len) = (p[i], p[i + 1], u16le(p, i + 2) as usize);
        if len < 4 || i + len > p.len() {
            l.s(" (damaged path)");
            return;
        }
        if ty == 0x7F {
            break; // end of this path (an entry may hold more; the first is the one used)
        }
        let n = &p[i..i + len];
        if !first {
            l.byte(b'/');
        }
        first = false;
        match (ty, sub) {
            (2, 1) => l.s("PciRoot"),
            (1, 1) if len >= 6 => {
                l.s("Pci(");
                l.hex(n[5] as u64, 2);
                l.byte(b'.');
                l.num(n[4] as u64);
                l.byte(b')');
            }
            (3, 0x12) if len >= 6 => {
                l.s("Sata(");
                l.num(u16le(n, 4) as u64);
                l.byte(b')');
            }
            (3, 0x17) if len >= 8 => {
                l.s("NVMe(ns ");
                l.num(u32le(n, 4) as u64);
                l.byte(b')');
            }
            (3, 5) if len >= 6 => {
                l.s("USB(");
                l.num(n[4] as u64);
                l.byte(b')');
            }
            (3, 1) => l.s("Scsi"),
            (3, 2) => l.s("Ata"),
            (3, 0x0b) => l.s("MAC"),
            (3, 0x0c) => l.s("IPv4"),
            (3, 0x0d) => l.s("IPv6"),
            (3, 0x18) => l.s("URI"),
            (4, 1) if len >= 42 => {
                l.s("HD(");
                l.num(u32le(n, 4) as u64);
                match n[41] {
                    2 => {
                        l.s(",GPT,");
                        guid(l, &n[24..40]);
                    }
                    1 => {
                        l.s(",MBR,");
                        l.hex(u32le(n, 24) as u64, 8);
                    }
                    _ => l.s(",?"),
                }
                l.s(",start ");
                l.num(u64le(n, 8));
                l.byte(b')');
            }
            (4, 2) => l.s("CDROM"),
            (4, 4) => {
                let mut k = 4;
                while k + 1 < len {
                    let c = u16le(n, k);
                    if c == 0 {
                        break;
                    }
                    let a = if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' };
                    l.byte(a);
                    if *file_n < file.len() {
                        file[*file_n] = a.to_ascii_lowercase();
                        *file_n += 1;
                    }
                    k += 2;
                }
            }
            (4, 6) => l.s("FvFile"),
            (4, 7) => l.s("Fv"),
            (1, 4) | (3, 10) | (4, 3) => l.s("Vendor"),
            (5, _) => l.s("Legacy"),
            _ => {
                l.byte(b'(');
                l.num(ty as u64);
                l.byte(b'/');
                l.num(sub as u64);
                l.byte(b')');
            }
        }
        i += len;
    }
    if first {
        l.s("(empty path)");
    }
}

/// `bootentries`
pub fn list() {
    let bv = uefi::boot_vars();
    if !bv.read {
        crate::kprintln!("bootentries: the firmware's boot entries could not be read at power-on ({})", bv.why);
        return;
    }
    if bv.changed {
        crate::kprintln!("bootentries: the firmware's boot entries, as read again after bootentry changed them (nothing is changed here)");
    } else {
        crate::kprintln!("bootentries: the firmware's boot entries, as read at power-on (nothing is changed)");
    }
    let mut l = Line::new();
    l.s("  BootOrder:");
    if bv.n_order == 0 {
        l.s(if bv.order_cut { " (too long to show)" } else { " (none)" });
    }
    for &o in &bv.order[..bv.n_order] {
        l.byte(b' ');
        for c in hex4(o) {
            l.byte(c);
        }
    }
    crate::kprintln!("{}", l.str());
    if let Some(nx) = bv.next {
        let h = hex4(nx);
        let hs = core::str::from_utf8(&h).unwrap_or("?");
        // a BootNext naming no Boot#### (12 Nov 2026): the firmware deletes it at the next power-on and follows
        // BootOrder as usual. Only said when every entry was read, so it cannot be one of the entries not kept.
        if bv.skipped == 0 && !bv.num[..bv.n].contains(&nx) {
            crate::kprintln!(
                "  BootNext: {} (no such entry: the firmware deletes it at the next power-on and follows BootOrder)",
                hs
            );
        } else {
            crate::kprintln!("  BootNext: {} (started ONCE at the next power-on, then the firmware deletes it)", hs);
        }
    }
    let mut ours = None;
    let mut used = [false; 0x100];
    for k in 0..bv.n {
        let (num, d) = (bv.num[k], &bv.data[k][..bv.len[k]]);
        if num < 0x100 {
            used[num as usize] = true;
        }
        let attr = u32le(d, 0);
        let fpl = u16le(d, 4) as usize;
        let mut l = Line::new();
        l.s("  Boot");
        for c in hex4(num) {
            l.byte(c);
        }
        l.s(if attr & 1 != 0 { "  active  " } else { "  off     " });
        // the description: UTF-16, NUL-ended, after the attributes and the path length
        let mut i = 6;
        let mut desc = [0u8; 64];
        let mut dn = 0;
        l.byte(b'"');
        while i + 1 < d.len() {
            let c = u16le(d, i);
            i += 2;
            if c == 0 {
                break;
            }
            let a = if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' };
            if dn < desc.len() {
                desc[dn] = a.to_ascii_lowercase();
                dn += 1;
            }
            if l.n < 70 {
                l.byte(a);
            }
        }
        l.s("\"  ");
        let mut file = [0u8; 64];
        let mut file_n = 0;
        if i + fpl <= d.len() {
            path(&mut l, &d[i..i + fpl], &mut file, &mut file_n);
        } else {
            l.s("(damaged entry)");
        }
        if attr & 8 != 0 {
            l.s("  [hidden]");
        }
        if bv.current == Some(num) {
            l.s("  <- started this time");
        }
        crate::kprintln!("{}", l.str());
        let (d, f) = (&desc[..dn], &file[..file_n]);
        if d.starts_with(b"afrisovera") || f.windows(12).any(|w| w == b"\\afrisovera\\") {
            ours = Some(num);
        }
    }
    if bv.skipped > 0 {
        crate::kprintln!("  ({} more entries not shown)", bv.skipped);
    }
    // places in BootOrder with no Boot#### behind them (12 Nov 2026): some firmware leaves one when another
    // system's entry is deleted; it passes over them, and bootentry never takes their numbers. Only said when
    // every entry was read, so a number cannot just be one of the entries not kept.
    if bv.skipped == 0 && !bv.order_cut {
        let mut l = Line::new();
        let (mut k, mut more) = (0, 0);
        for (i, &o) in bv.order[..bv.n_order].iter().enumerate() {
            if bv.num[..bv.n].contains(&o) || bv.order[..i].contains(&o) {
                continue;
            }
            if k == 10 { // the rest are counted, so the line stays within its 200 bytes
                more += 1;
                continue;
            }
            l.s(if k == 0 { "  in BootOrder but no such entry (the firmware passes over it, nothing is changed): " } else { " " });
            l.s("Boot");
            for c in hex4(o) {
                l.byte(c);
            }
            k += 1;
        }
        if more > 0 {
            l.s(" and ");
            l.num(more);
            l.s(" more");
        }
        if k > 0 {
            crate::kprintln!("{}", l.str());
        }
    }
    // the next free number: what an AfriSovera entry would be called
    match (0..0x100u16).find(|n| !used[*n as usize] && !bv.order[..bv.n_order].contains(n)) {
        Some(f) => {
            let h = hex4(f);
            crate::kprintln!("  {} entries; the next free number is Boot{}", bv.n, core::str::from_utf8(&h).unwrap_or("?"));
        }
        None => crate::kprintln!("  {} entries; no free number below Boot0100", bv.n),
    }
    match ours {
        Some(n) => {
            let h = hex4(n);
            crate::kprintln!("  AfriSovera's own entry: Boot{}", core::str::from_utf8(&h).unwrap_or("?"));
        }
        None => crate::kprintln!("  AfriSovera's own entry: none yet (adding one will ask for the owner's word)"),
    }
}

/// One Boot#### as read: its description (ASCII, `?` for anything else) and the file path its first device path
/// names (lower-case ASCII).
struct Entry {
    desc: [u8; 64],
    dn: usize,
    file: [u8; 64],
    file_n: usize,
}

/// Whose a Boot#### is: AfriSovera's main entry, its trial entry (safe updates, AFR-0010: the description is
/// exactly "AfriSovera trial"), or not ours. A trial entry is never taken for the main one; a hand-made entry with
/// another description stays as before.
#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Main,
    Trial,
    Other,
}

impl Entry {
    fn kind(&self) -> Kind {
        let low = self.desc.map(|c| c.to_ascii_lowercase());
        if low[..self.dn] == *b"afrisovera trial" {
            Kind::Trial
        } else if low[..self.dn].starts_with(b"afrisovera") || self.file[..self.file_n].windows(12).any(|w| w == b"\\afrisovera\\") {
            Kind::Main
        } else {
            Kind::Other
        }
    }
}

/// An EFI_LOAD_OPTION's description and file path; None when it is too short to hold them.
fn parse(d: &[u8]) -> Option<Entry> {
    if d.len() < 6 {
        return None;
    }
    let fpl = u16le(d, 4) as usize;
    let mut e = Entry { desc: [0; 64], dn: 0, file: [0; 64], file_n: 0 };
    let mut i = 6;
    while i + 1 < d.len() {
        let c = u16le(d, i);
        i += 2;
        if c == 0 {
            break;
        }
        if e.dn < e.desc.len() {
            e.desc[e.dn] = if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' };
            e.dn += 1;
        }
    }
    if i + fpl <= d.len() {
        path(&mut Line::new(), &d[i..i + fpl], &mut e.file, &mut e.file_n);
    }
    Some(e)
}

/// For `updatestage` (safe updates U3): AfriSovera's main entry and the file its device path names (lower-case,
/// such as `\efi\afrisovera\afrisovera.efi`), as read at power-on or after the last change; None if there is none.
#[allow(dead_code)] // its caller, updatestage, is the next change (architect: this refactor merges first)
pub(crate) fn main_file() -> Option<(u16, [u8; 64], usize)> {
    find(Kind::Main).map(|(n, e)| (n, e.file, e.file_n))
}

/// For `updatestage`: AfriSovera's trial entry, if there is one.
#[allow(dead_code)] // as main_file
pub(crate) fn trial_number() -> Option<u16> {
    find(Kind::Trial).map(|(n, _)| n)
}

/// The last Boot#### of this kind, as `numbers` picks AfriSovera's own.
fn find(kind: Kind) -> Option<(u16, Entry)> {
    let bv = uefi::boot_vars();
    if !bv.read {
        return None;
    }
    let mut found = None;
    for k in 0..bv.n {
        if let Some(e) = parse(&bv.data[k][..bv.len[k]]) {
            if e.kind() == kind {
                found = Some((bv.num[k], e));
            }
        }
    }
    found
}

/// For `bootplan`: (the next free Boot number, AfriSovera's own entry if it has one, the
/// first entry of BootOrder and its name), worked out as `list` does, printing nothing.
pub fn numbers() -> Option<(Option<u16>, Option<u16>, Option<(u16, [u8; 64], usize)>)> {
    let bv = uefi::boot_vars();
    if !bv.read {
        return None;
    }
    let mut used = [false; 0x100];
    let mut ours = None;
    let mut first = None;
    let mut first_rank: Option<usize> = None;
    for k in 0..bv.n {
        let (num, d) = (bv.num[k], &bv.data[k][..bv.len[k]]);
        if num < 0x100 {
            used[num as usize] = true;
        }
        let Some(e) = parse(d) else { continue };
        if e.kind() == Kind::Main {
            ours = Some(num);
        }
        let (desc, dn) = (e.desc, e.dn);
        // what the PC really starts first: the earliest in BootOrder that is active, not hidden
        // and a boot option (the firmware skips hidden ones and apps such as its own setup screen)
        let attr = u32le(d, 0);
        if attr & 1 != 0 && attr & 8 == 0 && attr & 0x1F00 == 0 {
            if let Some(r) = bv.order[..bv.n_order].iter().position(|o| *o == num) {
                if first_rank.map_or(true, |f| r < f) {
                    first_rank = Some(r);
                    first = Some((num, desc, dn));
                }
            }
        }
    }
    let free = (0..0x100u16).find(|n| !used[*n as usize] && !bv.order[..bv.n_order].contains(n));
    Some((free, ours, first))
}

/// For `bootplan` and `bootnext` (12 Nov 2026): the BootOrder places before the entry the PC really starts first,
/// each with why the firmware passes over it (no such entry, off, hidden, one of the firmware's own
/// apps, a damaged entry), so "keeps starting X first" is never taken for BootOrder's first place.
/// Prints nothing when BootOrder's first place is the one started, or when not every entry was read.
pub fn passed_over() {
    let bv = uefi::boot_vars();
    if !bv.read || bv.skipped > 0 || bv.order_cut {
        return;
    }
    let mut l = Line::new();
    let (mut c, mut more, mut found) = (0, 0u64, false);
    for (i, &o) in bv.order[..bv.n_order].iter().enumerate() {
        if bv.order[..i].contains(&o) {
            continue;
        }
        let why = match (0..bv.n).find(|&k| bv.num[k] == o) {
            None => "no such entry",
            Some(e) => {
                let d = &bv.data[e][..bv.len[e]];
                if d.len() < 6 {
                    "damaged"
                } else {
                    let attr = u32le(d, 0);
                    if attr & 1 == 0 {
                        "off"
                    } else if attr & 8 != 0 {
                        "hidden"
                    } else if attr & 0x1F00 != 0 {
                        "a firmware app"
                    } else {
                        found = true; // the rule numbers() uses for the entry started first
                        break;
                    }
                }
            }
        };
        if c == 6 { // the rest are counted, so the line stays within its 200 bytes
            more += 1;
            continue;
        }
        l.s(if c == 0 { "  passed over before it in BootOrder: Boot" } else { ", Boot" });
        for h in hex4(o) {
            l.byte(h);
        }
        l.s(" (");
        l.s(why);
        l.byte(b')');
        c += 1;
    }
    if !found || c == 0 {
        return;
    }
    if more > 0 {
        l.s(" and ");
        l.num(more);
        l.s(" more");
    }
    crate::kprintln!("{}", l.str());
}

/// For `bootplan` (11 Nov 2026): every other system's entry in BootOrder, one that starts a file
/// from a disk partition (Windows Boot Manager, ubuntu), each said to stay as it is, with its
/// partition, its file and its place in BootOrder. AfriSovera's own entry and the firmware's own
/// (its setup screen, a shell, a whole disk or the network) are left out. Prints nothing when
/// the entries were not read or no other system has one.
pub fn others() {
    let bv = uefi::boot_vars();
    if !bv.read {
        return;
    }
    for (place, &o) in bv.order[..bv.n_order].iter().enumerate() {
        let Some(k) = (0..bv.n).find(|&k| bv.num[k] == o) else { continue };
        let d = &bv.data[k][..bv.len[k]];
        if d.len() < 6 {
            continue;
        }
        let (attr, fpl) = (u32le(d, 0), u16le(d, 4) as usize);
        let mut l = Line::new();
        l.s("  stays as it is: Boot");
        for c in hex4(o) {
            l.byte(c);
        }
        l.s(" \"");
        let (mut i, mut low, mut ln) = (6, [0u8; 10], 0);
        while i + 1 < d.len() {
            let c = u16le(d, i);
            i += 2;
            if c == 0 {
                break;
            }
            let a = if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' };
            if ln < low.len() {
                low[ln] = a.to_ascii_lowercase();
                ln += 1;
            }
            if l.n < 90 {
                l.byte(a);
            }
        }
        l.byte(b'"');
        if i + fpl > d.len() {
            continue;
        }
        // the partition (HD node) and the file (its file path nodes, joined) of the first path
        let (p, mut j, mut part, mut file) = (&d[i..i + fpl], 0, None, Line::new());
        while j + 4 <= p.len() {
            let (ty, sub, len) = (p[j], p[j + 1], u16le(p, j + 2) as usize);
            if len < 4 || j + len > p.len() || ty == 0x7F {
                break;
            }
            let n = &p[j..j + len];
            if (ty, sub) == (4, 1) && len >= 42 {
                part = Some(u32le(n, 4));
            }
            if (ty, sub) == (4, 4) {
                let mut m = 4;
                while m + 1 < len {
                    let c = u16le(n, m);
                    if c == 0 {
                        break;
                    }
                    file.byte(if (0x20..0x7F).contains(&c) { c as u8 } else { b'?' });
                    m += 2;
                }
            }
            j += len;
        }
        let Some(pn) = part else { continue };
        let lowf = file.b.map(|c| c.to_ascii_lowercase());
        if file.n == 0 || low[..ln].starts_with(b"afrisovera") || lowf[..file.n].windows(12).any(|w| w == b"\\afrisovera\\") {
            continue;
        }
        l.s(", partition ");
        l.num(pn as u64);
        l.s(", ");
        l.s(file.str());
        if attr & 1 == 0 {
            l.s(", off");
        }
        if attr & 8 != 0 {
            l.s(", hidden");
        }
        l.s(" (place ");
        l.num(place as u64 + 1);
        l.s(" in BootOrder)");
        crate::kprintln!("{}", l.str());
    }
}

/// "Boot" + the number, as text.
pub fn name(n: u16) -> [u8; 8] {
    let h = hex4(n);
    [b'B', b'o', b'o', b't', h[0], h[1], h[2], h[3]]
}

// --- `bootentry [yes <code>]` / `bootentry remove [yes <code>]` (PC track step 5, 7 Nov 2026) ---
//
// AfriSovera's own firmware boot entry, next to Windows: one Boot#### "AfriSovera" pointing at
// \EFI\afrisovera\afrisovera.efi (put there by `bootcopy`) in its EFI system partition, added
// at the END of BootOrder so the PC keeps starting what it started before; AfriSovera is then
// picked from the firmware's boot menu. Only on the owner's code (tied to the entry and to
// BootOrder as the firmware has it now); read back after writing. `bootentry remove` is the
// way back: BootOrder without it, the Boot#### deleted.

const DESC: &str = "AfriSovera";
/// The trial entry's description (safe updates, AFR-0010): told apart from the main entry by this exact text.
#[allow(dead_code)] // its user, updatestage, is the next change
pub(crate) const TRIAL_DESC: &str = "AfriSovera trial";
/// Room for the longest option: a 16-character description and a 40-character file path.
const OPT_CAP: usize = 6 + 2 * (16 + 1) + 42 + 4 + 2 * (40 + 1) + 4;

/// The EFI_LOAD_OPTION: active, `desc`, HD(entry, start, size, GPT, guid)/File(`file`)/End, and its length.
/// For the main entry ("AfriSovera", esp::FILE) these are the same 140 bytes as before safe updates.
fn option(desc: &str, file: &str, ent: u64, start: u64, size: u64, guid: &[u8; 16]) -> ([u8; OPT_CAP], usize) {
    assert!(desc.len() <= 16 && file.len() <= 40);
    let file_node = 4 + 2 * (file.len() + 1);
    let path_len = 42 + file_node + 4;
    let mut o = [0u8; OPT_CAP];
    o[0..4].copy_from_slice(&1u32.to_le_bytes()); // LOAD_OPTION_ACTIVE
    o[4..6].copy_from_slice(&(path_len as u16).to_le_bytes());
    let mut i = 6;
    for c in desc.bytes() {
        o[i] = c;
        i += 2;
    }
    i += 2; // the description's NUL
    // HD: media path, hard drive, 42 bytes
    o[i..i + 4].copy_from_slice(&[4, 1, 42, 0]);
    o[i + 4..i + 8].copy_from_slice(&(ent as u32).to_le_bytes());
    o[i + 8..i + 16].copy_from_slice(&start.to_le_bytes());
    o[i + 16..i + 24].copy_from_slice(&size.to_le_bytes());
    o[i + 24..i + 40].copy_from_slice(guid);
    o[i + 40] = 2; // GPT
    o[i + 41] = 2; // the signature is a GUID
    i += 42;
    // File: media path, file path, UTF-16 with its NUL
    o[i..i + 2].copy_from_slice(&[4, 4]);
    o[i + 2..i + 4].copy_from_slice(&(file_node as u16).to_le_bytes());
    let mut k = i + 4;
    for c in file.bytes() {
        o[k] = c;
        k += 2;
    }
    i += file_node;
    o[i..i + 4].copy_from_slice(&[0x7F, 0xFF, 4, 0]);
    (o, i + 4)
}

/// BootOrder as the firmware has it now: (numbers, count), or why it cannot be used.
fn order_now() -> Result<([u16; uefi::ORDER_MAX], usize), &'static str> {
    let mut b = [0u8; uefi::ORDER_MAX * 2];
    let mut o = [0u16; uefi::ORDER_MAX];
    match uefi::get_global(b"BootOrder", &mut b) {
        Ok(n) if n % 2 == 0 => {
            for i in 0..n / 2 {
                o[i] = u16le(&b, 2 * i);
            }
            Ok((o, n / 2))
        }
        Ok(_) => Err("the firmware's BootOrder has an odd length"),
        Err(uefi::ERR_NOT_FOUND) => Ok((o, 0)),
        Err(_) => Err("the firmware's BootOrder could not be read now (or is longer than 64 entries)"),
    }
}

fn order_bytes(o: &[u16], out: &mut [u8; uefi::ORDER_MAX * 2]) -> usize {
    for (i, v) in o.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
    }
    o.len() * 2
}

fn order_line(l: &mut Line, o: &[u16]) {
    if o.is_empty() {
        l.s(" (none)");
    }
    for &v in o {
        l.byte(b' ');
        for c in hex4(v) {
            l.byte(c);
        }
    }
}

/// The owner's 4-hex code: the command, the number, the entry's bytes and BootOrder now.
fn code(tag: &[u8], num: u16, opt: &[u8], order: &[u8]) -> u32 {
    let mut c = crate::gpt::crc_feed(0xFFFF_FFFF, tag);
    c = crate::gpt::crc_feed(c, &num.to_le_bytes());
    c = crate::gpt::crc_feed(c, opt);
    c = crate::gpt::crc_feed(c, order);
    !c & 0xFFFF
}

fn agreed(cmd: &str, given: Option<&str>, want: u32) -> bool {
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want) => true,
        Some(_) => {
            crate::kprintln!("{}: that code does not fit the firmware's boot entries as they are now; nothing written", cmd);
            false
        }
        None => {
            crate::kprintln!("{}: nothing written yet. To do it, type exactly: {} yes {:04x}", cmd, cmd, want);
            false
        }
    }
}

/// Whether the firmware now holds exactly `want` under `name` (empty: no such variable).
fn reads_back(name: &[u8], want: &[u8]) -> bool {
    let mut b = [0u8; uefi::OPT_MAX];
    match uefi::get_global(name, &mut b) {
        Ok(n) => !want.is_empty() && n == want.len() && b[..n] == *want,
        Err(uefi::ERR_NOT_FOUND) => want.is_empty(),
        Err(_) => false,
    }
}

/// `bootentry [yes <code>]`
pub fn add(given: Option<&str>) {
    let Some((free, ours, first)) = numbers() else {
        crate::kprintln!("bootentry: the firmware's boot entries could not be read at power-on; nothing written");
        return;
    };
    if let Some(n) = ours {
        let nm = name(n);
        crate::kprintln!(
            "bootentry: AfriSovera already has a boot entry ({}); nothing to add (bootentry remove takes it out)",
            core::str::from_utf8(&nm).unwrap_or("?")
        );
        return;
    }
    let (label, ent, start, size, guid) = match crate::esp::placed() {
        Ok(v) => v,
        Err(why) => {
            crate::kprintln!("bootentry: {}; nothing written", why);
            return;
        }
    };
    let Some(num) = free else {
        crate::kprintln!("bootentry: no free Boot number below Boot0100; nothing written");
        return;
    };
    let nm = name(num);
    let nms = core::str::from_utf8(&nm).unwrap_or("?");
    let mut probe = [0u8; uefi::OPT_MAX];
    if !matches!(uefi::get_global(&nm, &mut probe), Err(uefi::ERR_NOT_FOUND)) {
        crate::kprintln!("bootentry: the firmware now has a {} that was not there at power-on; nothing written", nms);
        return;
    }
    let (order, n) = match order_now() {
        Ok(v) => v,
        Err(why) => {
            crate::kprintln!("bootentry: {}; nothing written", why);
            return;
        }
    };
    if n >= uefi::ORDER_MAX || order[..n].contains(&num) {
        crate::kprintln!("bootentry: BootOrder is full or already names {}; nothing written", nms);
        return;
    }
    let (opt, ol) = option(DESC, crate::esp::FILE, ent, start, size, &guid);
    let opt = &opt[..ol];
    let mut ob = [0u8; uefi::ORDER_MAX * 2];
    let olen = order_bytes(&order[..n], &mut ob);
    let mut new = order;
    new[n] = num;
    let mut nb = [0u8; uefi::ORDER_MAX * 2];
    let nlen = order_bytes(&new[..n + 1], &mut nb);
    crate::kprintln!(
        "bootentry: {} \"{}\" -> {} partition {} (start {}, {} blocks){}",
        nms,
        DESC,
        crate::gpt::label_str(&label),
        ent,
        start,
        size,
        crate::esp::FILE
    );
    let mut l = Line::new();
    l.s("  BootOrder now:");
    order_line(&mut l, &order[..n]);
    l.s("  -> after:");
    order_line(&mut l, &new[..n + 1]);
    crate::kprintln!("{}", l.str());
    match first {
        Some((_, d, dn)) => crate::kprintln!(
            "  the PC keeps starting \"{}\" first; AfriSovera is picked from the firmware's boot menu",
            core::str::from_utf8(&d[..dn]).unwrap_or("?")
        ),
        None => crate::kprintln!("  the PC keeps its first choice; AfriSovera is picked from the firmware's boot menu"),
    }
    if !agreed("bootentry", given, code(b"bootentry", num, opt, &ob[..olen])) {
        return;
    }
    let st = uefi::set_global(&nm, opt);
    if st != uefi::SUCCESS {
        crate::kprintln!("bootentry: the firmware refused to write {} (status {:#x}); nothing changed", nms, st);
        return;
    }
    if !reads_back(&nm, opt) {
        let back = uefi::set_global(&nm, &[]);
        crate::kprintln!(
            "bootentry: {} did not read back as written, so it was taken out again ({}); BootOrder not changed",
            nms,
            if back == uefi::SUCCESS { "done" } else { "the firmware refused: bootentries shows what is left" }
        );
        uefi::reread_boot_entries();
        return;
    }
    let st = uefi::set_global(b"BootOrder", &nb[..nlen]);
    if st != uefi::SUCCESS || !reads_back(b"BootOrder", &nb[..nlen]) {
        let back = uefi::set_global(&nm, &[]);
        let put = if st == uefi::SUCCESS && olen > 0 { uefi::set_global(b"BootOrder", &ob[..olen]) } else { uefi::SUCCESS };
        crate::kprintln!(
            "bootentry: BootOrder could not be written (status {:#x}); {} taken out again ({})",
            st,
            nms,
            if back == uefi::SUCCESS && put == uefi::SUCCESS { "nothing left changed" } else { "the firmware refused: bootentries shows what is left" }
        );
        uefi::reread_boot_entries();
        return;
    }
    uefi::reread_boot_entries();
    crate::kprintln!(
        "bootentry: {} \"{}\" added at the END of BootOrder and read back; the firmware's boot menu now offers AfriSovera (bootentry remove takes it out)",
        nms,
        DESC
    );
}

/// `bootentry remove [yes <code>]`
pub fn remove(given: Option<&str>) {
    let Some((_, ours, _)) = numbers() else {
        crate::kprintln!("bootentry remove: the firmware's boot entries could not be read at power-on; nothing written");
        return;
    };
    let Some(num) = ours else {
        crate::kprintln!("bootentry remove: AfriSovera has no boot entry; nothing to remove");
        return;
    };
    let nm = name(num);
    let nms = core::str::from_utf8(&nm).unwrap_or("?");
    let mut opt = [0u8; uefi::OPT_MAX];
    let olen = match uefi::get_global(&nm, &mut opt) {
        Ok(n) => n,
        Err(_) => {
            crate::kprintln!("bootentry remove: {} could not be read from the firmware now; nothing written", nms);
            return;
        }
    };
    let (order, n) = match order_now() {
        Ok(v) => v,
        Err(why) => {
            crate::kprintln!("bootentry remove: {}; nothing written", why);
            return;
        }
    };
    let mut ob = [0u8; uefi::ORDER_MAX * 2];
    let oblen = order_bytes(&order[..n], &mut ob);
    let mut new = [0u16; uefi::ORDER_MAX];
    let mut k = 0;
    for &v in &order[..n] {
        if v != num {
            new[k] = v;
            k += 1;
        }
    }
    let mut nb = [0u8; uefi::ORDER_MAX * 2];
    let nlen = order_bytes(&new[..k], &mut nb);
    crate::kprintln!("bootentry remove: {} \"{}\" would be deleted", nms, DESC);
    let mut l = Line::new();
    l.s("  BootOrder now:");
    order_line(&mut l, &order[..n]);
    l.s("  -> after:");
    order_line(&mut l, &new[..k]);
    crate::kprintln!("{}", l.str());
    crate::kprintln!("  the file in the EFI partition stays (bootcopy remove takes it out)");
    if !agreed("bootentry remove", given, code(b"bootentry remove", num, &opt[..olen], &ob[..oblen])) {
        return;
    }
    if k != n {
        let st = uefi::set_global(b"BootOrder", &nb[..nlen]);
        if st != uefi::SUCCESS || !reads_back(b"BootOrder", &nb[..nlen]) {
            crate::kprintln!("bootentry remove: BootOrder could not be written (status {:#x}), so {} is kept; bootentries shows what is there", st, nms);
            uefi::reread_boot_entries();
            return;
        }
    }
    let st = uefi::set_global(&nm, &[]);
    if st != uefi::SUCCESS || !reads_back(&nm, &[]) {
        crate::kprintln!("bootentry remove: the firmware did not delete {} (status {:#x}), but it is out of BootOrder; bootentries shows what is there", nms, st);
        uefi::reread_boot_entries();
        return;
    }
    uefi::reread_boot_entries();
    crate::kprintln!("bootentry remove: {} deleted and out of BootOrder, read back; the firmware's boot entries are as before bootentry", nms);
}

// --- `bootnext [yes <code>]` (PC track step 5, 7 Nov 2026) ---
//
// AfriSovera started ONCE from its own entry: BootNext = AfriSovera's Boot####. At the next
// power-on the firmware starts that entry instead of BootOrder's first, and deletes BootNext
// as it does so, so the restart after that goes back to what the PC always starts (Windows):
// the way back is built in. Only on the owner's code (tied to the entry's bytes and to
// BootNext as the firmware has it now); read back after writing.

/// `bootnext [yes <code>]`
pub fn next(given: Option<&str>) {
    let Some((_, ours, first)) = numbers() else {
        crate::kprintln!("bootnext: the firmware's boot entries could not be read at power-on; nothing written");
        return;
    };
    let Some(num) = ours else {
        crate::kprintln!("bootnext: AfriSovera has no boot entry (bootentry adds it first); nothing written");
        return;
    };
    let nm = name(num);
    let nms = core::str::from_utf8(&nm).unwrap_or("?");
    let mut opt = [0u8; uefi::OPT_MAX];
    let olen = match uefi::get_global(&nm, &mut opt) {
        Ok(n) => n,
        Err(_) => {
            crate::kprintln!("bootnext: {} could not be read from the firmware now; nothing written", nms);
            return;
        }
    };
    let mut nb = [0u8; 2];
    let now = match uefi::get_global(b"BootNext", &mut nb) {
        Ok(2) => Some(u16le(&nb, 0)),
        Err(uefi::ERR_NOT_FOUND) => None,
        _ => {
            crate::kprintln!("bootnext: the firmware's BootNext could not be read now; nothing written");
            return;
        }
    };
    if now == Some(num) {
        crate::kprintln!("bootnext: BootNext already starts {} \"{}\" at the next power-on; nothing to do", nms, DESC);
        return;
    }
    let mut l = Line::new();
    l.s("bootnext: the next power-on starts ");
    l.s(nms);
    l.s(" \"AfriSovera\" ONCE (BootNext now: ");
    match now {
        Some(v) => {
            for c in hex4(v) {
                l.byte(c);
            }
        }
        None => l.s("none"),
    }
    l.s(" -> ");
    for c in hex4(num) {
        l.byte(c);
    }
    l.byte(b')');
    crate::kprintln!("{}", l.str());
    match first {
        Some((_, d, dn)) => {
            crate::kprintln!(
                "  the firmware deletes BootNext as it starts it; the power-on after that follows BootOrder again (first in it: \"{}\")",
                core::str::from_utf8(&d[..dn]).unwrap_or("?")
            );
            // the places before it the firmware passes over (12 Nov 2026), as bootplan says them
            passed_over();
        }
        None => crate::kprintln!("  the firmware deletes BootNext as it starts it; the power-on after that follows BootOrder again"),
    }
    let nowb: &[u8] = if now.is_some() { &nb } else { &[] };
    if !agreed("bootnext", given, code(b"bootnext", num, &opt[..olen], nowb)) {
        return;
    }
    let want = num.to_le_bytes();
    let st = uefi::set_global(b"BootNext", &want);
    if st != uefi::SUCCESS || !reads_back(b"BootNext", &want) {
        crate::kprintln!("bootnext: BootNext could not be written (status {:#x}); bootentries shows what is there", st);
        uefi::reread_boot_entries();
        return;
    }
    uefi::reread_boot_entries();
    crate::kprintln!("bootnext: BootNext = {} written and read back; the next power-on starts AfriSovera from its own entry, once", nms);
}
