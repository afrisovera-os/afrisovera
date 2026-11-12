//! `lsparts`: the partition tables of the PC's internal disks, READ-ONLY (PC track step 5
//! groundwork, "install alongside the old OS"). Before AfriSovera may ever sit beside
//! Windows or Linux on a disk it has to see that disk exactly as it is: the GPT (or old MBR)
//! partitions, whether the GPT's two headers and its entry array are intact (CRC32), and
//! where the free space is. Nothing here writes a sector: every read goes through the
//! drivers' `raw_read`, which has no write path.
//!
//! `installplan [MiB]` reads the same tables and says where AfriSovera's own partition WOULD
//! go: the biggest free space, 1 MiB aligned, in a free table entry, or why not (damaged
//! table, no free entry, not enough room). It is a plan only; it never resizes another
//! system's partition (room is made by shrinking Windows from Windows itself).
//!
//! `tablebackup` copies every internal disk's partition table to a USB stick (read back and
//! CRC-checked) and `tablecheck` compares that copy with the disk now: the backup an install
//! must have before it may change a table.
//!
//! `installpart <disk> <MiB>` is the only code here that WRITES an internal disk: it adds
//! AfriSovera's (empty, unformatted) partition as one new GPT entry in the free space the plan
//! names (both GPT copies, both CRCs) and only when the disk's table is healthy, the USB
//! stick holds a backup of it that matches the disk now, and the owner types the code the plan
//! prints (tied to the table as it is, so a changed disk or a typo writes nothing).
//!
//! `tablerestore <disk>` is the way back: it puts the table saved on the USB stick back onto
//! the disk (only the sectors that differ, the backup GPT copy first), again only on the
//! owner's code, and reads it back until `tablecheck` says unchanged. `tablebackup` will not
//! overwrite a saved table whose disk has CHANGED since (that copy is the way back) unless
//! the owner types `tablebackup new`.
//!
//! `formatpart nvme` / `formatpart sataN` formats AfriSovera's partition (the one installpart made) with the VFS,
//! again on the owner's code; from then on the shell's files live inside that partition and
//! the driver finds it at every boot (`ours`). Nothing outside the partition is written.

/// CRC-32 (IEEE, reflected 0xEDB88320) carried over several pieces: start with
/// `0xFFFF_FFFF`, feed bytes with `crc_feed`, finish with `!state`.
pub(crate) fn crc_feed(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn u64le(b: &[u8], o: usize) -> u64 {
    u32le(b, o) as u64 | (u32le(b, o + 4) as u64) << 32
}

/// A GUID as text ("C12A7328-F81F-11D2-BA4B-00A0C93EC93B") -> its 16 bytes on disk (the first
/// three fields little-endian, the last two as written).
const fn guid(s: &str) -> [u8; 16] {
    let t = s.as_bytes();
    let mut hex = [0u8; 16];
    let (mut i, mut n) = (0, 0);
    while i < t.len() && n < 32 {
        let c = t[i];
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 255,
        };
        if v != 255 {
            hex[n / 2] |= if n % 2 == 0 { v << 4 } else { v };
            n += 1;
        }
        i += 1;
    }
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = [0u8; 16];
    let mut k = 0;
    while k < 16 {
        out[k] = hex[order[k]];
        k += 1;
    }
    out
}

const TYPES: [([u8; 16], &str); 10] = [
    (guid("C12A7328-F81F-11D2-BA4B-00A0C93EC93B"), "EFI system"),
    (guid("E3C9E316-0B5C-4DB8-817D-F92DF00215AE"), "Microsoft reserved"),
    (guid("EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"), "Windows/basic data"),
    (guid("DE94BBA4-06D1-4D40-A16A-BFD50179D6AC"), "Windows recovery"),
    (guid("0FC63DAF-8483-4772-8E79-3D69D8477DE4"), "Linux filesystem"),
    (guid("0657FD6D-A4AB-43C4-84E5-0933C84B4F4F"), "Linux swap"),
    (guid("E6D6D379-F507-44C2-A23C-238F2A3DF928"), "Linux LVM"),
    (guid("21686148-6449-6E6F-744E-656564454649"), "BIOS boot"),
    (guid("48465300-0000-11AA-AA11-00306543ECAC"), "Apple HFS+"),
    (AFRI_TYPE, "AfriSovera"),
];

/// AfriSovera's own partition type (installpart).
const AFRI_TYPE: [u8; 16] = guid("A5F1C0DE-0B5E-4A50-9AF1-50FE2A000001");

/// What `installpart` needs from a healthy GPT disk whose plan fits.
struct Plan {
    start: u64,   // first block of the new partition
    len: u64,     // its length in blocks
    slot: usize,  // the free entry, 1-based
    elba: u64,    // primary entry array (block)
    belba: u64,   // backup entry array (block)
    alt: u64,     // backup header (block)
    n_ent: u64,
    esz: u64,
    hcrc: u32,    // the primary header's CRC as it is now
}

fn type_name(g: &[u8]) -> &'static str {
    for (t, n) in TYPES.iter() {
        if g == &t[..] {
            return n;
        }
    }
    "other"
}

fn mbr_name(t: u8) -> &'static str {
    match t {
        0x01 | 0x04 | 0x06 | 0x0e => "FAT12/16",
        0x0b | 0x0c => "FAT32",
        0x07 => "NTFS/exFAT",
        0x05 | 0x0f => "extended",
        0x27 => "Windows recovery",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x8e => "Linux LVM",
        0xef => "EFI system",
        _ => "other",
    }
}

fn mib(sectors: u64, bs: u64) -> u64 {
    sectors.saturating_mul(bs) / (1024 * 1024)
}

/// The plan for one disk: `gaps` are the free (start, length) ranges in blocks of `bs` bytes,
/// `slot` the free table entry (1-based) it would use, `want` the MiB AfriSovera asks for.
fn plan(gaps: &[(u64, u64)], bs: u64, want: u64, slot: usize, what: &str) -> Option<(u64, u64)> {
    let align = (1024 * 1024 / bs).max(1);
    let mut best = (0u64, 0u64, 0u64, 0u64); // aligned start, aligned length, gap start, gap length
    for &(a, n) in gaps {
        let st = a.div_ceil(align) * align;
        let end = a + n;
        let len = if end > st { (end - st) / align * align } else { 0 };
        if len > best.1 {
            best = (st, len, a, n);
        }
    }
    let need = want * align;
    if best.1 == 0 {
        crate::kprintln!("  plan: no free space of 1 MiB or more; make room by shrinking Windows from Windows itself (Disk Management, Shrink Volume)");
    } else if best.1 < need {
        crate::kprintln!(
            "  plan: the largest free space holds {} MiB, less than the {} MiB asked; make room by shrinking Windows from Windows itself (Disk Management, Shrink Volume)",
            best.1 / align,
            want
        );
    } else {
        crate::kprintln!(
            "  plan: AfriSovera's partition would be LBA {}-{} ({} MiB) in the free space LBA {}-{}, as {} {}; no other partition moves or shrinks",
            best.0,
            best.0 + need - 1,
            want,
            best.2,
            best.2 + best.3 - 1,
            what,
            slot
        );
        return Some((best.0, need));
    }
    None
}

/// One disk: `rd` reads 512-byte sectors, `bs` is the disk's own block size (GPT counts in it).
/// `want` > 0: also print the install plan for a partition of `want` MiB.
fn show(rd: &mut dyn FnMut(u64, &mut [u8; 512]) -> bool, cap512: u64, bs: u64, want: u64) -> Option<Plan> {
    let per = (bs / 512).max(1);
    let blocks = cap512 / per;
    let mut s = [0u8; 512];
    if !rd(0, &mut s) {
        crate::kprintln!("  could not read sector 0");
        return None;
    }
    if s.iter().all(|&b| b == 0) {
        crate::kprintln!("  blank: no partition table");
        if want > 0 {
            crate::kprintln!("  plan: blank disk; AfriSovera can have all of it, only on the owner's word (claimdisk)");
        }
        return None;
    }
    if &s[..8] == b"AFRIVFS1" {
        crate::kprintln!("  AfriSovera's own files (AFRIVFS1, whole disk)");
        if want > 0 {
            crate::kprintln!("  plan: already AfriSovera's; nothing to do");
        }
        return None;
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        crate::kprintln!("  holds data but no partition table we know");
        if want > 0 {
            crate::kprintln!("  plan: none; a disk we cannot read is never changed");
        }
        return None;
    }
    let mut mbr = [(0u8, 0u64, 0u64); 4];
    for (i, e) in mbr.iter_mut().enumerate() {
        let o = 446 + 16 * i;
        *e = (s[o + 4], u32le(&s, o + 8) as u64, u32le(&s, o + 12) as u64);
    }
    if !mbr.iter().any(|e| e.0 == 0xEE) {
        let mut n = 0;
        for (i, &(t, start, len)) in mbr.iter().enumerate() {
            if t != 0 {
                n += 1;
                crate::kprintln!("  mbr {}: type {:#04x} ({}), start {}, {} sectors ({} MiB)", i + 1, t, mbr_name(t), start, len, mib(len, bs));
            }
        }
        crate::kprintln!("  mbr: {} partition(s)", n);
        if want > 0 {
            mbr_plan(&mbr, blocks, bs, want);
        }
        return None;
    }
    // GPT: the primary header in block 1, its backup in the block it names (normally the last)
    let Some(h) = header(rd, 1, per) else {
        crate::kprintln!("  gpt: protective MBR but the primary header is missing or damaged (CRC): not trusted");
        if want > 0 {
            crate::kprintln!("  plan: none; a damaged table is never changed");
        }
        return None;
    };
    let alt = u64le(&h, 32);
    let bh = if alt < blocks { header(rd, alt, per) } else { None };
    let backup = bh.is_some();
    let (first, last) = (u64le(&h, 40), u64le(&h, 48));
    let (elba, n_ent, esz) = (u64le(&h, 72), u32le(&h, 80) as u64, u32le(&h, 84) as u64);
    if esz < 128 || esz > 512 || 512 % esz != 0 || n_ent == 0 || n_ent > 1024 {
        crate::kprintln!("  gpt: entry array of {} x {} bytes not supported", n_ent, esz);
        return None;
    }
    // the entry array: CRC over all of it, the used entries listed
    let mut crc = 0xFFFF_FFFFu32;
    let mut used: [(u64, u64); 128] = [(0, 0); 128];
    let mut nu = 0usize;
    let mut free_slot = 0usize; // the first empty entry, 1-based (0: none)
    let per_sector = 512 / esz;
    let sectors = (n_ent * esz).div_ceil(512);
    let mut idx = 0u64;
    for k in 0..sectors {
        if !rd(elba * per + k, &mut s) {
            crate::kprintln!("  gpt: could not read the entry array");
            return None;
        }
        for j in 0..per_sector {
            if idx >= n_ent {
                break;
            }
            let e = &s[(j * esz) as usize..((j + 1) * esz) as usize];
            crc = crc_feed(crc, e);
            idx += 1;
            if e[..16].iter().all(|&b| b == 0) {
                if free_slot == 0 {
                    free_slot = idx as usize;
                }
                continue;
            }
            let (a, b) = (u64le(e, 32), u64le(e, 40));
            let mut name = [b' '; 36];
            let mut nl = 0;
            for c in 0..36 {
                let u = u16::from_le_bytes([e[56 + c * 2], e[57 + c * 2]]);
                if u == 0 {
                    break;
                }
                name[c] = if (0x20..0x7f).contains(&u) && u != b'"' as u16 { u as u8 } else { b'?' };
                nl = c + 1;
            }
            crate::kprintln!(
                "  part {}: {}, LBA {}-{} ({} MiB), \"{}\"",
                idx,
                type_name(&e[..16]),
                a,
                b,
                mib(b.saturating_sub(a) + 1, bs),
                core::str::from_utf8(&name[..nl]).unwrap_or("?")
            );
            if nu < used.len() {
                used[nu] = (a, b);
                nu += 1;
            }
        }
    }
    let entries_ok = !crc == u32le(&h, 88);
    crate::kprintln!(
        "  gpt: {} partition(s), usable LBA {}-{}, primary header OK, backup header {}, entries CRC {}",
        nu,
        first,
        last,
        if backup { "OK" } else { "MISSING/DAMAGED" },
        if entries_ok { "OK" } else { "BAD" }
    );
    // free space: the gaps between the used ranges inside the usable area
    let used = &mut used[..nu];
    used.sort_unstable();
    let (mut at, mut big, mut total, mut overlap) = (first, (0u64, 0u64), 0u64, false);
    let mut gaps: [(u64, u64); 129] = [(0, 0); 129];
    let mut ng = 0usize;
    for &(a, b) in used.iter() {
        if a < at {
            overlap = true; // starts inside the previous one (or before the usable area)
        } else if a > at {
            let gap = a - at;
            gaps[ng] = (at, gap);
            ng += 1;
            total += gap;
            if gap > big.1 {
                big = (at, gap);
            }
        }
        at = at.max(b.saturating_add(1));
    }
    if last >= at {
        let gap = last - at + 1;
        gaps[ng] = (at, gap);
        ng += 1;
        total += gap;
        if gap > big.1 {
            big = (at, gap);
        }
    }
    if big.1 == 0 {
        crate::kprintln!("  free: none");
    } else {
        crate::kprintln!(
            "  free: {} MiB in all, largest LBA {}-{} ({} MiB)",
            mib(total, bs),
            big.0,
            big.0 + big.1 - 1,
            mib(big.1, bs)
        );
    }
    if overlap {
        crate::kprintln!("  WARNING: partitions overlap; this table is not safe to change");
    }
    if !backup || !entries_ok {
        crate::kprintln!("  WARNING: this GPT is damaged; not safe to change");
    }
    if want > 0 {
        if overlap || !backup || !entries_ok {
            crate::kprintln!("  plan: none; a damaged table is never changed");
        } else if free_slot == 0 {
            crate::kprintln!("  plan: none; all {} GPT entries are in use", n_ent);
        } else if let (Some((start, len)), Some(b)) = (plan(&gaps[..ng], bs, want, free_slot, "GPT entry"), bh) {
            return Some(Plan { start, len, slot: free_slot, elba, belba: u64le(&b, 72), alt, n_ent, esz, hcrc: u32le(&h, 16) });
        }
    }
    None
}

/// The plan on an old MBR disk: a free primary slot and the free space between partitions,
/// inside the first 2^32 sectors (all an MBR can address).
fn mbr_plan(mbr: &[(u8, u64, u64); 4], blocks: u64, bs: u64, want: u64) {
    if mbr.iter().any(|e| matches!(e.0, 0x05 | 0x0f | 0x85)) {
        crate::kprintln!("  plan: none; this MBR has an extended partition (logical partitions are not handled yet)");
        return;
    }
    let Some(slot) = mbr.iter().position(|e| e.0 == 0) else {
        crate::kprintln!("  plan: none; all 4 MBR slots are in use");
        return;
    };
    let mut used: [(u64, u64); 4] = [(0, 0); 4];
    let mut nu = 0;
    for &(t, a, n) in mbr.iter() {
        if t != 0 && n > 0 {
            used[nu] = (a, a + n - 1);
            nu += 1;
        }
    }
    let used = &mut used[..nu];
    used.sort_unstable();
    let end = blocks.min(1 << 32); // first block past what the MBR can address
    let (mut at, mut gaps, mut ng) = (1u64, [(0u64, 0u64); 5], 0usize);
    for &(a, b) in used.iter() {
        if a < at || b >= end {
            crate::kprintln!("  plan: none; the MBR partitions overlap or run past the disk");
            return;
        }
        if a > at {
            gaps[ng] = (at, a - at);
            ng += 1;
        }
        at = b + 1;
    }
    if end > at {
        gaps[ng] = (at, end - at);
        ng += 1;
    }
    let _ = plan(&gaps[..ng], bs, want, slot + 1, "MBR slot");
}

/// The GPT header in block `lba`, if its signature and CRC are right.
pub(crate) fn header(rd: &mut dyn FnMut(u64, &mut [u8; 512]) -> bool, lba: u64, per: u64) -> Option<[u8; 512]> {
    let mut h = [0u8; 512];
    if !rd(lba * per, &mut h) || &h[..8] != b"EFI PART" {
        return None;
    }
    let size = u32le(&h, 12) as usize;
    if !(92..=512).contains(&size) || u64le(&h, 24) != lba {
        return None;
    }
    let want = u32le(&h, 16);
    let mut c = h;
    c[16..20].fill(0);
    if !crc_feed(0xFFFF_FFFF, &c[..size]) != want {
        return None;
    }
    Some(h)
}

/// `lsparts` (`want` 0) or `installplan <want MiB>`: every internal disk the PC's drivers
/// found, its partitions and free space, and with `want` where AfriSovera would go.
pub fn list(want: u64) {
    let mut n = 0;
    if let Some((cap, bs)) = crate::nvme::raw_info() {
        n += 1;
        crate::kprintln!("lsparts: nvme namespace 1, {} MiB, {}-byte blocks", cap / 2048, bs);
        show(&mut |k, o| crate::nvme::raw_read(k, o), cap, bs, want);
    }
    let mut i = 0;
    while let Some((p, cap, bs)) = crate::ahci::raw_disk(i) {
        n += 1;
        crate::kprintln!("lsparts: sata port {}, {} MiB, {}-byte sectors", p, cap / 2048, bs);
        show(&mut |k, o| crate::ahci::raw_read(p, k, o), cap, bs, want);
        i += 1;
    }
    let mut i = 0;
    while let Some((k, cap, bs)) = crate::ide::raw_disk(i) {
        n += 1;
        crate::kprintln!("lsparts: ide{} (SATA in IDE mode), {} MiB, {}-byte sectors", k, cap / 2048, bs);
        show(&mut |s, o| crate::ide::raw_read(k, s, o), cap, bs, want);
        i += 1;
    }
    if let Some((cap, bs)) = crate::sdhci::raw_info() {
        n += 1;
        crate::kprintln!("lsparts: emmc (built in), {} MiB, {}-byte blocks", cap / 2048, bs);
        show(&mut |k, o| crate::sdhci::raw_read(k, o), cap, bs, want);
    }
    if n == 0 {
        crate::kprintln!("lsparts: no NVMe, SATA, IDE-mode or eMMC disk");
    }
    crate::kprintln!("lsparts: read only; nothing was written");
    if want > 0 {
        crate::kprintln!("installplan: a plan only; installing will ask for the owner's word and back up the table first");
    }
}

// --- `tablebackup` / `tablecheck`: the partition tables saved on a USB stick before any
// install may change them (roadmap step 5, "with a backup of the original first"). The
// internal disks are only READ here; the only disk written is a USB stick the PC already
// treats as AfriSovera's (blank or AFRIVFS1, never the stick the PC booted from). The
// stick's sectors 0-48 hold its files, 100 the self-test and 128-871 the part of files over 512
// bytes (claude/vfs-big), so the backup starts at 1 MiB:
// sector BAK_AT is a directory ("AFRIPTBK", count, time), then one slot per disk: a record
// sector (label, size, the copied sector runs, CRC32 of the data, CRC32 of the record) and
// the copied sectors, exactly as they are on the disk.

const BAK_AT: u64 = 2048;
const BAK_SLOT: u64 = 260; // a record sector + up to 259 copied sectors
const BAK_DISKS: usize = 8;

#[derive(Clone, Copy)]
pub(crate) enum Disk {
    Nvme,
    Sata(u32),
    /// the built-in eMMC of a cheap laptop (sdhci.rs, 8 Nov 2026)
    Emmc,
    /// a SATA disk in IDE mode, by its drive number in lside (ide.rs, 10 Nov 2026)
    Ide(u32),
}

impl Disk {
    pub(crate) fn read(self, n: u64, out: &mut [u8; 512]) -> bool {
        match self {
            Disk::Nvme => crate::nvme::raw_read(n, out),
            Disk::Sata(p) => crate::ahci::raw_read(p, n, out),
            Disk::Emmc => crate::sdhci::raw_read(n, out),
            Disk::Ide(k) => crate::ide::raw_read(k, n, out),
        }
    }

    /// "nvme" or "sata port N", zero padded: the name a backup is matched by later.
    pub(crate) fn label(self) -> [u8; 32] {
        let mut l = [0u8; 32];
        match self {
            Disk::Nvme => l[..4].copy_from_slice(b"nvme"),
            Disk::Emmc => l[..4].copy_from_slice(b"emmc"),
            Disk::Ide(k) => {
                l[..3].copy_from_slice(b"ide");
                l[3] = b'0' + (k % 10) as u8;
            }
            Disk::Sata(p) => {
                l[..10].copy_from_slice(b"sata port ");
                let p = p % 100;
                if p >= 10 {
                    l[10] = b'0' + (p / 10) as u8;
                    l[11] = b'0' + (p % 10) as u8;
                } else {
                    l[10] = b'0' + p as u8;
                }
            }
        }
        l
    }
}

pub(crate) fn label_str(l: &[u8]) -> &str {
    let end = l.iter().position(|&c| c == 0).unwrap_or(l.len());
    core::str::from_utf8(&l[..end]).unwrap_or("?")
}

/// Every internal disk the drivers found: (disk, 512-byte sectors, block bytes).
pub(crate) fn disks() -> ([(Disk, u64, u64); BAK_DISKS], usize) {
    let mut d = [(Disk::Nvme, 0u64, 0u64); BAK_DISKS];
    let mut n = 0;
    if let Some((cap, bs)) = crate::nvme::raw_info() {
        d[n] = (Disk::Nvme, cap, bs);
        n += 1;
    }
    let mut i = 0;
    while let Some((p, cap, bs)) = crate::ahci::raw_disk(i) {
        if n < BAK_DISKS {
            d[n] = (Disk::Sata(p), cap, bs);
            n += 1;
        }
        i += 1;
    }
    let mut i = 0;
    while let Some((k, cap, bs)) = crate::ide::raw_disk(i) {
        if n < BAK_DISKS {
            d[n] = (Disk::Ide(k), cap, bs);
            n += 1;
        }
        i += 1;
    }
    if let Some((cap, bs)) = crate::sdhci::raw_info() {
        if n < BAK_DISKS {
            d[n] = (Disk::Emmc, cap, bs);
            n += 1;
        }
    }
    (d, n)
}

/// The 512-byte sector runs (start, count) that hold a disk's partition table, its kind,
/// or why there is nothing to back up.
fn table_runs(dk: Disk, cap512: u64, bs: u64) -> Result<([(u64, u64); 5], usize, &'static str), &'static str> {
    let per = (bs / 512).max(1);
    let blocks = cap512 / per;
    let mut s = [0u8; 512];
    if !dk.read(0, &mut s) {
        return Err("could not read sector 0");
    }
    if s.iter().all(|&b| b == 0) {
        return Err("blank, no partition table to back up");
    }
    if &s[..8] == b"AFRIVFS1" {
        return Err("AfriSovera's own files, no partition table to back up");
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        return Err("no partition table we know");
    }
    let mut runs = [(0u64, 0u64); 5];
    runs[0] = (0, 1);
    if !(0..4).any(|i| s[446 + 16 * i + 4] == 0xEE) {
        return Ok((runs, 1, "MBR"));
    }
    let mut rd = |k: u64, o: &mut [u8; 512]| dk.read(k, o);
    let Some(h) = header(&mut rd, 1, per) else {
        return Err("the GPT's primary header is damaged (a damaged table is never changed)");
    };
    let (n_ent, esz) = (u32le(&h, 80) as u64, u32le(&h, 84) as u64);
    let ns = (n_ent * esz).div_ceil(512);
    runs[1] = (per, 1);
    runs[2] = (u64le(&h, 72) * per, ns);
    let mut nr = 3;
    let alt = u64le(&h, 32);
    if alt < blocks {
        if let Some(b) = header(&mut rd, alt, per) {
            runs[3] = (u64le(&b, 72) * per, ns);
            runs[4] = (alt * per, 1);
            nr = 5;
        }
    }
    let total: u64 = runs[..nr].iter().map(|r| r.1).sum();
    if total > BAK_SLOT - 1 || runs[..nr].iter().any(|r| r.0.saturating_add(r.1) > cap512) {
        return Err("GPT entry array too big to back up");
    }
    Ok((runs, nr, if nr == 5 { "GPT" } else { "GPT (backup header missing)" }))
}

/// `tablebackup` (`fresh`: `tablebackup new`): every internal disk's partition table copied to
/// the USB stick, read back and checked. The internal disks are only read. A saved table whose
/// disk has changed since is the way back, so it is only overwritten with `new`.
pub fn backup(fresh: bool) {
    if !crate::xhci::msc_present() {
        crate::kprintln!("tablebackup: no USB stick to hold the backup (plug in a blank stick or one of AfriSovera's); nothing written");
        return;
    }
    let (ds, nd) = disks();
    if nd == 0 {
        crate::kprintln!("tablebackup: no NVMe or SATA disk");
        return;
    }
    if let (false, Some(count)) = (fresh, backups()) {
        for i in 0..count {
            if let (l, Cmp::Changed(_)) = compare(i, &ds[..nd]) {
                crate::kprintln!(
                    "tablebackup: {}: the USB stick holds this disk's table from BEFORE it changed; that copy is the way back (tablerestore), so nothing was written. To save the tables as they are now instead, type: tablebackup new",
                    label_str(&l)
                );
                return;
            }
        }
    }
    if crate::xhci::msc_capacity() < BAK_AT + 1 + nd as u64 * BAK_SLOT {
        crate::kprintln!("tablebackup: the USB stick is too small for the backup; nothing written");
        return;
    }
    let now = crate::cmos::epoch() as u64;
    let mut saved = 0u64;
    let mut s = [0u8; 512];
    for &(dk, cap, bs) in &ds[..nd] {
        let lab = dk.label();
        let (runs, nr, kind) = match table_runs(dk, cap, bs) {
            Ok(r) => r,
            Err(why) => {
                crate::kprintln!("tablebackup: {}: {}, skipped", label_str(&lab), why);
                continue;
            }
        };
        let base = BAK_AT + 1 + saved * BAK_SLOT;
        let (mut crc, mut k, mut ok) = (0xFFFF_FFFFu32, 0u64, true);
        for &(a, n) in &runs[..nr] {
            for j in 0..n {
                ok = ok && dk.read(a + j, &mut s) && crate::xhci::msc_write(base + 1 + k, &s);
                crc = crc_feed(crc, &s);
                k += 1;
            }
        }
        let mut r = [0u8; 512];
        r[..8].copy_from_slice(b"AFRIPTB1");
        r[8..40].copy_from_slice(&lab);
        r[40..48].copy_from_slice(&cap.to_le_bytes());
        r[48..56].copy_from_slice(&bs.to_le_bytes());
        r[56..64].copy_from_slice(&now.to_le_bytes());
        r[64..68].copy_from_slice(&(!crc).to_le_bytes());
        r[68..72].copy_from_slice(&(nr as u32).to_le_bytes());
        for (i, &(a, n)) in runs[..nr].iter().enumerate() {
            r[72 + 16 * i..80 + 16 * i].copy_from_slice(&a.to_le_bytes());
            r[80 + 16 * i..88 + 16 * i].copy_from_slice(&n.to_le_bytes());
        }
        let rc = !crc_feed(0xFFFF_FFFF, &r[..504]);
        r[504..508].copy_from_slice(&rc.to_le_bytes());
        ok = ok && crate::xhci::msc_write(base, &r);
        // read the copy back from the stick and check it against the CRC
        let mut c2 = 0xFFFF_FFFFu32;
        for j in 0..k {
            ok = ok && crate::xhci::msc_read(base + 1 + j, &mut s);
            c2 = crc_feed(c2, &s);
        }
        if !ok || c2 != crc {
            crate::kprintln!("tablebackup: {}: writing or reading back the USB stick FAILED, so this backup is not to be trusted", label_str(&lab));
            continue;
        }
        crate::kprintln!(
            "tablebackup: {}: {} table, {} sector(s) saved on the USB stick at sectors {}-{}, CRC {:08x}, read back OK",
            label_str(&lab),
            kind,
            k,
            base,
            base + k,
            !crc
        );
        saved += 1;
    }
    let mut d = [0u8; 512];
    d[..8].copy_from_slice(b"AFRIPTBK");
    d[8..16].copy_from_slice(&saved.to_le_bytes());
    d[16..24].copy_from_slice(&now.to_le_bytes());
    if !crate::xhci::msc_write(BAK_AT, &d) {
        crate::kprintln!("tablebackup: writing the backup's directory on the USB stick FAILED");
        return;
    }
    crate::kprintln!("tablebackup: {} table(s) saved on the USB stick; the internal disks were only read", saved);
}

/// One backup slot on the stick compared with its disk now.
enum Cmp {
    Damaged,         // the record sector's CRC is wrong
    Gone,            // that disk is not here
    Other(u64, u64), // a different disk now (MiB now, MiB then)
    ReadFail,
    DataBad,         // the copied sectors do not match their CRC
    Changed(u64),    // first differing sector
    Same(u64),       // sectors compared
}

/// Backup slot `i` on the stick: its disk label and how it compares with that disk now.
fn compare(i: u64, ds: &[(Disk, u64, u64)]) -> ([u8; 32], Cmp) {
    let base = BAK_AT + 1 + i * BAK_SLOT;
    let (mut s, mut t, mut r) = ([0u8; 512], [0u8; 512], [0u8; 512]);
    let mut lab = [0u8; 32];
    if !crate::xhci::msc_read(base, &mut r) || &r[..8] != b"AFRIPTB1" || !crc_feed(0xFFFF_FFFF, &r[..504]) != u32le(&r, 504) {
        return (lab, Cmp::Damaged);
    }
    lab.copy_from_slice(&r[8..40]);
    let (cap, bs, nr) = (u64le(&r, 40), u64le(&r, 48), (u32le(&r, 68) as usize).min(5));
    let Some(&(dk, ncap, nbs)) = ds.iter().find(|x| x.0.label() == lab) else {
        return (lab, Cmp::Gone);
    };
    if ncap != cap || nbs != bs {
        return (lab, Cmp::Other(ncap / 2048, cap / 2048));
    }
    let (mut crc, mut k, mut diff, mut ok) = (0xFFFF_FFFFu32, 0u64, None, true);
    for j in 0..nr {
        let (a, n) = (u64le(&r, 72 + 16 * j), u64le(&r, 80 + 16 * j));
        for m in 0..n.min(BAK_SLOT - 1 - k) {
            ok = ok && crate::xhci::msc_read(base + 1 + k, &mut s) && dk.read(a + m, &mut t);
            crc = crc_feed(crc, &s);
            if diff.is_none() && s != t {
                diff = Some(a + m);
            }
            k += 1;
        }
    }
    let c = if !ok {
        Cmp::ReadFail
    } else if !crc != u32le(&r, 64) {
        Cmp::DataBad
    } else if let Some(at) = diff {
        Cmp::Changed(at)
    } else {
        Cmp::Same(k)
    };
    (lab, c)
}

/// The number of backups on the stick, or None if it holds no backup directory.
fn backups() -> Option<u64> {
    let mut d = [0u8; 512];
    if !crate::xhci::msc_present() || !crate::xhci::msc_read(BAK_AT, &mut d) || &d[..8] != b"AFRIPTBK" {
        return None;
    }
    Some(u64le(&d, 8).min(BAK_DISKS as u64))
}

/// `tablecheck`: each backup on the USB stick compared, sector by sector, with its disk now.
pub fn check() {
    let Some(count) = backups() else {
        crate::kprintln!("tablecheck: no partition-table backup on a USB stick (run tablebackup)");
        return;
    };
    let (ds, nd) = disks();
    for i in 0..count {
        let (l, c) = compare(i, &ds[..nd]);
        let lab = label_str(&l);
        match c {
            Cmp::Damaged => crate::kprintln!("tablecheck: backup {} on the USB stick is damaged (record CRC)", i + 1),
            Cmp::Gone => crate::kprintln!("tablecheck: {}: that disk is not here now", lab),
            Cmp::Other(now, then) => crate::kprintln!("tablecheck: {}: a different disk now ({} MiB, was {} MiB)", lab, now, then),
            Cmp::ReadFail => crate::kprintln!("tablecheck: {}: a read FAILED, so it was not checked", lab),
            Cmp::DataBad => crate::kprintln!("tablecheck: {}: the copy on the USB stick is damaged (data CRC)", lab),
            Cmp::Changed(at) => crate::kprintln!(
                "tablecheck: {}: CHANGED since the backup (first difference in sector {}); back it up again before any install",
                lab,
                at
            ),
            Cmp::Same(k) => crate::kprintln!("tablecheck: {}: unchanged since the backup ({} sector(s))", lab, k),
        }
    }
    crate::kprintln!("tablecheck: read only; nothing was written");
}

// --- `installpart`: the one place an internal disk is written ---

impl Disk {
    pub(crate) fn write(self, n: u64, buf: &[u8; 512]) -> bool {
        match self {
            Disk::Nvme => crate::nvme::table_write(n, buf),
            Disk::Sata(p) => crate::ahci::table_write(p, n, buf),
            Disk::Emmc => crate::sdhci::table_write(n, buf),
            Disk::Ide(k) => crate::ide::table_write(k, n, buf),
        }
    }

    /// `data` (whole 512-byte sectors, at most RUN) from sector `n` on: one command and read back on
    /// a SATA disk (AHCI or IDE mode, the IDE one also one cache flush) or an NVMe disk (installdisk,
    /// bootcopy); other disks one sector at a time as `write`.
    pub(crate) fn write_run(self, n: u64, data: &[u8]) -> bool {
        match self {
            Disk::Nvme => crate::nvme::table_write_run(n, data),
            Disk::Ide(k) => crate::ide::table_write_run(k, n, data),
            Disk::Sata(p) => crate::ahci::table_write_run(p, n, data),
            _ => data.chunks_exact(512).enumerate().all(|(j, s)| self.write(n + j as u64, s.try_into().unwrap())),
        }
    }
}

/// Sectors gathered into one `write_run`.
pub(crate) const RUN: usize = 16;

/// The owner's code for this plan: tied to the primary header's CRC (which covers the whole
/// entry array), the start and the size, so it only fits the table exactly as it was shown.
fn code(p: &Plan, want: u64) -> u32 {
    let mut b = [0u8; 20];
    b[..4].copy_from_slice(&p.hcrc.to_le_bytes());
    b[4..12].copy_from_slice(&p.start.to_le_bytes());
    b[12..20].copy_from_slice(&want.to_le_bytes());
    !crc_feed(0xFFFF_FFFF, &b) & 0xFFFF
}

/// CRC32 of the entry array at block `elba`, with the 512-byte sector `patch.0` (absolute)
/// taken from `patch.1` instead of the disk when given.
fn array_crc(dk: Disk, p: &Plan, per: u64, elba: u64, patch: Option<(u64, &[u8; 512])>) -> Option<u32> {
    let mut s = [0u8; 512];
    let (mut crc, mut left, mut k) = (0xFFFF_FFFFu32, p.n_ent * p.esz, 0u64);
    while left > 0 {
        let at = elba * per + k;
        match patch {
            Some((n, b)) if n == at => s = *b,
            _ => {
                if !dk.read(at, &mut s) {
                    return None;
                }
            }
        }
        let take = left.min(512) as usize;
        crc = crc_feed(crc, &s[..take]);
        left -= take as u64;
        k += 1;
    }
    Some(!crc)
}

/// The GPT header in block `lba` with a new entry-array CRC and its own CRC redone.
fn reheader(dk: Disk, lba: u64, per: u64, ecrc: u32) -> Option<[u8; 512]> {
    let mut rd = |k: u64, o: &mut [u8; 512]| dk.read(k, o);
    let mut h = header(&mut rd, lba, per)?;
    let size = u32le(&h, 12) as usize;
    h[88..92].copy_from_slice(&ecrc.to_le_bytes());
    h[16..20].fill(0);
    let c = !crc_feed(0xFFFF_FFFF, &h[..size]);
    h[16..20].copy_from_slice(&c.to_le_bytes());
    Some(h)
}

/// `installpart <disk> <MiB> [yes <code>]`: add AfriSovera's partition (empty, unformatted) to
/// a healthy GPT disk as the plan says, only with a matching backup on the USB stick and the
/// owner's code. Without the code it shows the plan and the code, and writes nothing.
pub fn install(which: &str, want: u64, given: Option<&str>) {
    let (ds, nd) = disks();
    let Some((dk, cap, bs)) = pick(which, &ds[..nd]) else {
        crate::kprintln!("installpart: no disk \"{}\" here; say nvme, sataN, ideN or emmc (see lsparts); nothing written", which);
        return;
    };
    let lab = dk.label();
    let per = (bs / 512).max(1);
    crate::kprintln!("installpart: {}, {} MiB, AfriSovera's partition of {} MiB", label_str(&lab), cap / 2048, want);
    let Some(p) = show(&mut |k, o| dk.read(k, o), cap, bs, want) else {
        crate::kprintln!("installpart: nothing written; only a healthy GPT disk with room in a free entry can be changed (see the plan above)");
        return;
    };
    // the backup on the USB stick must be of this disk exactly as it is now
    let verdict = match backups() {
        None => Err("there is no partition-table backup on a USB stick; run tablebackup first"),
        Some(count) => {
            let mut v = Err("the USB stick's backup does not include this disk; run tablebackup first");
            for i in 0..count {
                let (l, c) = compare(i, &ds[..nd]);
                if l == lab {
                    v = match c {
                        Cmp::Same(_) => Ok(()),
                        Cmp::Changed(_) => Err("the table CHANGED since the backup; put it back (tablerestore) or save it as it is now (tablebackup new) first"),
                        _ => Err("the backup of this disk on the USB stick cannot be trusted; run tablebackup again first"),
                    };
                }
            }
            v
        }
    };
    if let Err(why) = verdict {
        crate::kprintln!("installpart: {}; nothing written", why);
        return;
    }
    crate::kprintln!("installpart: the table matches its backup on the USB stick");
    let want_code = code(&p, want);
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want_code) => {}
        Some(_) => {
            crate::kprintln!("installpart: that code does not fit this disk's table now; nothing written");
            return;
        }
        None => {
            crate::kprintln!("installpart: nothing written yet. To write it, type exactly: installpart {} {} yes {:04x}", which, want, want_code);
            return;
        }
    }
    // both copies must hold the same entries before one entry is added to each
    let (Some(pc), Some(bc)) = (array_crc(dk, &p, per, p.elba, None), array_crc(dk, &p, per, p.belba, None)) else {
        crate::kprintln!("installpart: a read FAILED; nothing written");
        return;
    };
    if pc != bc {
        crate::kprintln!("installpart: the two GPT copies hold different entries; a damaged table is never changed; nothing written");
        return;
    }
    // the new entry: AfriSovera's type, a fresh random GUID, the planned blocks, the name
    let off = (p.slot as u64 - 1) * p.esz;
    let (psec, bsec, eo) = (p.elba * per + off / 512, p.belba * per + off / 512, (off % 512) as usize);
    let mut s = [0u8; 512];
    if !dk.read(psec, &mut s) {
        crate::kprintln!("installpart: a read FAILED; nothing written");
        return;
    }
    let e = &mut s[eo..eo + p.esz as usize];
    if e[..16].iter().any(|&b| b != 0) {
        crate::kprintln!("installpart: GPT entry {} is not free after all; nothing written", p.slot);
        return;
    }
    e.fill(0);
    e[..16].copy_from_slice(&AFRI_TYPE);
    let mut u = [0u8; 16];
    u[..8].copy_from_slice(&crate::rng::next_u64().to_le_bytes());
    u[8..].copy_from_slice(&crate::rng::next_u64().to_le_bytes());
    u[7] = (u[7] & 0x0F) | 0x40; // version 4 (random): field 3 is stored little-endian
    u[8] = (u[8] & 0x3F) | 0x80; // RFC 4122 variant
    e[16..32].copy_from_slice(&u);
    e[32..40].copy_from_slice(&p.start.to_le_bytes());
    e[40..48].copy_from_slice(&(p.start + p.len - 1).to_le_bytes());
    for (i, c) in "AfriSovera".bytes().enumerate() {
        e[56 + 2 * i] = c;
    }
    let Some(ecrc) = array_crc(dk, &p, per, p.elba, Some((psec, &s))) else {
        crate::kprintln!("installpart: a read FAILED; nothing written");
        return;
    };
    let (Some(bh), Some(ph)) = (reheader(dk, p.alt, per, ecrc), reheader(dk, 1, per, ecrc)) else {
        crate::kprintln!("installpart: a GPT header could not be read again; nothing written");
        return;
    };
    // the backup copy first, then the primary: cut off half way, the firmware still finds a
    // whole, valid primary table (the old one) and repairs the other copy from it
    let ok = dk.write(bsec, &s) && dk.write(p.alt * per, &bh) && dk.write(psec, &s) && dk.write(per, &ph);
    // read everything back
    let mut rd = |k: u64, o: &mut [u8; 512]| dk.read(k, o);
    let back = ok
        && header(&mut rd, 1, per).is_some_and(|h| u32le(&h, 88) == ecrc)
        && header(&mut rd, p.alt, per).is_some_and(|h| u32le(&h, 88) == ecrc)
        && array_crc(dk, &p, per, p.elba, None) == Some(ecrc)
        && array_crc(dk, &p, per, p.belba, None) == Some(ecrc);
    if !back {
        crate::kprintln!("installpart: writing or reading back FAILED; the table as it was is on the USB stick (tablecheck)");
        return;
    }
    crate::kprintln!(
        "installpart: AfriSovera's partition written as GPT entry {}, LBA {}-{} ({} MiB), in both GPT copies, read back OK; no other partition was changed",
        p.slot,
        p.start,
        p.start + p.len - 1,
        want
    );
    crate::kprintln!("installpart: the partition is empty (not formatted yet); the table as it was stays on the USB stick, so tablecheck now says CHANGED");
}

/// The disk the owner named ("nvme", "sataN", "ideN" or "emmc") among those found.
fn pick(which: &str, ds: &[(Disk, u64, u64)]) -> Option<(Disk, u64, u64)> {
    let w = match which {
        "nvme" => Some(Disk::Nvme),
        "emmc" => Some(Disk::Emmc),
        _ => match which.strip_prefix("ide") {
            Some(n) => n.parse::<u32>().ok().filter(|&k| k < 10).map(Disk::Ide),
            None => which.strip_prefix("sata").and_then(|n| n.parse::<u32>().ok()).map(Disk::Sata),
        },
    }?;
    ds.iter().find(|x| x.0.label() == w.label()).copied()
}

// --- `tablerestore`: the saved table put back (the way back from an install) ---

/// `tablerestore <disk> [yes <code>]`: the partition table saved on the USB stick written back
/// onto the disk it came from, when that disk has CHANGED since. Without the code it says what
/// would change and writes nothing. Only the table's own sectors that differ are written (the
/// backup GPT copy first, then the primary, then sector 0); no partition's own sectors are
/// touched, so a partition added since simply leaves the table again.
pub fn restore(which: &str, given: Option<&str>) {
    let (ds, nd) = disks();
    let Some((dk, cap, bs)) = pick(which, &ds[..nd]) else {
        crate::kprintln!("tablerestore: no disk \"{}\" here; say nvme or sataN (see lsparts); nothing written", which);
        return;
    };
    let lab = dk.label();
    let name = label_str(&lab);
    let Some(count) = backups() else {
        crate::kprintln!("tablerestore: no partition-table backup on a USB stick, so nothing to restore from; nothing written");
        return;
    };
    let mut found = None;
    for i in 0..count {
        let (l, c) = compare(i, &ds[..nd]);
        if l == lab {
            found = Some((i, c));
        }
    }
    let Some((i, c)) = found else {
        crate::kprintln!("tablerestore: the USB stick's backup does not include {}; nothing written", name);
        return;
    };
    match c {
        Cmp::Changed(at) => crate::kprintln!("tablerestore: {}: CHANGED since the backup (first difference in sector {})", name, at),
        Cmp::Same(_) => {
            crate::kprintln!("tablerestore: {}: already the same as its backup, so nothing to restore; nothing written", name);
            return;
        }
        Cmp::Other(now, then) => {
            crate::kprintln!("tablerestore: {}: a different disk now ({} MiB, was {} MiB), and a table is never put onto another disk; nothing written", name, now, then);
            return;
        }
        _ => {
            crate::kprintln!("tablerestore: {}: the copy on the USB stick cannot be trusted (damaged or unreadable); nothing written", name);
            return;
        }
    }
    // the record: which sectors were saved, and where their copies are on the stick
    let base = BAK_AT + 1 + i * BAK_SLOT;
    let (mut r, mut s, mut t) = ([0u8; 512], [0u8; 512], [0u8; 512]);
    if !crate::xhci::msc_read(base, &mut r) {
        crate::kprintln!("tablerestore: reading the USB stick FAILED; nothing written");
        return;
    }
    let nr = u32le(&r, 68) as usize;
    let order: &[usize] = match nr {
        5 => &[3, 4, 2, 1, 0], // backup array, backup header, primary array, primary header, MBR
        3 => &[2, 1, 0],
        1 => &[0],
        _ => {
            crate::kprintln!("tablerestore: the saved record has an unknown shape; nothing written");
            return;
        }
    };
    let mut runs = [(0u64, 0u64, 0u64); 5]; // (disk sector, count, its first copy on the stick)
    let mut k = 0u64;
    for (j, run) in runs[..nr].iter_mut().enumerate() {
        let (a, n) = (u64le(&r, 72 + 16 * j), u64le(&r, 80 + 16 * j));
        *run = (a, n, base + 1 + k);
        k = if a.saturating_add(n) > cap { BAK_SLOT } else { k.saturating_add(n) };
    }
    if k > BAK_SLOT - 1 {
        crate::kprintln!("tablerestore: the saved record does not fit this disk; nothing written");
        return;
    }
    // never over AfriSovera's own whole-disk files, never into a table that now lies elsewhere
    if !dk.read(0, &mut s) {
        crate::kprintln!("tablerestore: a read FAILED; nothing written");
        return;
    }
    if &s[..8] == b"AFRIVFS1" {
        crate::kprintln!("tablerestore: {} now holds AfriSovera's own files (claimdisk), and the old table would break them; nothing written", name);
        return;
    }
    if let Ok((now, nn, _)) = table_runs(dk, cap, bs) {
        if nn != nr || (0..nr).any(|j| now[j] != (runs[j].0, runs[j].1)) {
            crate::kprintln!("tablerestore: the table on {} now lies in other sectors than the saved one, so restoring could overwrite data; nothing written", name);
            return;
        }
    }
    // the code: tied to the saved copy and to the disk's table sectors exactly as they are now
    let (mut cur, mut differ) = (0xFFFF_FFFFu32, 0u64);
    for &(a, n, at) in &runs[..nr] {
        for m in 0..n {
            if !dk.read(a + m, &mut t) || !crate::xhci::msc_read(at + m, &mut s) {
                crate::kprintln!("tablerestore: a read FAILED; nothing written");
                return;
            }
            cur = crc_feed(cur, &t);
            differ += (s != t) as u64;
        }
    }
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&r[64..68]);
    b[4..].copy_from_slice(&(!cur).to_le_bytes());
    let want_code = !crc_feed(0xFFFF_FFFF, &b) & 0xFFFF;
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want_code) => {}
        Some(_) => {
            crate::kprintln!("tablerestore: that code does not fit this disk's table now; nothing written");
            return;
        }
        None => {
            crate::kprintln!("tablerestore: {} of the table's {} sector(s) differ from the saved copy; a partition added since would leave the table (its sectors are not touched)", differ, k);
            crate::kprintln!("tablerestore: nothing written yet. To put the saved table back, type exactly: tablerestore {} yes {:04x}", which, want_code);
            return;
        }
    }
    let (mut written, mut ok) = (0u64, true);
    for &j in order {
        let (a, n, at) = runs[j];
        for m in 0..n {
            ok = ok && crate::xhci::msc_read(at + m, &mut s) && dk.read(a + m, &mut t);
            if ok && s != t {
                ok = dk.write(a + m, &s);
                written += 1;
            }
        }
    }
    if !ok || !matches!(compare(i, &ds[..nd]).1, Cmp::Same(_)) {
        crate::kprintln!("tablerestore: writing or reading back FAILED; the saved copy is still on the USB stick, so try again");
        return;
    }
    crate::kprintln!(
        "tablerestore: {}: the saved table is back: {} sector(s) written, read back OK, tablecheck now says unchanged; no partition's own sectors were touched",
        name,
        written
    );
    let released = match dk {
        Disk::Nvme => crate::nvme::release_part(),
        Disk::Sata(p) => crate::ahci::release_part(p),
        Disk::Emmc => crate::sdhci::release_part(),
        Disk::Ide(k) => crate::ide::release_part(k),
    };
    if released {
        crate::kprintln!("tablerestore: AfriSovera's partition is no longer in the table, so its files are no longer kept on {} (they stay in memory until the PC is switched off)", name);
    }
}

/// AfriSovera's own partition on a healthy GPT disk, both headers and the entry array
/// CRC-checked, inside the usable area, overlapping no other entry, the only one of its type:
/// (first 512-byte sector, 512-byte sectors). Quiet: asked by the driver at boot.
fn ours(dk: Disk, cap512: u64, bs: u64) -> Option<(u64, u64)> {
    let per = (bs / 512).max(1);
    let blocks = cap512 / per;
    let mut rd = |n: u64, o: &mut [u8; 512]| dk.read(n, o);
    let mut s = [0u8; 512];
    if !rd(0, &mut s) || s[510] != 0x55 || s[511] != 0xAA || !(0..4).any(|i| s[446 + 16 * i + 4] == 0xEE) {
        return None;
    }
    let h = header(&mut rd, 1, per)?;
    let alt = u64le(&h, 32);
    if alt >= blocks {
        return None;
    }
    let b = header(&mut rd, alt, per)?;
    let (first, last) = (u64le(&h, 40), u64le(&h, 48));
    let (elba, n_ent, esz) = (u64le(&h, 72), u32le(&h, 80) as u64, u32le(&h, 84) as u64);
    if esz < 128 || esz > 512 || 512 % esz != 0 || n_ent == 0 || n_ent > 1024 || u32le(&b, 88) != u32le(&h, 88) {
        return None;
    }
    let mut crc = 0xFFFF_FFFFu32;
    let mut used: [(u64, u64); 128] = [(0, 0); 128];
    let (mut nu, mut mine, mut idx) = (0usize, None, 0u64);
    for k in 0..(n_ent * esz).div_ceil(512) {
        if !rd(elba * per + k, &mut s) {
            return None;
        }
        for j in 0..512 / esz {
            if idx >= n_ent {
                break;
            }
            let e = &s[(j * esz) as usize..((j + 1) * esz) as usize];
            crc = crc_feed(crc, e);
            idx += 1;
            if e[..16].iter().all(|&x| x == 0) {
                continue;
            }
            let r = (u64le(e, 32), u64le(e, 40));
            if e[..16] == AFRI_TYPE {
                if mine.is_some() {
                    return None; // two of ours: which one is meant is not clear
                }
                mine = Some(r);
            } else if nu < used.len() {
                used[nu] = r;
                nu += 1;
            } else {
                return None;
            }
        }
    }
    let (a, z) = mine?;
    if !crc != u32le(&h, 88) || a < first || z > last || a > z || used[..nu].iter().any(|&(p, q)| p <= z && a <= q) {
        return None;
    }
    Some((a * per, (z - a + 1) * per))
}

/// For nvme.rs at boot: AfriSovera's partition on namespace 1, if there is a healthy one.
pub fn ours_nvme() -> Option<(u64, u64)> {
    let (cap, bs) = crate::nvme::raw_info()?;
    ours(Disk::Nvme, cap, bs)
}

/// For sdhci.rs at boot: AfriSovera's partition on the built-in eMMC, if there is a healthy one.
pub fn ours_emmc() -> Option<(u64, u64)> {
    let (cap, bs) = crate::sdhci::raw_info()?;
    ours(Disk::Emmc, cap, bs)
}

/// For ide.rs at boot: AfriSovera's partition on IDE-mode drive `k`, if there is a healthy one.
pub fn ours_ide(k: u32, cap512: u64) -> Option<(u64, u64)> {
    ours(Disk::Ide(k), cap512, 512)
}

/// For ahci.rs at boot: AfriSovera's partition on the disk on SATA port `p`, if there is a healthy one.
pub fn ours_sata(p: u32, cap512: u64, bs: u64) -> Option<(u64, u64)> {
    ours(Disk::Sata(p), cap512, bs)
}

/// `formatpart <nvme|sataN> [yes <code>]`: AfriSovera's partition (made by installpart) formatted
/// with the VFS and used for the shell's files from now on, on the owner's code (tied to where
/// the partition is and what its first sector holds now). Writes only inside that partition.
pub fn format(which: &str, given: Option<&str>) {
    let (ds, nd) = disks();
    if which == "sata" {
        crate::kprintln!("formatpart: say which SATA port, e.g. formatpart sata2 (see lsparts); nothing written");
        return;
    }
    let Some((dk, cap, bs)) = pick(which, &ds[..nd]) else {
        crate::kprintln!("formatpart: no disk \"{}\" here; say nvme, sataN, ideN or emmc (see lsparts); nothing written", which);
        return;
    };
    let lab = dk.label();
    let name = label_str(&lab);
    // the disk the shell's files are on now: nvme, then SATA, then a USB stick (disk.rs)
    let nv = crate::nvme::present();
    let sa = crate::ahci::part();
    let em = crate::sdhci::internal();
    let id = crate::ide::part();
    match dk {
        Disk::Nvme if nv => {
            let (a, l) = crate::nvme::part();
            if l > 0 {
                crate::kprintln!("formatpart: nvme already keeps AfriSovera's files in its partition at sector {} ({} MiB); nothing written", a, l / 2048);
            } else {
                crate::kprintln!("formatpart: nvme already holds AfriSovera's files on the whole disk (claimdisk); nothing written");
            }
            return;
        }
        Disk::Sata(p) if sa.is_some_and(|x| x.0 == p) => {
            let (_, a, l) = sa.unwrap_or_default();
            if l > 0 {
                crate::kprintln!("formatpart: {} already keeps AfriSovera's files in its partition at sector {} ({} MiB); nothing written", name, a, l / 2048);
            } else {
                crate::kprintln!("formatpart: {} already holds AfriSovera's files on the whole disk (claimdisk); nothing written", name);
            }
            return;
        }
        Disk::Emmc if em => {
            let (a, l) = crate::sdhci::part();
            if l > 0 {
                crate::kprintln!("formatpart: emmc already keeps AfriSovera's files in its partition at sector {} ({} MiB); nothing written", a, l / 2048);
            } else {
                crate::kprintln!("formatpart: emmc already holds AfriSovera's files on the whole disk (claimdisk); nothing written");
            }
            return;
        }
        Disk::Ide(k) if id.is_some_and(|x| x.0 == k) => {
            let (_, a, l) = id.unwrap_or_default();
            if l > 0 {
                crate::kprintln!("formatpart: {} already keeps AfriSovera's files in its partition at sector {} ({} MiB); nothing written", name, a, l / 2048);
            } else {
                crate::kprintln!("formatpart: {} already holds AfriSovera's files on the whole disk (claimdisk); nothing written", name);
            }
            return;
        }
        Disk::Sata(_) | Disk::Emmc | Disk::Ide(_) if nv || sa.is_some() || id.is_some() || em || crate::sdhci::present() || crate::virtio_core::blk_present() => {
            // only one internal disk holds the files; formatting a second would be a disk nobody reads
            crate::kprintln!("formatpart: the shell's files are already kept on another internal disk; nothing written");
            return;
        }
        _ => {}
    }
    let Some((a, l)) = ours(dk, cap, bs) else {
        crate::kprintln!("formatpart: {} has no AfriSovera partition in a healthy GPT; make one with installpart first; nothing written", name);
        return;
    };
    // the VFS needs its superblock + two sectors per file; the disk self-test uses sector 100; files
    // over 512 bytes also need sectors 128-871 (a smaller partition saves them as 512 bytes)
    if l < 128 {
        crate::kprintln!("formatpart: AfriSovera's partition is only {} sector(s), too small; nothing written", l);
        return;
    }
    let mut s = [0u8; 512];
    if !dk.read(a, &mut s) {
        crate::kprintln!("formatpart: a read FAILED; nothing written");
        return;
    }
    let want_code = !crc_feed(crc_feed(crc_feed(0xFFFF_FFFF, &a.to_le_bytes()), &l.to_le_bytes()), &s) & 0xFFFF;
    let cmd = match dk {
        Disk::Nvme => "nvme",
        Disk::Emmc => "emmc",
        Disk::Sata(_) | Disk::Ide(_) => which,
    };
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want_code) => {}
        Some(_) => {
            crate::kprintln!("formatpart: that code does not fit this partition now; nothing written");
            return;
        }
        None => {
            crate::kprintln!("formatpart: {}: AfriSovera's partition, sectors {}-{} ({} MiB); whatever is in it now is lost, no other partition is touched", name, a, a + l - 1, l / 2048);
            crate::kprintln!("formatpart: nothing written yet. To format it, type exactly: formatpart {} yes {:04x}", cmd, want_code);
            return;
        }
    }
    let used = match dk {
        Disk::Nvme => crate::nvme::use_part(a, l),
        Disk::Sata(p) => crate::ahci::use_part(p, a, l),
        Disk::Emmc => crate::sdhci::use_part(a, l),
        Disk::Ide(k) => crate::ide::use_part(k, a, l),
    };
    if !used || !crate::vfs::sync_to_disk() || !dk.read(a, &mut s) || &s[..8] != b"AFRIVFS1" {
        crate::kprintln!("formatpart: writing or reading back FAILED");
        return;
    }
    crate::kprintln!("formatpart: {}: AfriSovera's partition formatted (AFRIVFS1 at sector {}, {} MiB), read back OK; the shell's files are kept there from now on and come back at the next boot", name, a, l / 2048);
}

// --- `installdisk`: a whole internal disk given to AfriSovera (a spare PC) ---
//
// For a PC that is AfriSovera's alone: the disk gets a fresh GPT with two partitions --
// an EFI system partition (FAT32, 300 MiB) holding EFI\BOOT\BOOTX64.EFI, the very file the
// kernel was started from (uefi::own_image), and AfriSovera's partition (the rest) for the
// shell's files. The firmware starts any disk holding that file, so the PC then boots
// AfriSovera with no USB stick. EVERYTHING on the disk is lost: it is shown first, and
// only the owner's code (tied to the disk's size and its first two sectors as they are now)
// writes anything.

pub(crate) const ESP_TYPE: [u8; 16] = guid("C12A7328-F81F-11D2-BA4B-00A0C93EC93B");
const ESP_MIB: u64 = 300;
static mut GPT_ARRAY: [u8; 16384] = [0; 16384];

fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

fn random_guid() -> [u8; 16] {
    let mut u = [0u8; 16];
    u[..8].copy_from_slice(&crate::rng::next_u64().to_le_bytes());
    u[8..].copy_from_slice(&crate::rng::next_u64().to_le_bytes());
    u[7] = (u[7] & 0x0F) | 0x40;
    u[8] = (u[8] & 0x3F) | 0x80;
    u
}

/// Writes whole blocks of `bs` bytes as 512-byte sectors, in order; counts what it wrote.
/// Sectors that follow each other are gathered (up to RUN) and sent with one `write_run` when the
/// next one does not follow, the run is full, or at `done` (call it before reading `ok` or `n`).
struct BlockWriter {
    dk: Disk,
    per: u64,
    ok: bool,
    n: u64,
    at: u64,
    k: usize,
}

/// BlockWriter's gathered run (shell context only, one writer at a time).
static mut GATHER: [u8; RUN * 512] = [0; RUN * 512];

impl BlockWriter {
    fn new(dk: Disk, per: u64) -> BlockWriter {
        BlockWriter { dk, per, ok: true, n: 0, at: 0, k: 0 }
    }

    /// Block `blk` (absolute) = `data` (at most one block), zero filled.
    fn block(&mut self, blk: u64, data: &[u8]) {
        for j in 0..self.per {
            if !self.ok {
                return;
            }
            let x = blk * self.per + j;
            if self.k > 0 && (x != self.at + self.k as u64 || self.k == RUN) {
                self.done();
            }
            if self.k == 0 {
                self.at = x;
            }
            // SAFETY: shell context only; one BlockWriter at a time.
            let s = unsafe { &mut (&mut *core::ptr::addr_of_mut!(GATHER))[self.k * 512..self.k * 512 + 512] };
            s.fill(0);
            let o = (j * 512) as usize;
            if o < data.len() {
                let e = (o + 512).min(data.len());
                s[..e - o].copy_from_slice(&data[o..e]);
            }
            self.k += 1;
        }
    }

    /// Send the gathered run.
    fn done(&mut self) {
        let k = core::mem::take(&mut self.k);
        if k == 0 || !self.ok {
            return;
        }
        // SAFETY: as in block.
        let run = unsafe { &(&*core::ptr::addr_of!(GATHER))[..k * 512] };
        if self.dk.write_run(self.at, run) {
            self.n += k as u64;
        } else {
            self.ok = false;
        }
    }
}

/// The owner's code for a whole-disk install: the disk's size, block size and its first two
/// 512-byte sectors as they are now.
fn disk_code(dk: Disk, cap: u64, bs: u64) -> Option<u32> {
    let (mut a, mut b) = ([0u8; 512], [0u8; 512]);
    if !dk.read(0, &mut a) || !dk.read(1, &mut b) {
        return None;
    }
    let mut c = crc_feed(0xFFFF_FFFF, b"installdisk");
    c = crc_feed(c, &cap.to_le_bytes());
    c = crc_feed(c, &bs.to_le_bytes());
    c = crc_feed(crc_feed(c, &a), &b);
    Some(!c & 0xFFFF)
}

/// One 32-byte FAT directory entry.
pub(crate) fn dirent(d: &mut [u8], name: &[u8; 11], attr: u8, clus: u32, size: u32) {
    const DATE: u16 = ((2026 - 1980) << 9) | (11 << 5) | 6;
    d[..32].fill(0);
    d[..11].copy_from_slice(name);
    d[11] = attr;
    put16(d, 14, 12 << 11); // created 12:00
    put16(d, 16, DATE);
    put16(d, 18, DATE);
    put16(d, 20, (clus >> 16) as u16);
    put16(d, 22, 12 << 11);
    put16(d, 24, DATE);
    put16(d, 26, clus as u16);
    put32(d, 28, size);
}

/// FAT32 with one sector (= one disk block) per cluster in the partition at block `start`,
/// `len` blocks, holding EFI\BOOT\BOOTX64.EFI = `file`. Directories: root (cluster 2), EFI (3),
/// BOOT (4); the file from cluster 5 on, in one run.
fn write_esp(w: &mut BlockWriter, start: u64, len: u64, bs: u64, file: &[u8]) -> bool {
    let b = bs as usize;
    let rsvd = 32u64;
    let mut fsz = 1u64;
    loop {
        let clusters = len - rsvd - 2 * fsz;
        let need = ((clusters + 2) * 4).div_ceil(bs);
        if need <= fsz {
            break;
        }
        fsz = need;
    }
    let clusters = len - rsvd - 2 * fsz;
    let fclus = (file.len() as u64).div_ceil(bs);
    if clusters < 65525 || 3 + fclus >= clusters {
        return false;
    }
    let data0 = start + rsvd + 2 * fsz; // the block of cluster 2
    let mut blk = [0u8; 4096];
    // boot sector (+ its copy at 6)
    let bsec = &mut blk[..b];
    bsec.fill(0);
    bsec[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bsec[3..11].copy_from_slice(b"AFRISOVR");
    put16(bsec, 11, bs as u16);
    bsec[13] = 1; // sectors per cluster
    put16(bsec, 14, rsvd as u16);
    bsec[16] = 2; // FATs
    bsec[21] = 0xF8;
    put16(bsec, 24, 63);
    put16(bsec, 26, 255);
    put32(bsec, 28, start as u32); // hidden sectors: where the partition starts
    put32(bsec, 32, len as u32);
    put32(bsec, 36, fsz as u32);
    put32(bsec, 44, 2); // root directory cluster
    put16(bsec, 48, 1); // FSInfo sector
    put16(bsec, 50, 6); // backup boot sector
    bsec[64] = 0x80;
    bsec[66] = 0x29;
    put32(bsec, 67, crate::rng::next_u64() as u32);
    bsec[71..82].copy_from_slice(b"AFRISOVERA ");
    bsec[82..90].copy_from_slice(b"FAT32   ");
    bsec[510] = 0x55;
    bsec[511] = 0xAA;
    let boot = blk;
    // FSInfo (+ its copy at 7)
    let mut info = [0u8; 4096];
    put32(&mut info, 0, 0x4161_5252);
    put32(&mut info, 484, 0x6141_7272);
    put32(&mut info, 488, (clusters - 3 - fclus) as u32);
    put32(&mut info, 492, (5 + fclus) as u32);
    put32(&mut info, 508, 0xAA55_0000);
    for r in 0..rsvd {
        match r {
            0 | 6 => w.block(start + r, &boot[..b]),
            1 | 7 => w.block(start + r, &info[..b]),
            _ => w.block(start + r, &[]),
        }
    }
    // the two FATs: 0 and 1 reserved, 2-4 the directories (one cluster each), then the file
    let last = 5 + fclus - 1; // the file's last cluster
    let per_blk = bs / 4;
    for copy in 0..2 {
        for k in 0..fsz {
            let first = k * per_blk; // the FAT entry this block starts with
            if first > last {
                w.block(start + rsvd + copy * fsz + k, &[]);
                continue;
            }
            let mut f = [0u8; 4096];
            for i in 0..per_blk {
                let c = first + i;
                let v: u32 = match c {
                    0 => 0x0FFF_FFF8,
                    1..=4 => 0x0FFF_FFFF,
                    c if c < last => (c + 1) as u32,
                    c if c == last => 0x0FFF_FFFF,
                    _ => 0,
                };
                put32(&mut f, (i * 4) as usize, v);
            }
            w.block(start + rsvd + copy * fsz + k, &f[..b]);
        }
    }
    // directories
    let mut d = [0u8; 4096];
    dirent(&mut d[0..], b"AFRISOVERA ", 0x08, 0, 0);
    dirent(&mut d[32..], b"EFI        ", 0x10, 3, 0);
    w.block(data0, &d[..b]);
    d.fill(0);
    dirent(&mut d[0..], b".          ", 0x10, 3, 0);
    dirent(&mut d[32..], b"..         ", 0x10, 0, 0);
    dirent(&mut d[64..], b"BOOT       ", 0x10, 4, 0);
    w.block(data0 + 1, &d[..b]);
    d.fill(0);
    dirent(&mut d[0..], b".          ", 0x10, 4, 0);
    dirent(&mut d[32..], b"..         ", 0x10, 3, 0);
    dirent(&mut d[64..], b"BOOTX64 EFI", 0x20, 5, file.len() as u32);
    w.block(data0 + 2, &d[..b]);
    // the file
    for k in 0..fclus {
        let o = (k * bs) as usize;
        w.block(data0 + 3 + k, &file[o..(o + b).min(file.len())]);
    }
    w.done();
    w.ok
}

/// `installdisk <nvme|sataN> [yes <code>]` (see above).
pub fn install_disk(which: &str, given: Option<&str>) {
    let (ds, nd) = disks();
    if which == "sata" {
        crate::kprintln!("installdisk: say which SATA port, e.g. installdisk sata2 (see lsparts); nothing written");
        return;
    }
    let Some((dk, cap, bs)) = pick(which, &ds[..nd]) else {
        crate::kprintln!("installdisk: no disk \"{}\" here; say nvme, sataN, ideN or emmc (see lsparts); nothing written", which);
        return;
    };
    let lab = dk.label();
    let name = label_str(&lab);
    let Some(file) = crate::uefi::own_image() else {
        crate::kprintln!("installdisk: the boot file was not kept at start-up (see the [pc] own boot file line); nothing written");
        return;
    };
    // the shell's files must not be on this disk now (they are about to be erased), and only
    // one internal disk is read for them
    let nv = crate::nvme::present();
    let sa = crate::ahci::part();
    let busy = match dk {
        Disk::Nvme => nv || crate::virtio_core::blk_present(),
        Disk::Sata(_) => nv || sa.is_some() || crate::virtio_core::blk_present(),
        // an SD card or the eMMC itself holding the files: one disk is read for them
        Disk::Emmc => nv || sa.is_some() || crate::sdhci::present() || crate::virtio_core::blk_present(),
        // an IDE-mode disk comes after SATA in disk.rs, before a USB stick or an SD card
        Disk::Ide(_) => nv || sa.is_some() || crate::ide::present() || crate::virtio_core::blk_present(),
    };
    if busy {
        crate::kprintln!("installdisk: the shell's files are kept on an internal disk now; installdisk is for a disk AfriSovera does not use yet; nothing written");
        return;
    }
    let per = (bs / 512).max(1);
    let blocks = cap / per;
    let mib = |b: u64| b * bs / (1024 * 1024);
    let ent_blocks = 16384 / bs; // 128 entries of 128 bytes
    let first = 2 + ent_blocks;
    let last = blocks.saturating_sub(2 + ent_blocks);
    let align = (1 << 20) / bs;
    let esp = (align, ESP_MIB * align);
    let afri = (esp.0 + esp.1, last.saturating_sub(esp.0 + esp.1) / align * align);
    if bs > 4096 || afri.1 < 256 * align {
        crate::kprintln!("installdisk: {} is {} MiB, too small (AfriSovera needs at least {} MiB); nothing written", name, mib(blocks), ESP_MIB + 258);
        return;
    }
    let Some(want_code) = disk_code(dk, cap, bs) else {
        crate::kprintln!("installdisk: a read FAILED; nothing written");
        return;
    };
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want_code) => {}
        Some(_) => {
            crate::kprintln!("installdisk: that code does not fit this disk as it is now; nothing written");
            return;
        }
        None => {
            crate::kprintln!("installdisk: {}, {} MiB, {}-byte blocks. What is on it now:", name, mib(blocks), bs);
            show(&mut |k, o| dk.read(k, o), cap, bs, 0);
            crate::kprintln!("installdisk: EVERYTHING on this disk will be ERASED; Windows or anything else on it is lost for good.");
            crate::kprintln!(
                "installdisk: it gets an EFI system partition ({} MiB, the boot file, {} KiB) and AfriSovera's partition ({} MiB); the PC then starts AfriSovera from it with no USB stick",
                ESP_MIB,
                file.len() / 1024,
                mib(afri.1)
            );
            crate::kprintln!("installdisk: nothing written yet. Only on a PC that is AfriSovera's alone, type exactly: installdisk {} yes {:04x}", which, want_code);
            return;
        }
    }
    crate::kprintln!("installdisk: {}: writing the EFI system partition ({} MiB) ...", name, ESP_MIB);
    let mut w = BlockWriter::new(dk, per);
    // the old tables first, so a disk cut off half way is plainly blank, never half-old
    w.block(0, &[]);
    w.block(1, &[]);
    w.block(blocks - 1, &[]);
    if !write_esp(&mut w, esp.0, esp.1, bs, file) {
        crate::kprintln!("installdisk: writing the EFI system partition FAILED after {} sectors; the disk holds no usable table now", w.n);
        return;
    }
    // AfriSovera's partition: its first block cleared (formatted below)
    w.block(afri.0, &[]);
    // the entry array
    // SAFETY: only used here, in the shell's context.
    let arr = unsafe { &mut *core::ptr::addr_of_mut!(GPT_ARRAY) };
    arr.fill(0);
    for (i, (ty, (a, l), nm)) in [(ESP_TYPE, esp, "EFI system"), (AFRI_TYPE, afri, "AfriSovera")].iter().enumerate() {
        let e = &mut arr[i * 128..(i + 1) * 128];
        e[..16].copy_from_slice(ty);
        e[16..32].copy_from_slice(&random_guid());
        put64(e, 32, *a);
        put64(e, 40, a + l - 1);
        for (k, c) in nm.bytes().enumerate() {
            e[56 + 2 * k] = c;
        }
    }
    let ecrc = !crc_feed(0xFFFF_FFFF, &arr[..]);
    let dguid = random_guid();
    let hdr = |me: u64, alt: u64, elba: u64| {
        let mut h = [0u8; 512];
        h[..8].copy_from_slice(b"EFI PART");
        put32(&mut h, 8, 0x0001_0000);
        put32(&mut h, 12, 92);
        put64(&mut h, 24, me);
        put64(&mut h, 32, alt);
        put64(&mut h, 40, first);
        put64(&mut h, 48, last);
        h[56..72].copy_from_slice(&dguid);
        put64(&mut h, 72, elba);
        put32(&mut h, 80, 128);
        put32(&mut h, 84, 128);
        put32(&mut h, 88, ecrc);
        let c = !crc_feed(0xFFFF_FFFF, &h[..92]);
        put32(&mut h, 16, c);
        h
    };
    let belba = blocks - 1 - ent_blocks;
    for k in 0..ent_blocks {
        let o = (k * bs) as usize;
        w.block(belba + k, &arr[o..o + bs as usize]);
    }
    w.block(blocks - 1, &hdr(blocks - 1, 1, belba));
    for k in 0..ent_blocks {
        let o = (k * bs) as usize;
        w.block(2 + k, &arr[o..o + bs as usize]);
    }
    w.block(1, &hdr(1, blocks - 1, 2));
    // the protective MBR last: until it is there no tool takes the disk for a GPT one
    let mut m = [0u8; 512];
    m[446 + 2] = 0x02;
    m[446 + 4] = 0xEE;
    m[446 + 5..446 + 8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    put32(&mut m, 446 + 8, 1);
    put32(&mut m, 446 + 12, (blocks - 1).min(0xFFFF_FFFF) as u32);
    m[510] = 0x55;
    m[511] = 0xAA;
    w.block(0, &m);
    w.done();
    if !w.ok {
        crate::kprintln!("installdisk: writing the partition table FAILED after {} sectors", w.n);
        return;
    }
    // read back: the table as the driver will find it at boot, and the boot file
    let found = ours(dk, cap, bs);
    let mut fileok = true;
    {
        let mut s = [0u8; 512];
        let rsvd = 32u64;
        let mut fsz = 1u64;
        loop {
            let need = ((esp.1 - rsvd - 2 * fsz + 2) * 4).div_ceil(bs);
            if need <= fsz {
                break;
            }
            fsz = need;
        }
        let data0 = esp.0 + rsvd + 2 * fsz;
        let n512 = (file.len() as u64).div_ceil(512);
        for k in 0..n512 {
            if !dk.read((data0 + 3) * per + k, &mut s) {
                fileok = false;
                break;
            }
            let o = (k * 512) as usize;
            let e = (o + 512).min(file.len());
            if s[..e - o] != file[o..e] {
                fileok = false;
                break;
            }
        }
    }
    if found != Some((afri.0 * per, afri.1 * per)) || !fileok {
        crate::kprintln!("installdisk: reading back FAILED (table {}, boot file {})", if found.is_some() { "found" } else { "not found" }, if fileok { "ok" } else { "differs" });
        return;
    }
    crate::kprintln!(
        "installdisk: {}: GPT written (EFI system partition LBA {}-{}, AfriSovera's partition LBA {}-{}), boot file {} KiB written and read back OK ({} sectors)",
        name,
        esp.0,
        esp.0 + esp.1 - 1,
        afri.0,
        afri.0 + afri.1 - 1,
        file.len() / 1024,
        w.n
    );
    // the shell's files go into AfriSovera's partition from now on
    let (a, l) = (afri.0 * per, afri.1 * per);
    let used = match dk {
        Disk::Nvme => crate::nvme::use_part(a, l),
        Disk::Sata(p) => crate::ahci::use_part(p, a, l),
        Disk::Emmc => crate::sdhci::use_part(a, l),
        Disk::Ide(k) => crate::ide::use_part(k, a, l),
    };
    let mut s = [0u8; 512];
    if !used || !crate::vfs::sync_to_disk() || !dk.read(a, &mut s) || &s[..8] != b"AFRIVFS1" {
        crate::kprintln!("installdisk: the boot partition is ready, but formatting AfriSovera's partition FAILED (formatpart {} can try again)", which);
        return;
    }
    crate::kprintln!("installdisk: AfriSovera's partition formatted ({} MiB); the shell's files are kept there from now on", l / 2048);
    crate::kprintln!("installdisk: DONE; take the USB stick out and restart (reboot): the PC starts AfriSovera from this disk");
}
