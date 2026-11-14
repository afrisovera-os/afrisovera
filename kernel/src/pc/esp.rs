//! `bootplan`: where AfriSovera's boot file and its firmware boot entry WOULD go next to
//! Windows, READ ONLY (PC track step 5, 7 Nov 2026).
//!
//! Booting AfriSovera from a disk Windows shares needs two things: its boot file in the disk's
//! existing EFI system partition (the FAT32 partition the firmware starts Windows from), and
//! one more firmware boot entry pointing at that file. Before anything is ever written, this
//! reads every internal disk's EFI system partition and says what would happen: the FAT32
//! checked (boot sector, both FAT copies equal, closed cleanly), what is in \EFI, the free
//! room, where the file would go (\EFI\afrisovera\afrisovera.efi) and whether it fits, and
//! the boot entry that would be added (the next free Boot number, at the END of BootOrder so
//! the PC keeps starting Windows first). Or why not: no EFI partition, a damaged table, a
//! file system not closed cleanly (Windows' Fast Startup), FAT copies that differ, not enough
//! room. Every sector goes through the drivers' raw_read; bootplan writes nothing.
//!
//! `bootcopy [yes <code>]` (7 Nov 2026) then copies the boot file there on the owner's code --
//! free clusters, their FAT entries in both copies, two free slots of \EFI, FSInfo's count;
//! nothing already in the partition is moved or changed, and `bootcopy remove` takes it out
//! again (the way back). No firmware boot entry is added by either.

use crate::gpt::{self, Disk};

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u32le(b, o) as u64 | (u32le(b, o + 4) as u64) << 32
}

/// Where AfriSovera's boot file would go in the EFI system partition.
pub const FILE: &str = "\\EFI\\afrisovera\\afrisovera.efi";
/// The most FAT sectors (512 bytes) read to count the free room: a 64 MiB FAT, far beyond
/// any EFI system partition.
const FAT_MAX: u64 = 131_072;

/// Something that reads 512-byte sectors: a disk the drivers know, or the USB stick (firmware files,
/// 13 Nov 2026). Reading is all the FAT walk needs, so the same code reads either, and a host-side fuzz target
/// can read an image from memory.
pub(crate) trait Sect: Copy {
    fn read(&self, n: u64, out: &mut [u8; 512]) -> bool;
}

impl Sect for Disk {
    fn read(&self, n: u64, out: &mut [u8; 512]) -> bool {
        Disk::read(*self, n, out)
    }
}

/// The USB stick or disk the xHCI driver holds (shell context only, as the VFS uses it).
#[derive(Clone, Copy)]
struct Stick;

impl Sect for Stick {
    fn read(&self, n: u64, out: &mut [u8; 512]) -> bool {
        crate::xhci::msc_read(n, out)
    }
}

/// Something the FAT writer may write: an internal disk (bootcopy, on the owner's code) or the USB stick
/// AfriSovera started from (`report save`, report home 13 Nov 2026). `settle` puts what was written on the medium.
pub(crate) trait SectW: Sect {
    fn write(&self, n: u64, s: &[u8; 512]) -> bool;
    fn write_run(&self, n: u64, data: &[u8]) -> bool;
    fn settle(&self) -> bool;
}

impl SectW for Disk {
    fn write(&self, n: u64, s: &[u8; 512]) -> bool {
        Disk::write(*self, n, s)
    }
    fn write_run(&self, n: u64, data: &[u8]) -> bool {
        Disk::write_run(*self, n, data)
    }
    fn settle(&self) -> bool {
        Disk::settle(*self)
    }
}

/// The USB stick AfriSovera started from, for WRITING (architect, 14 Nov: one door). Its only constructor is
/// `report_save`, after the boot-medium and boot-file checks; every other use of the stick goes through `Stick`,
/// which implements only reading. scripts/pc-reportsave-test.sh greps that `BootStick(())` appears once.
#[derive(Clone, Copy)]
struct BootStick(());

impl Sect for BootStick {
    fn read(&self, n: u64, out: &mut [u8; 512]) -> bool {
        crate::xhci::msc_read(n, out)
    }
}

impl SectW for BootStick {
    fn write(&self, n: u64, s: &[u8; 512]) -> bool {
        crate::xhci::msc_write(n, s)
    }
    fn write_run(&self, n: u64, data: &[u8]) -> bool {
        data.chunks_exact(512).enumerate().all(|(j, s)| crate::xhci::msc_write(n + j as u64, s.try_into().unwrap()))
    }
    fn settle(&self) -> bool {
        crate::xhci::msc_settle()
    }
}

/// A FAT32 file system as its boot sector describes it. Sector numbers are 512-byte sectors
/// of the disk.
struct Fat<D: Sect = Disk> {
    dk: D,
    part: u64,     // the partition's first 512-byte sector
    bps: u64,      // bytes per FAT sector
    spc: u64,      // FAT sectors per cluster
    fat0: u64,     // the first FAT's first 512-byte sector
    fat512: u64,   // one FAT's length in 512-byte sectors
    nfats: u64,
    data0: u64,    // cluster 2's first 512-byte sector
    clusters: u64, // data clusters (numbered 2 ..= clusters + 1)
    root: u32,
    plen: u64, // the partition's length in 512-byte sectors, as opened
}

/// One directory entry: its 8.3 name, its long name if it has one (ASCII, '?' for others),
/// attributes, first cluster and size.
struct Ent {
    short: [u8; 11],
    long: [u8; 64],
    long_n: usize,
    attr: u8,
    clus: u32,
    size: u32,
}

impl Ent {
    /// The name to show: the long one, else the 8.3 one as "NAME.EXT".
    fn name(&self, out: &mut [u8; 64]) -> usize {
        if self.long_n > 0 {
            out[..self.long_n].copy_from_slice(&self.long[..self.long_n]);
            return self.long_n;
        }
        let mut n = 0;
        for &c in self.short[..8].iter().filter(|&&c| c != b' ') {
            out[n] = c;
            n += 1;
        }
        if self.short[8..].iter().any(|&c| c != b' ') {
            out[n] = b'.';
            n += 1;
            for &c in self.short[8..].iter().filter(|&&c| c != b' ') {
                out[n] = c;
                n += 1;
            }
        }
        n
    }
    fn is(&self, want: &str) -> bool {
        let mut b = [0u8; 64];
        let n = self.name(&mut b);
        b[..n].eq_ignore_ascii_case(want.as_bytes())
    }
}

impl<D: Sect> Fat<D> {
    /// The boot sector of the partition at `part` (512-byte sectors), `len` sectors long:
    /// a FAT32 we can trust, or why not.
    fn open(dk: D, part: u64, len: u64) -> Result<Fat<D>, &'static str> {
        let mut s = [0u8; 512];
        if !dk.read(part, &mut s) {
            return Err("could not read its first sector");
        }
        if s[510] != 0x55 || s[511] != 0xAA {
            return Err("holds no file system we know");
        }
        let bps = u16le(&s, 11) as u64;
        let spc = s[13] as u64;
        let rsvd = u16le(&s, 14) as u64;
        let nfats = s[16] as u64;
        let root_ents = u16le(&s, 17);
        let tot16 = u16le(&s, 19) as u64;
        let fat16sz = u16le(&s, 22);
        let tot = if tot16 != 0 { tot16 } else { u32le(&s, 32) as u64 };
        let fatsz = u32le(&s, 36) as u64;
        if !matches!(bps, 512 | 1024 | 2048 | 4096) || !spc.is_power_of_two() || spc > 128 || rsvd == 0 || !(1..=2).contains(&nfats) {
            return Err("holds no FAT file system we know");
        }
        if root_ents != 0 || fat16sz != 0 || fatsz == 0 {
            return Err("is FAT12/FAT16, not FAT32 (not changed by AfriSovera)");
        }
        let per = bps / 512;
        if tot * per > len {
            return Err("has a file system larger than the partition: not trusted");
        }
        let meta = rsvd + nfats * fatsz;
        if meta >= tot {
            return Err("has a damaged FAT32 boot sector: not trusted");
        }
        let clusters = (tot - meta) / spc;
        if clusters < 65525 {
            return Err("is too small to be FAT32 (FAT16 by its size): not changed");
        }
        if (clusters + 2) * 4 > fatsz * bps {
            return Err("has a FAT too short for its clusters: not trusted");
        }
        let root = u32le(&s, 44);
        if root < 2 || root as u64 >= clusters + 2 || u16le(&s, 42) != 0 {
            return Err("has a damaged FAT32 boot sector: not trusted");
        }
        Ok(Fat {
            dk,
            part,
            bps,
            spc,
            fat0: part + rsvd * per,
            fat512: fatsz * per,
            nfats,
            data0: part + meta * per,
            clusters,
            root,
            plen: len,
        })
    }

    fn cluster_bytes(&self) -> u64 {
        self.bps * self.spc
    }

    /// The partition's length in 512-byte sectors, as it was opened.
    fn len(&self) -> u64 {
        self.plen
    }

    /// The FAT entry of cluster `c` (first FAT copy).
    fn next(&self, c: u32) -> Option<u32> {
        let at = c as u64 * 4;
        let mut s = [0u8; 512];
        if !self.dk.read(self.fat0 + at / 512, &mut s) {
            return None;
        }
        Some(u32le(&s, (at % 512) as usize) & 0x0FFF_FFFF)
    }

    /// (free clusters, the FAT copies equal, closed cleanly), or why it cannot be counted.
    fn free(&self) -> Result<(u64, bool, bool), &'static str> {
        if self.fat512 > FAT_MAX {
            return Err("its FAT is too large to check");
        }
        let (mut a, mut b) = ([0u8; 512], [0u8; 512]);
        let (mut free, mut same, mut clean) = (0u64, true, false);
        let last = self.clusters + 1; // the highest cluster number
        for k in 0..self.fat512 {
            if !self.dk.read(self.fat0 + k, &mut a) {
                return Err("could not read its FAT");
            }
            if self.nfats == 2 {
                if !self.dk.read(self.fat0 + self.fat512 + k, &mut b) {
                    return Err("could not read its second FAT");
                }
                if a != b {
                    same = false;
                }
            }
            for j in 0..128u64 {
                let c = k * 128 + j;
                let v = u32le(&a, (j * 4) as usize) & 0x0FFF_FFFF;
                if c == 1 {
                    // bit 27: closed cleanly; bit 26: no disk errors (FAT32 only)
                    let raw = u32le(&a, 4);
                    clean = raw & 0x0800_0000 != 0 && raw & 0x0400_0000 != 0;
                } else if c >= 2 && c <= last && v == 0 {
                    free += 1;
                }
            }
            if (k + 1) * 128 > last {
                break;
            }
        }
        Ok((free, same, clean))
    }

    /// Every entry of the directory starting at cluster `c`; `f` returns true to stop.
    /// Err if the directory's cluster chain is damaged or a read fails.
    fn dir(&self, mut c: u32, f: &mut dyn FnMut(&Ent) -> bool) -> Result<(), &'static str> {
        let mut lfn = [0u8; 64];
        let mut lfn_n = 0usize;
        let mut lfn_sum: Option<u8> = None;
        let mut steps = 0u64;
        let per = self.bps / 512;
        loop {
            if c < 2 || c as u64 > self.clusters + 1 {
                return Err("a folder's cluster chain is damaged");
            }
            let first = self.data0 + (c as u64 - 2) * self.spc * per;
            for k in 0..self.spc * per {
                let mut s = [0u8; 512];
                if !self.dk.read(first + k, &mut s) {
                    return Err("could not read a folder");
                }
                for e in s.chunks(32) {
                    if e[0] == 0 {
                        return Ok(()); // the end of the directory
                    }
                    if e[0] == 0xE5 {
                        lfn_sum = None;
                        continue;
                    }
                    if e[11] == 0x0F {
                        // a long-name piece: 13 UTF-16 characters, the last piece first
                        let seq = (e[0] & 0x1F) as usize;
                        if e[0] & 0x40 != 0 {
                            lfn.fill(0);
                            lfn_n = 0;
                            lfn_sum = Some(e[13]);
                        }
                        if seq == 0 || lfn_sum != Some(e[13]) {
                            lfn_sum = None;
                            continue;
                        }
                        let offs = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
                        for (i, &o) in offs.iter().enumerate() {
                            let ch = u16le(e, o);
                            let at = (seq - 1) * 13 + i;
                            if ch == 0 || ch == 0xFFFF || at >= lfn.len() {
                                continue;
                            }
                            lfn[at] = if (0x20..0x7F).contains(&ch) { ch as u8 } else { b'?' };
                            lfn_n = lfn_n.max(at + 1);
                        }
                        continue;
                    }
                    let mut short = [0u8; 11];
                    short.copy_from_slice(&e[..11]);
                    let sum = short.iter().fold(0u8, |s, &x| (s >> 1 | s << 7).wrapping_add(x));
                    let mut ent = Ent {
                        short,
                        long: [0; 64],
                        long_n: 0,
                        attr: e[11],
                        clus: (u16le(e, 20) as u32) << 16 | u16le(e, 26) as u32,
                        size: u32le(e, 28),
                    };
                    if lfn_sum == Some(sum) && lfn_n > 0 && !lfn[..lfn_n].contains(&0) {
                        ent.long = lfn;
                        ent.long_n = lfn_n;
                    }
                    lfn_sum = None;
                    if ent.attr & 0x08 != 0 || short[0] == b'.' {
                        continue; // the volume label, "." and ".."
                    }
                    if f(&ent) {
                        return Ok(());
                    }
                }
            }
            steps += 1;
            match self.next(c) {
                Some(n) if n >= 0x0FFF_FFF8 => return Ok(()),
                Some(n) if steps <= self.clusters => c = n,
                _ => return Err("a folder's cluster chain is damaged"),
            }
        }
    }

    /// The sub-folder `name` of the folder at cluster `c`: Some(its cluster), None if absent.
    fn sub(&self, c: u32, name: &str) -> Result<Option<(u32, bool)>, &'static str> {
        let mut found = None;
        self.dir(c, &mut |e| {
            if e.is(name) {
                found = Some((e.clus, e.attr & 0x10 != 0));
                return true;
            }
            false
        })?;
        Ok(found)
    }

    /// Bytes used by the files directly in the folder at cluster `c`.
    fn bytes_in(&self, c: u32) -> Result<u64, &'static str> {
        let mut n = 0u64;
        self.dir(c, &mut |e| {
            if e.attr & 0x10 == 0 {
                n += e.size as u64;
            }
            false
        })?;
        Ok(n)
    }
}

/// An EFI system partition found in a healthy GPT: (entry number, first and last LBA, its
/// unique GUID as stored).
fn esps<D: Sect>(dk: D, cap512: u64, bs: u64, out: &mut [(u64, u64, u64, [u8; 16]); 4]) -> Result<usize, &'static str> {
    let per = (bs / 512).max(1);
    let blocks = cap512 / per;
    let mut s = [0u8; 512];
    if !dk.read(0, &mut s) {
        return Err("could not read sector 0");
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        return Err("no partition table we know");
    }
    if !(0..4).any(|i| s[446 + 16 * i + 4] == 0xEE) {
        return Err("an old MBR disk (boot entries are planned on GPT disks only)");
    }
    let mut rd = |n: u64, o: &mut [u8; 512]| dk.read(n, o);
    let Some(h) = gpt::header(&mut rd, 1, per) else {
        return Err("the GPT's primary header is damaged (a damaged table is never changed)");
    };
    if !gpt::in_disk(&h, blocks, per) {
        return Err("the GPT's header points outside the disk (a damaged table is never changed)");
    }
    let alt = u64le(&h, 32);
    if alt >= blocks || gpt::header(&mut rd, alt, per).is_none() {
        return Err("the GPT's backup header is missing or damaged (a damaged table is never changed)");
    }
    let (elba, n_ent, esz) = (u64le(&h, 72), u32le(&h, 80) as u64, u32le(&h, 84) as u64);
    if esz < 128 || esz > 512 || 512 % esz != 0 || n_ent == 0 || n_ent > 1024 {
        return Err("the GPT's entry array is not one we know");
    }
    let (mut crc, mut idx, mut n) = (0xFFFF_FFFFu32, 0u64, 0usize);
    for k in 0..(n_ent * esz).div_ceil(512) {
        if !rd(elba * per + k, &mut s) {
            return Err("could not read the GPT's entries");
        }
        for j in 0..512 / esz {
            if idx >= n_ent {
                break;
            }
            let e = &s[(j * esz) as usize..((j + 1) * esz) as usize];
            crc = gpt::crc_feed(crc, e);
            idx += 1;
            if e[..16] == gpt::ESP_TYPE && n < out.len() {
                let mut g = [0u8; 16];
                g.copy_from_slice(&e[16..32]);
                out[n] = (idx, u64le(e, 32), u64le(e, 40), g);
                n += 1;
            }
        }
    }
    if !crc != u32le(&h, 88) {
        return Err("the GPT's entries are damaged (CRC), and a damaged table is never changed");
    }
    Ok(n)
}

fn kib(b: u64) -> u64 {
    b.div_ceil(1024)
}

/// kprintln! only when `$on` (bootplan says everything; bootcopy only wants the answer).
macro_rules! say {
    ($on:expr, $($t:tt)*) => {
        if $on {
            crate::kprintln!($($t)*);
        }
    };
}

/// An EFI system partition `scan` found that may be changed: a FAT32 closed cleanly, with
/// its two FAT copies equal, in a healthy GPT.
#[derive(Clone, Copy)]
struct Spot {
    dk: Disk,
    label: [u8; 32],
    ent: u64,     // its GPT entry
    guid: [u8; 16], // its unique GUID (as stored in the entry)
    a: u64,       // its first LBA
    z: u64,       // its last LBA
    part: u64,    // its first 512-byte sector
    len: u64,     // its length in 512-byte sectors
    windows: bool,
    efi: u32,     // \EFI's cluster (0: no \EFI)
    ours: u32,    // \EFI\afrisovera's cluster (0: not there)
    free: u64,    // free clusters
}

/// Every internal disk's EFI system partitions, said aloud when `on` (bootplan): (the one the
/// boot file would be copied to, the one already holding \EFI\afrisovera).
fn scan(on: bool) -> (Option<Spot>, Option<Spot>) {
    let file = crate::uefi::own_image();
    match file {
        Some(f) => say!(on, "bootplan: AfriSovera's boot file is {} KiB (kept at power-on)", kib(f.len() as u64)),
        None => say!(on, "bootplan: AfriSovera's boot file was not kept at power-on, so the EFI partitions are shown, no plan is made"),
    }
    let (ds, nd) = gpt::disks();
    let mut chosen: Option<Spot> = None;
    let mut there: Option<Spot> = None;
    for &(dk, cap, bs) in &ds[..nd] {
        let label = dk.label();
        let lname = gpt::label_str(&label);
        let mut found = [(0u64, 0u64, 0u64, [0u8; 16]); 4];
        let n = match esps(dk, cap, bs, &mut found) {
            Ok(n) => n,
            Err(why) => {
                say!(on, "bootplan: {}: {}", lname, why);
                continue;
            }
        };
        if n == 0 {
            say!(on, "bootplan: {}: no EFI system partition", lname);
            continue;
        }
        let per = (bs / 512).max(1);
        for &(ent, a, z, guid) in &found[..n] {
            say!(
                on,
                "bootplan: {}: EFI system partition (GPT entry {}, LBA {}-{}, {} MiB)",
                lname,
                ent,
                a,
                z,
                ((z - a + 1) * per) / 2048
            );
            let fat = match Fat::open(dk, a * per, (z - a + 1) * per) {
                Ok(f) => f,
                Err(why) => {
                    say!(on, "  it {}", why);
                    continue;
                }
            };
            let (free, same, clean) = match fat.free() {
                Ok(v) => v,
                Err(why) => {
                    say!(on, "  FAT32, but {}; not changed", why);
                    continue;
                }
            };
            let cb = fat.cluster_bytes();
            say!(on, "  FAT32, {}-byte clusters, {} KiB free", cb, kib(free * cb));
            // what is in \EFI
            let efi = match fat.sub(fat.root, "EFI") {
                Ok(v) => v,
                Err(why) => {
                    say!(on, "  {}; not changed", why);
                    continue;
                }
            };
            let mut windows = false;
            let mut ours = None;
            match efi {
                Some((c, true)) => {
                    let mut l = [0u8; 160];
                    let mut ln = 0;
                    let r = fat.dir(c, &mut |e| {
                        if e.attr & 0x10 != 0 {
                            let mut b = [0u8; 64];
                            let k = e.name(&mut b);
                            if ln > 0 && ln + 2 <= l.len() {
                                l[ln..ln + 2].copy_from_slice(b", ");
                                ln += 2;
                            }
                            let k = k.min(l.len() - ln);
                            l[ln..ln + k].copy_from_slice(&b[..k]);
                            ln += k;
                            if e.is("Microsoft") {
                                windows = true;
                            }
                            if e.is("afrisovera") {
                                ours = Some(e.clus);
                            }
                        }
                        false
                    });
                    if let Err(why) = r {
                        say!(on, "  {}; not changed", why);
                        continue;
                    }
                    let what = core::str::from_utf8(&l[..ln]).unwrap_or("?");
                    say!(on, "  \\EFI holds: {}", if ln == 0 { "nothing" } else { what });
                }
                Some((_, false)) => {
                    say!(on, "  \\EFI is a file, not a folder; not changed");
                    continue;
                }
                None => say!(on, "  no \\EFI folder yet"),
            }
            if !clean {
                say!(on, "  plan: none; the file system was not closed cleanly (Windows Fast Startup or a crash): not changed until Windows has checked it");
                continue;
            }
            if !same {
                say!(on, "  plan: none; its two FAT copies differ: not changed until Windows has checked it");
                continue;
            }
            let spot = Spot {
                dk,
                label,
                ent,
                guid,
                a,
                z,
                part: a * per,
                len: (z - a + 1) * per,
                windows,
                efi: efi.map_or(0, |e| e.0),
                ours: 0,
                free,
            };
            if let Some(c) = ours {
                match fat.bytes_in(c) {
                    Ok(b) => say!(on, "  \\EFI\\afrisovera is already there ({} KiB); nothing to copy", kib(b)),
                    Err(why) => say!(on, "  \\EFI\\afrisovera is already there, but {}", why),
                }
                if there.is_none() {
                    there = Some(Spot { ours: c, ..spot });
                }
                continue;
            }
            let Some(f) = file else { continue };
            // the file's clusters + the new folder + one spare for \EFI to grow (+ \EFI itself)
            let need = (f.len() as u64).div_ceil(cb) + 2 + if efi.is_none() { 1 } else { 0 };
            if need > free {
                say!(
                    on,
                    "  plan: none; the boot file needs {} KiB here, only {} KiB free",
                    kib(need * cb),
                    kib(free * cb)
                );
                continue;
            }
            say!(
                on,
                "  plan: AfriSovera's boot file would be copied to {} ({} KiB of the {} KiB free); nothing else in this partition changes",
                FILE,
                kib(need * cb),
                kib(free * cb)
            );
            let better = match chosen {
                None => true,
                Some(c) => windows && !c.windows,
            };
            if better {
                chosen = Some(spot);
            }
        }
    }
    (chosen, there)
}

/// `bootplan`
pub fn plan() {
    let (chosen, there) = scan(true);
    let already = there.is_some();
    // a PC can have more than one EFI system partition (Windows on one disk, Linux on another): say which one
    // the boot file goes to, so bootcopy's target is never a guess (11 Nov 2026)
    if let (Some(s), None) = (chosen, there) {
        crate::kprintln!(
            "bootplan: the boot file would go to {} partition {} (start {}){}; no other EFI partition is changed",
            gpt::label_str(&s.label),
            s.ent,
            s.a,
            if s.windows { ", the one Windows starts from" } else { "" }
        );
    }
    match crate::bootent::numbers() {
        None => crate::kprintln!("  boot entry: the firmware's boot entries were not read at power-on, so none can be planned"),
        Some((_, Some(n), _)) => {
            let nm = crate::bootent::name(n);
            crate::kprintln!("  boot entry: AfriSovera already has one ({}); nothing to add", core::str::from_utf8(&nm).unwrap_or("?"));
        }
        Some((free, None, first)) => match (chosen, free) {
            (None, _) if already => crate::kprintln!("  boot entry: AfriSovera's file is there but no entry points at it yet; adding one will ask for the owner's word"),
            (None, _) => crate::kprintln!("  boot entry: none planned; no EFI system partition with room"),
            (Some(_), None) => crate::kprintln!("  boot entry: none planned; no free Boot number below Boot0100"),
            (Some(s), Some(num)) => {
                let nm = crate::bootent::name(num);
                crate::kprintln!(
                    "  boot entry: {} \"AfriSovera\" -> {} partition {} (start {}){} would be added at the END of BootOrder",
                    core::str::from_utf8(&nm).unwrap_or("?"),
                    gpt::label_str(&s.label),
                    s.ent,
                    s.a,
                    FILE
                );
                match first {
                    Some((_, d, dn)) => {
                        crate::kprintln!(
                            "  the PC keeps starting \"{}\" first; AfriSovera is picked from the firmware's boot menu",
                            core::str::from_utf8(&d[..dn]).unwrap_or("?")
                        );
                        // BootOrder's places before it that the firmware passes over, and why (12 Nov 2026)
                        crate::bootent::passed_over();
                    }
                    None => crate::kprintln!("  the PC keeps its first choice; AfriSovera is picked from the firmware's boot menu"),
                }
            }
        },
    }
    // the other systems' own entries (Windows Boot Manager, ubuntu): named, and said to stay as they are
    crate::bootent::others();
    crate::kprintln!("bootplan: a plan only, so nothing was written; adding it will ask for the owner's word, with the way back");
}

// --- `bootcopy`: the boot file copied into the EFI system partition, and taken out again ---
//
// The only writes into a partition AfriSovera does not own. What is written: free clusters
// (the file, then its folder), the FAT entries of exactly those clusters in both FAT copies,
// free slots of \EFI for the folder's entry, and FSInfo's free count. Nothing that is already
// in the partition is moved or changed; Windows' own files are only read. `bootcopy remove`
// undoes it: the entry marked deleted and the same clusters marked free again.

const EOC: u32 = 0x0FFF_FFFF;
const DIR_NAME: &str = "afrisovera";
const FILE_NAME: &str = "afrisovera.efi";

/// The checksum a long name's pieces carry of their 8.3 name.
fn short_sum(short: &[u8]) -> u8 {
    short[..11].iter().fold(0u8, |s, &x| (s >> 1 | s << 7).wrapping_add(x))
}

/// The long-name pieces of `name` (ASCII) for the 8.3 name `short`, in the order they are
/// written (the last piece first); how many.
fn lfn(name: &str, short: &[u8; 11], out: &mut [[u8; 32]; 4]) -> usize {
    let nb = name.as_bytes();
    let pieces = nb.len().div_ceil(13).min(out.len());
    let sum = short_sum(short);
    let offs = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
    for (p, e) in out.iter_mut().take(pieces).enumerate() {
        let seq = pieces - p;
        e.fill(0);
        e[0] = seq as u8 | if p == 0 { 0x40 } else { 0 };
        e[11] = 0x0F;
        e[13] = sum;
        for (i, &o) in offs.iter().enumerate() {
            let at = (seq - 1) * 13 + i;
            let ch: u16 = if at < nb.len() { nb[at] as u16 } else if at == nb.len() { 0 } else { 0xFFFF };
            e[o..o + 2].copy_from_slice(&ch.to_le_bytes());
        }
    }
    pieces
}

/// Where `Fat::take` gets a file's bytes. It asks in order, never going back, so a source may be a stream read
/// once from a USB stick (safe updates U3, AFR-0010): the file is never needed whole in memory.
pub(crate) trait Source {
    /// Fills `out` (already zeroed) with the file's bytes from byte `from`, as far as the file goes; false stops
    /// the write (nothing points at the clusters written so far).
    fn fill(&mut self, from: usize, out: &mut [u8]) -> bool;
}

/// A file already in memory (bootcopy's own boot file, report save's report, an empty folder).
impl Source for &[u8] {
    fn fill(&mut self, from: usize, out: &mut [u8]) -> bool {
        if from < self.len() {
            let to = (from + out.len()).min(self.len());
            out[..to - from].copy_from_slice(&self[from..to]);
        }
        true
    }
}

/// Fat::take's run of cluster sectors (shell context only).
static mut BIG: [u8; crate::gpt::RUN * 512] = [0; crate::gpt::RUN * 512];

impl<D: Sect> Fat<D> {
    /// 512-byte sectors per cluster.
    fn c512(&self) -> u64 {
        self.spc * self.bps / 512
    }

    /// The first 512-byte sector of cluster `c`.
    fn at(&self, c: u32) -> u64 {
        self.data0 + (c as u64 - 2) * self.c512()
    }

    /// The FAT entry of `c`, the FAT sector kept in `cache` between calls.
    fn next_c(&self, c: u32, cache: &mut (u64, [u8; 512])) -> Option<u32> {
        let k = c as u64 / 128;
        if cache.0 != k {
            if !self.dk.read(self.fat0 + k, &mut cache.1) {
                return None;
            }
            cache.0 = k;
        }
        Some(u32le(&cache.1, (c as usize % 128) * 4) & 0x0FFF_FFFF)
    }
}

impl<D: SectW> Fat<D> {
    /// FAT sector `k` (512 bytes) written to every FAT copy (they were checked equal).
    fn fat_put(&self, k: u64, s: &[u8; 512]) -> bool {
        (0..self.nfats).all(|i| self.dk.write(self.fat0 + i * self.fat512 + k, s))
    }

    /// FAT entry `c` = `v` in every copy, its top four bits kept.
    fn link(&self, c: u32, v: u32) -> bool {
        let (k, o) = (c as u64 / 128, (c as usize % 128) * 4);
        let mut s = [0u8; 512];
        if !self.dk.read(self.fat0 + k, &mut s) {
            return false;
        }
        let old = u32le(&s, o);
        s[o..o + 4].copy_from_slice(&((old & 0xF000_0000) | v).to_le_bytes());
        self.fat_put(k, &s)
    }

    /// The length of the chain from `c` (clusters), or None if it is damaged.
    fn chain_len(&self, mut c: u32) -> Option<u64> {
        let mut cache = (u64::MAX, [0u8; 512]);
        let mut n = 0u64;
        loop {
            if c < 2 || c as u64 > self.clusters + 1 || n > self.clusters {
                return None;
            }
            n += 1;
            match self.next_c(c, &mut cache)? {
                v if v >= 0x0FFF_FFF8 => return Some(n),
                0 => return None,
                v => c = v,
            }
        }
    }

    /// Writes the first `n` sectors gathered in BIG at sector `at` (one write_run).
    fn put_big(&self, at: u64, n: usize) -> bool {
        // SAFETY: shell context only; nothing else uses BIG while a copy runs.
        let run = unsafe { &(&*core::ptr::addr_of!(BIG))[..n * 512] };
        self.dk.write_run(at, run)
    }

    /// Takes `need` free clusters, lowest first, as one chain; `data` (zero filled after its
    /// end) is written into each cluster BEFORE its FAT entry changes. The chain's first cluster.
    fn take(&self, need: u64, data: &mut impl Source) -> Result<u32, &'static str> {
        let last = self.clusters + 1;
        let cb = self.cluster_bytes() as usize;
        let (mut first, mut prev, mut got) = (0u32, 0u32, 0u64);
        // the run being gathered in BIG: its first sector and how many sectors
        let (mut ps, mut pn) = (0u64, 0usize);
        let mut a = [0u8; 512];
        for k in 0..self.fat512 {
            if k * 128 > last {
                break;
            }
            if !self.dk.read(self.fat0 + k, &mut a) {
                return Err("could not read the FAT");
            }
            let mut changed = false;
            for j in 0..128u64 {
                let c = k * 128 + j;
                if c < 2 || c > last {
                    continue;
                }
                let o = (j * 4) as usize;
                let v = u32le(&a, o);
                if v & 0x0FFF_FFFF != 0 {
                    continue;
                }
                let base = self.at(c as u32);
                // the cluster's sectors join the run being gathered when they follow it on disk; the run (up
                // to RUN sectors) is one command and one flush on an IDE-mode disk. Small clusters (512 bytes
                // on a 100 MiB EFI partition) were one command and one flush each before (10 Nov 2026).
                let mut q = 0u64;
                while q < self.c512() {
                    if pn > 0 && (ps + pn as u64 != base + q || pn == crate::gpt::RUN) {
                        if !self.put_big(ps, pn) {
                            return Err("a write FAILED");
                        }
                        pn = 0;
                    }
                    if pn == 0 {
                        ps = base + q;
                    }
                    let n = (self.c512() - q).min((crate::gpt::RUN - pn) as u64) as usize;
                    // SAFETY: shell context only; nothing else uses BIG while a copy runs.
                    let run = unsafe { &mut (&mut *core::ptr::addr_of_mut!(BIG))[pn * 512..(pn + n) * 512] };
                    run.fill(0);
                    if !data.fill(got as usize * cb + q as usize * 512, run) {
                        return Err("the file could not be read");
                    }
                    pn += n;
                    q += n as u64;
                }
                a[o..o + 4].copy_from_slice(&((v & 0xF000_0000) | EOC).to_le_bytes());
                changed = true;
                if prev != 0 {
                    if prev as u64 / 128 == k {
                        let po = (prev as usize % 128) * 4;
                        let pv = u32le(&a, po);
                        a[po..po + 4].copy_from_slice(&((pv & 0xF000_0000) | c as u32).to_le_bytes());
                    } else {
                        // the data reaches the disk before any FAT entry points at it
                        if pn > 0 && !self.put_big(ps, pn) {
                            return Err("a write FAILED");
                        }
                        pn = 0;
                        if !self.link(prev, c as u32) {
                            return Err("a write FAILED");
                        }
                    }
                }
                if first == 0 {
                    first = c as u32;
                }
                prev = c as u32;
                got += 1;
                if got == need {
                    break;
                }
            }
            if pn > 0 {
                if !self.put_big(ps, pn) {
                    return Err("a write FAILED");
                }
                pn = 0;
            }
            if changed && !self.fat_put(k, &a) {
                return Err("a write FAILED");
            }
            if got == need {
                return Ok(first);
            }
        }
        Err("there was not enough free room after all")
    }

    /// Marks the chain from `c` free in every FAT copy (checked whole first by chain_len).
    fn release(&self, mut c: u32) -> bool {
        let mut k = u64::MAX;
        let mut a = [0u8; 512];
        for _ in 0..=self.clusters {
            let ck = c as u64 / 128;
            if ck != k {
                if k != u64::MAX && !self.fat_put(k, &a) {
                    return false;
                }
                if !self.dk.read(self.fat0 + ck, &mut a) {
                    return false;
                }
                k = ck;
            }
            let o = (c as usize % 128) * 4;
            let v = u32le(&a, o);
            a[o..o + 4].copy_from_slice(&(v & 0xF000_0000).to_le_bytes());
            match v & 0x0FFF_FFFF {
                n if n >= 0x0FFF_FFF8 => return self.fat_put(k, &a),
                n if n >= 2 && n as u64 <= self.clusters + 1 => c = n,
                _ => return false,
            }
        }
        false
    }

    /// Every 32-byte slot of the folder at cluster `c` (to its chain's end) as (512-byte
    /// sector, offset, slot); `f` returns true to stop.
    fn slots(&self, mut c: u32, f: &mut dyn FnMut(u64, usize, &[u8]) -> bool) -> Result<(), &'static str> {
        let mut steps = 0u64;
        loop {
            if c < 2 || c as u64 > self.clusters + 1 {
                return Err("a folder's cluster chain is damaged");
            }
            let first = self.at(c);
            for k in 0..self.c512() {
                let mut s = [0u8; 512];
                if !self.dk.read(first + k, &mut s) {
                    return Err("could not read a folder");
                }
                for (i, e) in s.chunks(32).enumerate() {
                    if f(first + k, i * 32, e) {
                        return Ok(());
                    }
                }
            }
            steps += 1;
            match self.next(c) {
                Some(n) if n >= 0x0FFF_FFF8 => return Ok(()),
                Some(n) if steps <= self.clusters => c = n,
                _ => return Err("a folder's cluster chain is damaged"),
            }
        }
    }

    /// One 32-byte slot (sector `n`, offset `o`) replaced by `e`, or its first byte by `e[0]`
    /// alone when `e` is one byte long.
    fn put_slot(&self, n: u64, o: usize, e: &[u8]) -> bool {
        let mut s = [0u8; 512];
        if !self.dk.read(n, &mut s) {
            return false;
        }
        s[o..o + e.len()].copy_from_slice(e);
        self.dk.write(n, &s)
    }

    /// FSInfo's free-cluster count moved by `delta` when the partition keeps one (its
    /// signatures right and the count known); otherwise left as it is.
    fn fsinfo(&self, delta: i64) -> bool {
        let mut s = [0u8; 512];
        if !self.dk.read(self.part, &mut s) {
            return false;
        }
        let fsi = u16le(&s, 48) as u64;
        if fsi == 0 || fsi == 0xFFFF {
            return true;
        }
        let n = self.part + fsi * (self.bps / 512);
        if !self.dk.read(n, &mut s) {
            return false;
        }
        if u32le(&s, 0) != 0x4161_5252 || u32le(&s, 484) != 0x6141_7272 || u32le(&s, 508) != 0xAA55_0000 {
            return true;
        }
        let free = u32le(&s, 488);
        if free == 0xFFFF_FFFF {
            return true;
        }
        let new = free as i64 + delta;
        let v = if new < 0 || new as u64 > self.clusters { 0xFFFF_FFFF } else { new as u32 };
        s[488..492].copy_from_slice(&v.to_le_bytes());
        self.dk.write(n, &s)
    }

    /// \EFI\afrisovera\afrisovera.efi compared with `file` sector by sector: Ok(the folder's cluster).
    fn same_file(&self, efi: u32, file: &[u8]) -> Result<u32, &'static str> {
        let Some((d, true)) = self.sub(efi, DIR_NAME)? else {
            return Err("\\EFI\\afrisovera is not there");
        };
        let mut got = None;
        self.dir(d, &mut |e| {
            if e.is(FILE_NAME) && e.attr & 0x10 == 0 {
                got = Some((e.clus, e.size));
                return true;
            }
            false
        })?;
        let Some((mut c, size)) = got else { return Err("afrisovera.efi is not in \\EFI\\afrisovera") };
        if size as usize != file.len() {
            return Err("afrisovera.efi has the wrong size");
        }
        let cb = self.cluster_bytes() as usize;
        let mut cache = (u64::MAX, [0u8; 512]);
        let mut off = 0usize;
        let mut s = [0u8; 512];
        loop {
            if c < 2 || c as u64 > self.clusters + 1 {
                return Err("afrisovera.efi's cluster chain is damaged");
            }
            for q in 0..self.c512() {
                let from = off + q as usize * 512;
                if !self.dk.read(self.at(c) + q, &mut s) {
                    return Err("could not read afrisovera.efi back");
                }
                if from < file.len() {
                    let to = (from + 512).min(file.len());
                    if s[..to - from] != file[from..to] {
                        return Err("afrisovera.efi does not read back the same");
                    }
                }
            }
            off += cb;
            let n = self.next_c(c, &mut cache).ok_or("could not read the FAT")?;
            if off >= file.len() {
                return if n >= 0x0FFF_FFF8 { Ok(d) } else { Err("afrisovera.efi's chain is too long") };
            }
            if n >= 0x0FFF_FFF8 || n == 0 {
                return Err("afrisovera.efi's chain is too short");
            }
            c = n;
        }
    }
}

/// The owner's code: tied to what is changed, the partition, its free room now and the first
/// sector of \EFI as it is now, so a changed partition (or a typo) writes nothing.
fn code(tag: &[u8], s: &Spot, fat: &Fat, extra: u64) -> Option<u32> {
    let mut b = [0u8; 512];
    if !s.dk.read(fat.at(s.efi), &mut b) {
        return None;
    }
    let mut c = gpt::crc_feed(0xFFFF_FFFF, tag);
    c = gpt::crc_feed(c, &s.label);
    c = gpt::crc_feed(c, &s.part.to_le_bytes());
    c = gpt::crc_feed(c, &s.free.to_le_bytes());
    c = gpt::crc_feed(c, &extra.to_le_bytes());
    c = gpt::crc_feed(c, &b);
    Some(!c & 0xFFFF)
}

/// Whether the owner typed this exact code; says why not.
fn agreed(cmd: &str, given: Option<&str>, want: u32) -> bool {
    match given {
        Some(c) if c.len() == 4 && u32::from_str_radix(c, 16).ok() == Some(want) => true,
        Some(_) => {
            crate::kprintln!("{}: that code does not fit this EFI partition as it is now; nothing written", cmd);
            false
        }
        None => {
            crate::kprintln!("{}: nothing written yet. To do it, type exactly: {} yes {:04x}", cmd, cmd, want);
            false
        }
    }
}

/// `bootcopy [yes <code>]`: the boot file kept at power-on copied to \EFI\afrisovera\afrisovera.efi
/// in the EFI system partition bootplan picks. No boot entry is added here.
pub fn copy(given: Option<&str>) {
    let Some(file) = crate::uefi::own_image() else {
        crate::kprintln!("bootcopy: AfriSovera's boot file was not kept at power-on, so there is nothing to copy; nothing written");
        return;
    };
    let (chosen, there) = scan(false);
    if let Some(t) = there {
        crate::kprintln!(
            "bootcopy: {} partition {} already holds \\EFI\\afrisovera; nothing to copy (bootcopy remove takes it out)",
            gpt::label_str(&t.label),
            t.ent
        );
        return;
    }
    let Some(s) = chosen else {
        crate::kprintln!("bootcopy: no EFI system partition can take the boot file (bootplan says why); nothing written");
        return;
    };
    let lname = gpt::label_str(&s.label);
    if s.efi == 0 {
        crate::kprintln!("bootcopy: {} partition {} has no \\EFI folder, so it is not changed (an EFI partition Windows uses always has one); nothing written", lname, s.ent);
        return;
    }
    let fat = match Fat::open(s.dk, s.part, s.len) {
        Ok(f) => f,
        Err(why) => {
            crate::kprintln!("bootcopy: the EFI partition {}; nothing written", why);
            return;
        }
    };
    let cb = fat.cluster_bytes();
    let nfile = (file.len() as u64).div_ceil(cb);
    // two free slots in \EFI next to each other (a long-name piece + the entry), and an 8.3
    // name no other entry there has
    let mut used = [false; 10];
    let (mut run, mut rn, mut ended) = ([(0u64, 0usize); 2], 0usize, false);
    let r = fat.slots(s.efi, &mut |n, o, e| {
        if e[0] == 0 {
            ended = true;
        }
        let free = ended || e[0] == 0xE5;
        if !free && e[11] != 0x0F && &e[..7] == b"AFRISO~" && (b'1'..=b'9').contains(&e[7]) {
            used[(e[7] - b'0') as usize] = true;
        }
        if rn < 2 {
            if free {
                run[rn] = (n, o);
                rn += 1;
            } else {
                rn = 0;
            }
        }
        ended && rn == 2
    });
    if let Err(why) = r {
        crate::kprintln!("bootcopy: {}; nothing written", why);
        return;
    }
    if rn < 2 {
        crate::kprintln!("bootcopy: \\EFI has no two free entries side by side (it would have to grow), so it is not changed; nothing written");
        return;
    }
    let Some(tilde) = (1..=9u8).find(|&i| !used[i as usize]) else {
        crate::kprintln!("bootcopy: \\EFI already has AFRISO~1 to AFRISO~9, so it is not changed; nothing written");
        return;
    };
    crate::kprintln!(
        "bootcopy: AfriSovera's boot file ({} KiB) -> {} partition {} (start {}){}",
        kib(file.len() as u64),
        lname,
        s.ent,
        s.a,
        FILE
    );
    crate::kprintln!(
        "  it takes {} KiB of the {} KiB free; nothing already in the partition is moved or changed; no boot entry is added yet",
        kib((nfile + 1) * cb),
        kib(s.free * cb)
    );
    let Some(want) = code(b"bootcopy", &s, &fat, file.len() as u64) else {
        crate::kprintln!("bootcopy: a read FAILED; nothing written");
        return;
    };
    if !agreed("bootcopy", given, want) {
        return;
    }
    // 1: the file's clusters, 2: the folder's, both written before their FAT entries change;
    // 3: the folder's entries; 4: \EFI's entry last, so a cut-off copy leaves only clusters
    // no folder points at (Windows' chkdsk frees them), never an entry pointing at nothing
    let fc = match fat.take(nfile, &mut &file[..]) {
        Ok(c) => c,
        Err(why) => {
            crate::kprintln!("bootcopy: {}; \\EFI was not changed, and chkdsk in Windows frees any clusters already taken", why);
            return;
        }
    };
    let dc = match fat.take(1, &mut &[][..]) {
        Ok(c) => c,
        Err(why) => {
            crate::kprintln!("bootcopy: {}; \\EFI was not changed, and chkdsk in Windows frees any clusters already taken", why);
            return;
        }
    };
    let mut sd = [b' '; 11];
    sd[..6].copy_from_slice(b"AFRISO");
    sd[6] = b'~';
    sd[7] = b'0' + tilde;
    let mut sf = sd;
    sf[8..].copy_from_slice(b"EFI");
    let mut pieces = [[0u8; 32]; 4];
    let mut d = [0u8; 512];
    gpt::dirent(&mut d[0..32], b".          ", 0x10, dc, 0);
    gpt::dirent(&mut d[32..64], b"..         ", 0x10, s.efi, 0);
    let np = lfn(FILE_NAME, &sf, &mut pieces);
    for (i, p) in pieces[..np].iter().enumerate() {
        d[64 + 32 * i..96 + 32 * i].copy_from_slice(p);
    }
    let fo = 64 + 32 * np;
    gpt::dirent(&mut d[fo..fo + 32], &sf, 0x20, fc, file.len() as u32);
    let mut de = [0u8; 32];
    gpt::dirent(&mut de, &sd, 0x10, dc, 0);
    let mut dp = [[0u8; 32]; 4];
    lfn(DIR_NAME, &sd, &mut dp);
    let ok = s.dk.write(fat.at(dc), &d)
        && fat.put_slot(run[0].0, run[0].1, &dp[0])
        && fat.put_slot(run[1].0, run[1].1, &de)
        && fat.fsinfo(-((nfile + 1) as i64))
        && s.dk.settle();
    if !ok {
        crate::kprintln!("bootcopy: a write FAILED part way; bootcopy remove (or chkdsk in Windows) puts it right");
        return;
    }
    // read it all back
    let back = Fat::open(s.dk, s.part, s.len).and_then(|f| {
        let d2 = f.same_file(s.efi, file)?;
        let (free, same, _) = f.free()?;
        if d2 != dc || !same || free + nfile + 1 != s.free {
            return Err("the FAT does not read back as written");
        }
        Ok(free)
    });
    match back {
        Ok(free) => {
            crate::kprintln!(
                "bootcopy: copied to {} partition {}{}, read back the same ({} KiB, both FAT copies equal, {} KiB free now)",
                lname,
                s.ent,
                FILE,
                kib(file.len() as u64),
                kib(free * cb)
            );
            crate::kprintln!("bootcopy: no boot entry was added, so the PC starts exactly as before; bootcopy remove takes the file out again");
        }
        Err(why) => crate::kprintln!("bootcopy: reading back FAILED: {}; bootcopy remove takes it out again", why),
    }
}

/// `bootcopy remove [yes <code>]`: \EFI\afrisovera taken out again, only when it holds
/// afrisovera.efi and nothing else (what bootcopy put there).
pub fn remove(given: Option<&str>) {
    let (_, there) = scan(false);
    let Some(s) = there else {
        crate::kprintln!("bootcopy remove: no EFI system partition holds \\EFI\\afrisovera that may be changed (bootplan says why); nothing written");
        return;
    };
    let lname = gpt::label_str(&s.label);
    let fat = match Fat::open(s.dk, s.part, s.len) {
        Ok(f) => f,
        Err(why) => {
            crate::kprintln!("bootcopy remove: the EFI partition {}; nothing written", why);
            return;
        }
    };
    let (mut file, mut others) = (None, 0u32);
    if let Err(why) = fat.dir(s.ours, &mut |e| {
        if e.is(FILE_NAME) && e.attr & 0x10 == 0 && file.is_none() {
            file = Some((e.clus, e.size));
        } else {
            others += 1;
        }
        false
    }) {
        crate::kprintln!("bootcopy remove: {}; nothing written", why);
        return;
    }
    if others > 0 {
        crate::kprintln!("bootcopy remove: \\EFI\\afrisovera holds other files too, so it is not removed (only what bootcopy put there is taken out); nothing written");
        return;
    }
    let cb = fat.cluster_bytes();
    let (fc, size) = file.unwrap_or((0, 0));
    let flen = if fc == 0 { Some(0) } else { fat.chain_len(fc) };
    let dlen = fat.chain_len(s.ours);
    let (Some(flen), Some(dlen)) = (flen, dlen) else {
        crate::kprintln!("bootcopy remove: a cluster chain is damaged, so it is not changed until Windows has checked it; nothing written");
        return;
    };
    if flen != (size as u64).div_ceil(cb) {
        crate::kprintln!("bootcopy remove: afrisovera.efi's chain does not fit its size, so it is not changed until Windows has checked it; nothing written");
        return;
    }
    // \EFI's entry for the folder and the long-name pieces just before it
    let mut at = [(0u64, 0usize); 5];
    let (mut an, mut found) = (0usize, 0usize);
    let r = fat.slots(s.efi, &mut |n, o, e| {
        if e[0] == 0 {
            return true;
        }
        if e[0] == 0xE5 {
            an = 0;
            return false;
        }
        if e[11] == 0x0F {
            if an < at.len() - 1 {
                at[an] = (n, o);
                an += 1;
            }
            return false;
        }
        let clus = (u16le(e, 20) as u32) << 16 | u16le(e, 26) as u32;
        if e[11] & 0x10 != 0 && clus == s.ours && e[0] != b'.' {
            // keep only the pieces that belong to this entry (their checksum)
            let sum = short_sum(e);
            let mut k = 0;
            for i in 0..an {
                let mut p = [0u8; 512];
                if s.dk.read(at[i].0, &mut p) && p[at[i].1 + 13] == sum {
                    at[k] = at[i];
                    k += 1;
                }
            }
            at[k] = (n, o);
            found = k + 1;
            return true;
        }
        an = 0;
        false
    });
    if r.is_err() || found == 0 {
        crate::kprintln!("bootcopy remove: \\EFI\\afrisovera's entry could not be found again; nothing written");
        return;
    }
    crate::kprintln!(
        "bootcopy remove: {} partition {} (start {}): \\EFI\\afrisovera ({} KiB) would be taken out; nothing else in the partition changes",
        lname,
        s.ent,
        s.a,
        kib(size as u64)
    );
    let Some(want) = code(b"bootremove", &s, &fat, (s.ours as u64) << 32 | size as u64) else {
        crate::kprintln!("bootcopy remove: a read FAILED; nothing written");
        return;
    };
    if !agreed("bootcopy remove", given, want) {
        return;
    }
    // the entry first (marked deleted, its long-name pieces too), then its clusters freed
    let mut ok = (0..found).rev().all(|i| fat.put_slot(at[i].0, at[i].1, &[0xE5]));
    ok = ok && (fc == 0 || fat.release(fc)) && fat.release(s.ours) && fat.fsinfo((flen + dlen) as i64) && s.dk.settle();
    if !ok {
        crate::kprintln!("bootcopy remove: a write FAILED part way; chkdsk in Windows puts the partition right");
        return;
    }
    let back = Fat::open(s.dk, s.part, s.len).and_then(|f| {
        let (free, same, _) = f.free()?;
        if f.sub(s.efi, DIR_NAME)?.is_some() || !same || free != s.free + flen + dlen {
            return Err("the partition does not read back as written");
        }
        Ok(free)
    });
    match back {
        Ok(free) => crate::kprintln!(
            "bootcopy remove: \\EFI\\afrisovera taken out of {} partition {}, read back (both FAT copies equal, {} KiB free again)",
            lname,
            s.ent,
            kib(free * cb)
        ),
        Err(why) => crate::kprintln!("bootcopy remove: reading back FAILED: {}", why),
    }
}

/// For `bootentry`: the EFI system partition holding \EFI\afrisovera\afrisovera.efi, checked
/// to be the very boot file kept at power-on: (disk name, GPT entry number, first LBA, LBAs,
/// unique GUID). Err says why there is none.
pub(crate) fn placed() -> Result<([u8; 32], u64, u64, u64, [u8; 16]), &'static str> {
    let Some(file) = crate::uefi::own_image() else {
        return Err("AfriSovera's boot file was not kept at power-on, so its copy cannot be checked");
    };
    let (_, there) = scan(false);
    let Some(s) = there else {
        return Err("no EFI system partition holds \\EFI\\afrisovera (bootcopy puts it there first)");
    };
    let fat = Fat::open(s.dk, s.part, s.len).map_err(|_| "the EFI partition holding it could not be opened")?;
    fat.same_file(s.efi, file).map_err(|_| "\\EFI\\afrisovera\\afrisovera.efi is not the boot file this PC was started with (bootcopy remove, then bootcopy)")?;
    Ok((s.label, s.ent, s.a, s.z - s.a + 1, s.guid))
}

// --- device firmware files, read only (13 Nov 2026, FIRMWARE_PLAN.md, architect's verdict of 13 Nov) ---
//
// A driver's firmware file lives in \EFI\afrisovera\firmware\ on an EFI system partition. Anyone who boots the PC
// can write that partition, so this is a parser of hostile storage: it takes only a name from fwdb.rs (never a
// path from a device or a person), never writes, reads at most the size fwdb.rs gives (not the size in the
// directory entry), and checks every cluster number and the chain's length. The caller hashes what comes back
// before a byte goes to a device (fwdb.rs).

/// The folders a firmware file sits in, from the partition's root.
const FW_DIR: [&str; 3] = ["EFI", "afrisovera", "firmware"];
/// The most clusters one file may use: fwdb::MAX in the smallest cluster (512 bytes).
const FW_CHAIN: usize = crate::fwdb::MAX / 512;
/// The clusters of the file being read, to find a chain that comes back on itself (shell context only).
static mut CHAIN: [u32; FW_CHAIN] = [0; FW_CHAIN];

/// What reading one firmware file gave.
pub(crate) enum FwRead {
    /// the file's bytes are in `out[..size]`
    Read,
    /// no EFI system partition has the file
    Missing,
    /// the directory entry's size differs from fwdb.rs's (nothing was read)
    WrongSize(u32),
    /// the file or the way to it is damaged
    Bad(&'static str),
}

impl<D: Sect> Fat<D> {
    /// The firmware file `name`, exactly `want` bytes, into `out[..want]`.
    fn fw_file(&self, name: &str, want: usize, out: &mut [u8]) -> FwRead {
        let mut c = self.root;
        for d in FW_DIR {
            match self.sub(c, d) {
                Ok(Some((k, true))) => c = k,
                Ok(Some((_, false))) => return FwRead::Bad("a folder on the way to it is a file"),
                Ok(None) => return FwRead::Missing,
                Err(why) => return FwRead::Bad(why),
            }
        }
        let mut found = None;
        if let Err(why) = self.dir(c, &mut |e| {
            if e.is(name) {
                found = Some((e.clus, e.size, e.attr));
                return true;
            }
            false
        }) {
            return FwRead::Bad(why);
        }
        let Some((first, size, attr)) = found else { return FwRead::Missing };
        if attr & 0x10 != 0 {
            return FwRead::Bad("it is a folder, not a file");
        }
        if size as usize != want {
            return FwRead::WrongSize(size);
        }
        if want == 0 || want > out.len() {
            return FwRead::Bad("its size is outside what a firmware file may be");
        }
        let per = self.bps / 512;
        let cb = self.cluster_bytes() as usize;
        let need = want.div_ceil(cb);
        if need > FW_CHAIN {
            return FwRead::Bad("it uses more clusters than a firmware file may");
        }
        // SAFETY: shell context only (the `firmware` command and drivers' setup from it), one reader at a time
        // (fwdb::load holds its lock around this call).
        let chain = unsafe { &mut *core::ptr::addr_of_mut!(CHAIN) };
        let (mut fs, mut fbuf) = (u64::MAX, [0u8; 512]); // the FAT sector last read
        let (mut c, mut off) = (first, 0usize);
        for i in 0..need {
            if c < 2 || c as u64 > self.clusters + 1 {
                return FwRead::Bad("its cluster chain points outside the partition");
            }
            chain[i] = c;
            let at = self.data0 + (c as u64 - 2) * self.spc * per;
            for k in 0..self.spc * per {
                if off >= want {
                    break;
                }
                let mut s = [0u8; 512];
                if !self.dk.read(at + k, &mut s) {
                    return FwRead::Bad("a sector of it could not be read");
                }
                let n = (want - off).min(512);
                out[off..off + n].copy_from_slice(&s[..n]);
                off += n;
            }
            if i + 1 == need {
                break;
            }
            let fat_at = c as u64 * 4;
            let sec = self.fat0 + fat_at / 512;
            if sec != fs {
                if !self.dk.read(sec, &mut fbuf) {
                    return FwRead::Bad("its FAT could not be read");
                }
                fs = sec;
            }
            let n = u32le(&fbuf, (fat_at % 512) as usize) & 0x0FFF_FFFF;
            if n >= 0x0FFF_FFF8 {
                return FwRead::Bad("its cluster chain ends before the file's size");
            }
            c = n; // 0, 1, a bad-cluster mark or a number past the end is refused at the top of the loop
        }
        // a chain that comes back to a cluster it already used reads the same bytes twice; refused whatever the
        // hash would say
        let used = &mut chain[..need];
        used.sort_unstable();
        if used.windows(2).any(|w| w[0] == w[1]) {
            return FwRead::Bad("its cluster chain loops back on itself");
        }
        FwRead::Read
    }
}

/// The firmware file `name` (from fwdb.rs only), exactly `want` bytes, into `out[..want]`: from the USB stick
/// first, then every internal disk, in the first EFI system partition that has it. The disk it came from is
/// written into `from`. Shell context only.
pub(crate) fn read_file(name: &str, want: usize, out: &mut [u8], from: &mut [u8; 32]) -> FwRead {
    // the stick (the usual way AfriSovera is started today), then the internal disks; a FAT32 stick is not a VFS
    // stick, so it is reached by msc_attached (14 Nov 2026), not msc_present
    if crate::xhci::msc_attached() {
        let r = fw_on_stick(name, want, out);
        if !matches!(r, FwRead::Missing) {
            *from = [0; 32];
            from[..9].copy_from_slice(b"usb stick");
            return r;
        }
    }
    let (ds, nd) = gpt::disks();
    for &(dk, cap, bs) in &ds[..nd] {
        let r = fw_on_disk(dk, cap, bs, name, want, out);
        if !matches!(r, FwRead::Missing) {
            *from = dk.label();
            return r;
        }
    }
    FwRead::Missing
}

/// The file on one disk's EFI system partitions (Missing when none has it, or the disk has none we can read).
fn fw_on_disk<D: Sect>(dk: D, cap: u64, bs: u64, name: &str, want: usize, out: &mut [u8]) -> FwRead {
    let mut found = [(0u64, 0u64, 0u64, [0u8; 16]); 4];
    let Ok(n) = esps(dk, cap, bs, &mut found) else { return FwRead::Missing };
    let per = (bs / 512).max(1);
    for &(_, a, z, _) in &found[..n] {
        let Ok(fat) = Fat::open(dk, a * per, (z - a + 1) * per) else { continue };
        match fat.fw_file(name, want, out) {
            FwRead::Missing => continue,
            r => return r,
        }
    }
    FwRead::Missing
}

/// The file on the USB stick. A stick made the way make-pc-usb.sh says (Windows' Format, FAT32) has either no
/// partition table at all (the FAT32 starts at sector 0) or an MBR with one FAT32 partition; a GPT stick is
/// read like an internal disk. Each start and length is checked against the stick's size.
fn fw_on_stick(name: &str, want: usize, out: &mut [u8]) -> FwRead {
    let cap = crate::xhci::msc_capacity();
    if let Ok(fat) = Fat::open(Stick, 0, cap) {
        return fat.fw_file(name, want, out);
    }
    let mut s = [0u8; 512];
    if !Stick.read(0, &mut s) || s[510] != 0x55 || s[511] != 0xAA {
        return FwRead::Missing;
    }
    if (0..4).any(|i| s[446 + 16 * i + 4] == 0xEE) {
        return fw_on_disk(Stick, cap, crate::xhci::msc_block_size(), name, want, out);
    }
    let per = (crate::xhci::msc_block_size() / 512).max(1); // MBR numbers count the stick's own blocks
    for i in 0..4 {
        let e = &s[446 + 16 * i..446 + 16 * (i + 1)];
        let (a, n) = (u32le(e, 8) as u64 * per, u32le(e, 12) as u64 * per);
        // 0x0B and 0x0C: FAT32; 0xEF: an EFI system partition on an MBR disk
        if !matches!(e[4], 0x0B | 0x0C | 0xEF) || a == 0 || n == 0 || a + n > cap {
            continue;
        }
        let Ok(fat) = Fat::open(Stick, a, n) else { continue };
        match fat.fw_file(name, want, out) {
            FwRead::Missing => continue,
            r => return r,
        }
    }
    FwRead::Missing
}

// --- `report save`: the field report written to the USB stick AfriSovera started from (report home, 13 Nov 2026,
// ARCHITECT_REVIEWS "report home plan") ---
//
// Only on a USB mass-storage stick the firmware started us from (never an internal disk, even one we booted from),
// only as a NEW file \AFREPORT\<n>.TXT (8.3 names, n one more than the highest there, at most 64 files), at most
// the report's 96 KiB, and only when at least 1 MiB stays free. bootcopy's checks and order: the stick's FAT32
// closed cleanly with both FAT copies equal before anything is written; the file's clusters, then (the first time)
// \AFREPORT's own cluster with "." and ".." and the file's entry, each written before its FAT entries in both
// copies; then FSInfo; the entry that names the new file or folder last. Then the stick is flushed and the file
// read back and compared before it is called saved.

/// Most report files kept in \AFREPORT.
const SAVE_MAX: u32 = 64;
const SAVE_DIR: &[u8; 11] = b"AFREPORT   ";

/// (start, length) in 512-byte sectors of each FAT32 the stick may hold: the whole stick (no partition table), the
/// FAT32 partitions of an MBR, or the EFI system partitions of a GPT; how many.
fn stick_parts(out: &mut [(u64, u64); 4]) -> usize {
    let cap = crate::xhci::msc_capacity();
    if Fat::open(Stick, 0, cap).is_ok() {
        out[0] = (0, cap);
        return 1;
    }
    let mut s = [0u8; 512];
    if !Stick.read(0, &mut s) || s[510] != 0x55 || s[511] != 0xAA {
        return 0;
    }
    let bs = crate::xhci::msc_block_size();
    let per = (bs / 512).max(1);
    if (0..4).any(|i| s[446 + 16 * i + 4] == 0xEE) {
        let mut found = [(0u64, 0u64, 0u64, [0u8; 16]); 4];
        let Ok(n) = esps(Stick, cap, bs, &mut found) else { return 0 };
        for (i, &(_, a, z, _)) in found[..n].iter().enumerate() {
            out[i] = (a * per, (z - a + 1) * per);
        }
        return n;
    }
    let mut k = 0;
    for i in 0..4 {
        let e = &s[446 + 16 * i..446 + 16 * (i + 1)];
        let (a, n) = (u32le(e, 8) as u64 * per, u32le(e, 12) as u64 * per);
        if matches!(e[4], 0x0B | 0x0C | 0xEF) && a != 0 && n != 0 && a + n <= cap {
            out[k] = (a, n);
            k += 1;
        }
    }
    k
}

/// The FAT time and date of now (the firmware clock), or noon on the build's fixed date.
fn fat_now() -> (u16, u16) {
    match crate::uefi::get_time() {
        Ok(t) if (1980..2108).contains(&t.year) && (1..=12).contains(&t.month) && (1..=31).contains(&t.day) => (
            (t.hour as u16) << 11 | (t.minute as u16) << 5 | (t.second as u16 / 2),
            (t.year - 1980) << 9 | (t.month as u16) << 5 | t.day as u16,
        ),
        _ => (12 << 11, ((2026 - 1980) << 9) | (11 << 5) | 13),
    }
}

/// A directory entry as gpt::dirent makes it, stamped with `when` (time, date).
fn dirent_at(d: &mut [u8], name: &[u8; 11], attr: u8, clus: u32, size: u32, when: (u16, u16)) {
    gpt::dirent(d, name, attr, clus, size);
    for (o, v) in [(14, when.0), (16, when.1), (18, when.1), (22, when.0), (24, when.1)] {
        d[o..o + 2].copy_from_slice(&v.to_le_bytes());
    }
}

/// `<n>.<ext>` as an 8.3 name; None past 8 digits.
fn txt_name(n: u32, ext: &[u8; 3]) -> Option<[u8; 11]> {
    let mut d = [0u8; 10];
    let mut k = 0;
    let mut v = n;
    loop {
        d[k] = b'0' + (v % 10) as u8;
        k += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if k > 8 {
        return None;
    }
    let mut s = [b' '; 11];
    for i in 0..k {
        s[i] = d[k - 1 - i];
    }
    s[8..].copy_from_slice(ext);
    Some(s)
}

/// The number of an 8.3 name `<digits>.TXT` or `<digits>.BIN` (one numbering for both: a report and a saved table
/// set never share a number).
fn txt_number(short: &[u8; 11]) -> Option<u32> {
    if &short[8..] != b"TXT" && &short[8..] != b"BIN" {
        return None;
    }
    let digits = short[..8].iter().take_while(|&&c| c != b' ').count();
    if digits == 0 || short[digits..8].iter().any(|&c| c != b' ') {
        return None;
    }
    let mut v = 0u32;
    for &c in &short[..digits] {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    Some(v)
}

impl<D: Sect> Fat<D> {
    /// The file at `path` (backslashes, from the root) starts with the first `cmp` bytes of `want` and is exactly
    /// `want.len()` bytes long: the boot file AfriSovera was started from is on this file system.
    fn holds(&self, path: &str, want: &[u8], cmp: usize) -> Result<bool, &'static str> {
        let mut c = self.root;
        let parts = path.split('\\').filter(|p| !p.is_empty());
        let n = parts.clone().count();
        if n == 0 {
            return Ok(false);
        }
        for (i, name) in parts.enumerate() {
            let mut got = None;
            self.dir(c, &mut |e| {
                if e.is(name) {
                    got = Some((e.clus, e.attr, e.size));
                    return true;
                }
                false
            })?;
            let Some((clus, attr, size)) = got else { return Ok(false) };
            if i + 1 < n {
                if attr & 0x10 == 0 {
                    return Ok(false);
                }
                c = clus;
                continue;
            }
            if attr & 0x10 != 0 || size as usize != want.len() {
                return Ok(false);
            }
            // the first `cmp` bytes, sector by sector along the chain
            let (mut cl, mut off) = (clus, 0usize);
            let mut s = [0u8; 512];
            let mut cache = (u64::MAX, [0u8; 512]);
            let cmp = cmp.min(want.len());
            while off < cmp {
                if cl < 2 || cl as u64 > self.clusters + 1 {
                    return Ok(false);
                }
                for q in 0..self.c512() {
                    if off >= cmp {
                        break;
                    }
                    if !self.dk.read(self.at(cl) + q, &mut s) {
                        return Err("could not read the boot file");
                    }
                    let to = (off + 512).min(cmp);
                    if s[..to - off] != want[off..to] {
                        return Ok(false);
                    }
                    off = to;
                }
                match self.next_c(cl, &mut cache) {
                    Some(nx) if nx >= 0x0FFF_FFF8 => break,
                    Some(nx) if nx >= 2 => cl = nx,
                    _ => return Ok(false),
                }
            }
            return Ok(off >= cmp);
        }
        Ok(false)
    }

    /// The file `name` in the folder at `dirc` read back and compared with `want`: Ok when its size, first
    /// cluster and every byte are as written.
    fn reads_back(&self, dirc: u32, name: &[u8; 11], first: u32, want: &[u8]) -> Result<(), &'static str> {
        let mut got = None;
        self.dir(dirc, &mut |e| {
            if &e.short == name && e.attr & 0x10 == 0 {
                got = Some((e.clus, e.size));
                return true;
            }
            false
        })?;
        let Some((c0, size)) = got else { return Err("the file's entry is not there") };
        if c0 != first || size as usize != want.len() {
            return Err("the file's entry does not read back as written");
        }
        let (mut c, mut off) = (c0, 0usize);
        let mut s = [0u8; 512];
        let mut cache = (u64::MAX, [0u8; 512]);
        loop {
            if c < 2 || c as u64 > self.clusters + 1 {
                return Err("the file's cluster chain is damaged");
            }
            for q in 0..self.c512() {
                if off >= want.len() {
                    break;
                }
                if !self.dk.read(self.at(c) + q, &mut s) {
                    return Err("could not read the file back");
                }
                let to = (off + 512).min(want.len());
                if s[..to - off] != want[off..to] {
                    return Err("the file does not read back the same");
                }
                off = to;
            }
            let nx = self.next_c(c, &mut cache).ok_or("could not read the FAT")?;
            if off >= want.len() {
                return if nx >= 0x0FFF_FFF8 { Ok(()) } else { Err("the file's chain is too long") };
            }
            if nx >= 0x0FFF_FFF8 || nx == 0 {
                return Err("the file's chain is too short");
            }
            c = nx;
        }
    }
}

/// `report save`: `body` written to the USB stick AfriSovera started from as \AFREPORT\<n>.TXT. One line per
/// outcome; true when it was saved and read back.
pub fn report_save(body: &[u8]) -> bool {
    stick_save("report save", body, b"TXT")
}

/// `acpisave` (AML rung C2, ARCHITECT_REVIEWS 14 Nov): the firmware's DSDT, SSDTs and FACP, put together by
/// hwscan::acpi_save, written the same way as \AFREPORT\<n>.BIN.
pub fn acpi_save(body: &[u8]) -> bool {
    stick_save("acpisave", body, b"BIN")
}

/// The one door (ARCHITECT_REVIEWS 14 Nov): `body` as a new file \AFREPORT\<n>.<ext> on the USB stick AfriSovera
/// started from, after the boot-medium and boot-file checks; `who` starts every line.
fn stick_save(who: &'static str, body: &[u8], ext: &[u8; 3]) -> bool {
    use crate::uefi::Medium;
    match crate::uefi::boot_medium() {
        Medium::Usb => {}
        Medium::Other(k) => {
            crate::kprintln!("{}: AfriSovera started from {} (not a USB stick), and {} writes only to the USB stick it started from; nothing written.{}", who, k, who, if who == "report save" { " Use  report <address>  instead." } else { "" });
            return false;
        }
        Medium::Unknown => {
            crate::kprintln!("{}: the firmware did not say what AfriSovera started from, so nothing is written (it writes only to the USB stick it started from)", who);
            return false;
        }
    }
    if !crate::xhci::msc_attached() {
        crate::kprintln!("{}: AfriSovera started from a USB stick, but the stick is not reachable now (was it pulled out?); nothing written", who);
        return false;
    }
    let Some(own) = crate::uefi::own_image() else {
        crate::kprintln!("{}: the boot file was not kept at power-on, so the stick it came from cannot be recognised; nothing written", who);
        return false;
    };
    // the stick the driver holds must be the one we started from: it holds our boot file, byte for byte at the start
    let mut parts = [(0u64, 0u64); 4];
    let np = stick_parts(&mut parts);
    let mut fat = None;
    for &(a, n) in &parts[..np] {
        let Ok(f) = Fat::open(Stick, a, n) else { continue };
        if let Ok(true) = f.holds(crate::uefi::own_path(), own, 64 * 1024) {
            fat = Some(f);
            break;
        }
    }
    let Some(seen) = fat else {
        crate::kprintln!("{}: the USB stick plugged in is not the one AfriSovera started from (its FAT32 does not hold this boot file); nothing written", who);
        return false;
    };
    // the one door: the only place the stick is opened for writing, after both checks above
    let Ok(fat) = Fat::open(BootStick(()), seen.part, seen.len()) else {
        crate::kprintln!("{}: the stick's FAT32 could not be opened again; nothing written", who);
        return false;
    };
    match fat.free() {
        Err(why) => {
            crate::kprintln!("{}: the stick's file system {}; nothing written", who, why);
            return false;
        }
        Ok((_, false, _)) => {
            crate::kprintln!("{}: the stick's two FAT copies differ (let Windows check it); nothing written", who);
            return false;
        }
        Ok((_, _, false)) => {
            crate::kprintln!("{}: the stick was not closed cleanly (pulled out without Safely Remove, or Windows found errors). Put it in Windows, let it check the stick, eject it safely, then try again; nothing written", who);
            return false;
        }
        Ok(_) => {}
    }
    let (free0, _, _) = fat.free().unwrap_or((0, false, false));
    let cb = fat.cluster_bytes();
    // \AFREPORT: there or not, the reports in it, a free slot
    let mut dir: Option<u32> = None;
    let mut not_dir = false;
    let r = fat.dir(fat.root, &mut |e| {
        if &e.short == SAVE_DIR {
            if e.attr & 0x10 != 0 {
                dir = Some(e.clus);
            } else {
                not_dir = true;
            }
            return true;
        }
        false
    });
    if let Err(why) = r {
        crate::kprintln!("{}: the stick's top folder: {}; nothing written", who, why);
        return false;
    }
    if not_dir {
        crate::kprintln!("{}: the stick has a FILE named AFREPORT where the reports' folder goes; nothing written", who);
        return false;
    }
    let (mut count, mut high) = (0u32, 0u32);
    let mut slot = None;
    let mut ended = false;
    let home = dir.unwrap_or(fat.root);
    if let Some(dc) = dir {
        let r = fat.dir(dc, &mut |e| {
            if let Some(n) = txt_number(&e.short) {
                if e.attr & 0x18 == 0 {
                    count += 1;
                    high = high.max(n);
                }
            }
            false
        });
        if let Err(why) = r {
            crate::kprintln!("{}: \\AFREPORT: {}; nothing written", who, why);
            return false;
        }
        if count >= SAVE_MAX {
            crate::kprintln!("{}: \\AFREPORT already holds {} reports. Move them to the main PC (report-from-stick), delete them from the stick, then try again; nothing written", who, count);
            return false;
        }
    }
    let r = fat.slots(home, &mut |n, o, e| {
        if e[0] == 0 {
            ended = true;
        }
        if ended || e[0] == 0xE5 {
            slot = Some((n, o));
            return true;
        }
        false
    });
    if let Err(why) = r {
        crate::kprintln!("{}: {}; nothing written", who, why);
        return false;
    }
    let Some(slot) = slot else {
        crate::kprintln!("{}: the stick's {} folder is full (it would have to grow); nothing written", who, if dir.is_some() { "\\AFREPORT" } else { "top" });
        return false;
    };
    let n = high + 1;
    let Some(name) = txt_name(n, ext) else {
        crate::kprintln!("{}: the next report number is too large for an 8.3 name; nothing written", who);
        return false;
    };
    let nfile = (body.len() as u64).div_ceil(cb).max(1);
    let used = nfile + dir.is_none() as u64;
    let keep = (1024 * 1024u64).div_ceil(cb);
    if free0 < used + keep {
        crate::kprintln!("{}: the stick has {} KiB free; the report needs {} KiB and 1 MiB must stay free; nothing written", who, kib(free0 * cb), kib(used * cb));
        return false;
    }
    let when = fat_now();
    // 1: the file's clusters (data, then both FAT copies)
    let fc = match fat.take(nfile, &mut &body[..]) {
        Ok(c) => c,
        Err(why) => {
            crate::kprintln!("{}: {}. No folder entry was written, so the stick shows no new file; Windows' check frees any clusters already taken", who, why);
            return false;
        }
    };
    let mut fe = [0u8; 32];
    dirent_at(&mut fe, &name, 0x20, fc, body.len() as u32, when);
    let ok = if dir.is_some() {
        // 2: FSInfo; 3: the file's entry in \AFREPORT, last
        fat.fsinfo(-(nfile as i64)) && fat.put_slot(slot.0, slot.1, &fe)
    } else {
        // 2: \AFREPORT's own cluster: take() writes it zeroed, then its FAT entry; then its first sector gets
        // "." "..", and the file's entry; 3: FSInfo; 4: the top folder's entry naming \AFREPORT, last
        let mut d = [0u8; 96];
        match fat.take(1, &mut &[][..]) {
            Err(why) => {
                crate::kprintln!("{}: {}. No folder entry was written, so the stick shows no new file; Windows' check frees any clusters already taken", who, why);
                return false;
            }
            Ok(dc) => {
                dirent_at(&mut d[0..32], b".          ", 0x10, dc, 0, when);
                dirent_at(&mut d[32..64], b"..         ", 0x10, 0, 0, when);
                d[64..96].copy_from_slice(&fe);
                let mut s = [0u8; 512];
                s[..96].copy_from_slice(&d);
                let mut de = [0u8; 32];
                dirent_at(&mut de, SAVE_DIR, 0x10, dc, 0, when);
                dir = Some(dc);
                // the cluster was zeroed by take; its first sector now holds the three entries
                fat.dk.write(fat.at(dc), &s) && fat.fsinfo(-(used as i64)) && fat.put_slot(slot.0, slot.1, &de)
            }
        }
    };
    let ok = ok && fat.dk.settle();
    if !ok {
        crate::kprintln!("{}: a write FAILED part way. The entry naming the report was {}; Windows' check of the stick puts it right", who, if dir.is_some() { "perhaps not written" } else { "not written" });
        return false;
    }
    // read it all back from the stick (a fresh look at the file system)
    let back = Fat::open(Stick, fat.part, fat.len()).and_then(|f| {
        let dc = dir.ok_or("the folder is not known")?;
        f.reads_back(dc, &name, fc, body)?;
        let (free, same, clean) = f.free()?;
        if !same || !clean || free + used != free0 {
            return Err("the FAT does not read back as written");
        }
        Ok(free)
    });
    let mut shown = [0u8; 12];
    let k = name[..8].iter().take_while(|&&c| c != b' ').count();
    shown[..k].copy_from_slice(&name[..k]);
    shown[k] = b'.';
    shown[k + 1..k + 4].copy_from_slice(ext);
    let shown = core::str::from_utf8(&shown[..k + 4]).unwrap_or("?");
    match back {
        Ok(free) => {
            crate::kprintln!("{}: SAVED \\AFREPORT\\{} ({} bytes), read back the same; {} KiB free on the stick", who, shown, body.len(), kib(free * cb));
            true
        }
        Err(why) => {
            crate::kprintln!("{}: written as \\AFREPORT\\{}, but reading it back FAILED: {}. Do not rely on it; try again", who, shown, why);
            false
        }
    }
}
