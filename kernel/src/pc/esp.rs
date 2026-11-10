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
//! nothing already in the partition is moved or changed -- and `bootcopy remove` takes it out
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

/// A FAT32 file system as its boot sector describes it. Sector numbers are 512-byte sectors
/// of the disk.
struct Fat {
    dk: Disk,
    part: u64,     // the partition's first 512-byte sector
    bps: u64,      // bytes per FAT sector
    spc: u64,      // FAT sectors per cluster
    fat0: u64,     // the first FAT's first 512-byte sector
    fat512: u64,   // one FAT's length in 512-byte sectors
    nfats: u64,
    data0: u64,    // cluster 2's first 512-byte sector
    clusters: u64, // data clusters (numbered 2 ..= clusters + 1)
    root: u32,
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

impl Fat {
    /// The boot sector of the partition at `part` (512-byte sectors), `len` sectors long:
    /// a FAT32 we can trust, or why not.
    fn open(dk: Disk, part: u64, len: u64) -> Result<Fat, &'static str> {
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
        })
    }

    fn cluster_bytes(&self) -> u64 {
        self.bps * self.spc
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
fn esps(dk: Disk, cap512: u64, bs: u64, out: &mut [(u64, u64, u64, [u8; 16]); 4]) -> Result<usize, &'static str> {
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
        return Err("the GPT's primary header is damaged -- a damaged table is never changed");
    };
    let alt = u64le(&h, 32);
    if alt >= blocks || gpt::header(&mut rd, alt, per).is_none() {
        return Err("the GPT's backup header is missing or damaged -- a damaged table is never changed");
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
        return Err("the GPT's entries are damaged (CRC) -- a damaged table is never changed");
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
        None => say!(on, "bootplan: AfriSovera's boot file was not kept at power-on -- the EFI partitions are shown, no plan is made"),
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
                    say!(on, "  FAT32, but {} -- not changed", why);
                    continue;
                }
            };
            let cb = fat.cluster_bytes();
            say!(on, "  FAT32, {}-byte clusters, {} KiB free", cb, kib(free * cb));
            // what is in \EFI
            let efi = match fat.sub(fat.root, "EFI") {
                Ok(v) => v,
                Err(why) => {
                    say!(on, "  {} -- not changed", why);
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
                        say!(on, "  {} -- not changed", why);
                        continue;
                    }
                    let what = core::str::from_utf8(&l[..ln]).unwrap_or("?");
                    say!(on, "  \\EFI holds: {}", if ln == 0 { "nothing" } else { what });
                }
                Some((_, false)) => {
                    say!(on, "  \\EFI is a file, not a folder -- not changed");
                    continue;
                }
                None => say!(on, "  no \\EFI folder yet"),
            }
            if !clean {
                say!(on, "  plan: none -- the file system was not closed cleanly (Windows Fast Startup or a crash): not changed until Windows has checked it");
                continue;
            }
            if !same {
                say!(on, "  plan: none -- its two FAT copies differ: not changed until Windows has checked it");
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
                    Ok(b) => say!(on, "  \\EFI\\afrisovera is already there ({} KiB) -- nothing to copy", kib(b)),
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
                    "  plan: none -- the boot file needs {} KiB here, only {} KiB free",
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
    match crate::bootent::numbers() {
        None => crate::kprintln!("  boot entry: the firmware's boot entries were not read at power-on -- none can be planned"),
        Some((_, Some(n), _)) => {
            let nm = crate::bootent::name(n);
            crate::kprintln!("  boot entry: AfriSovera already has one ({}) -- nothing to add", core::str::from_utf8(&nm).unwrap_or("?"));
        }
        Some((free, None, first)) => match (chosen, free) {
            (None, _) if already => crate::kprintln!("  boot entry: AfriSovera's file is there but no entry points at it yet -- adding one will ask for the owner's word"),
            (None, _) => crate::kprintln!("  boot entry: none planned -- no EFI system partition with room"),
            (Some(_), None) => crate::kprintln!("  boot entry: none planned -- no free Boot number below Boot0100"),
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
                    Some((_, d, dn)) => crate::kprintln!(
                        "  the PC keeps starting \"{}\" first; AfriSovera is picked from the firmware's boot menu",
                        core::str::from_utf8(&d[..dn]).unwrap_or("?")
                    ),
                    None => crate::kprintln!("  the PC keeps its first choice; AfriSovera is picked from the firmware's boot menu"),
                }
            }
        },
    }
    crate::kprintln!("bootplan: a plan only -- nothing was written; adding it will ask for the owner's word, with the way back");
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

/// Fat::take's run of cluster sectors (shell context only).
static mut BIG: [u8; crate::gpt::RUN * 512] = [0; crate::gpt::RUN * 512];

impl Fat {
    /// 512-byte sectors per cluster.
    fn c512(&self) -> u64 {
        self.spc * self.bps / 512
    }

    /// The first 512-byte sector of cluster `c`.
    fn at(&self, c: u32) -> u64 {
        self.data0 + (c as u64 - 2) * self.c512()
    }

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

    /// Takes `need` free clusters, lowest first, as one chain; `data` (zero filled after its
    /// end) is written into each cluster BEFORE its FAT entry changes. The chain's first cluster.
    fn take(&self, need: u64, data: &[u8]) -> Result<u32, &'static str> {
        let last = self.clusters + 1;
        let cb = self.cluster_bytes() as usize;
        let (mut first, mut prev, mut got) = (0u32, 0u32, 0u64);
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
                // the cluster in runs of up to RUN sectors (one command and one flush each on an IDE-mode disk)
                let mut q = 0u64;
                while q < self.c512() {
                    let k = (self.c512() - q).min(crate::gpt::RUN as u64) as usize;
                    // SAFETY: shell context only; nothing else uses BIG while a copy runs.
                    let run = unsafe { &mut (&mut *core::ptr::addr_of_mut!(BIG))[..k * 512] };
                    run.fill(0);
                    let from = got as usize * cb + q as usize * 512;
                    if from < data.len() {
                        let to = (from + k * 512).min(data.len());
                        run[..to - from].copy_from_slice(&data[from..to]);
                    }
                    if !self.dk.write_run(base + q, run) {
                        return Err("a write FAILED");
                    }
                    q += k as u64;
                }
                a[o..o + 4].copy_from_slice(&((v & 0xF000_0000) | EOC).to_le_bytes());
                changed = true;
                if prev != 0 {
                    if prev as u64 / 128 == k {
                        let po = (prev as usize % 128) * 4;
                        let pv = u32le(&a, po);
                        a[po..po + 4].copy_from_slice(&((pv & 0xF000_0000) | c as u32).to_le_bytes());
                    } else if !self.link(prev, c as u32) {
                        return Err("a write FAILED");
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
            crate::kprintln!("{}: that code does not fit this EFI partition as it is now -- nothing written", cmd);
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
        crate::kprintln!("bootcopy: AfriSovera's boot file was not kept at power-on -- nothing to copy; nothing written");
        return;
    };
    let (chosen, there) = scan(false);
    if let Some(t) = there {
        crate::kprintln!(
            "bootcopy: {} partition {} already holds \\EFI\\afrisovera -- nothing to copy (bootcopy remove takes it out)",
            gpt::label_str(&t.label),
            t.ent
        );
        return;
    }
    let Some(s) = chosen else {
        crate::kprintln!("bootcopy: no EFI system partition can take the boot file (bootplan says why) -- nothing written");
        return;
    };
    let lname = gpt::label_str(&s.label);
    if s.efi == 0 {
        crate::kprintln!("bootcopy: {} partition {} has no \\EFI folder -- not changed (an EFI partition Windows uses always has one); nothing written", lname, s.ent);
        return;
    }
    let fat = match Fat::open(s.dk, s.part, s.len) {
        Ok(f) => f,
        Err(why) => {
            crate::kprintln!("bootcopy: the EFI partition {} -- nothing written", why);
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
        crate::kprintln!("bootcopy: {} -- nothing written", why);
        return;
    }
    if rn < 2 {
        crate::kprintln!("bootcopy: \\EFI has no two free entries side by side (it would have to grow) -- not changed; nothing written");
        return;
    }
    let Some(tilde) = (1..=9u8).find(|&i| !used[i as usize]) else {
        crate::kprintln!("bootcopy: \\EFI already has AFRISO~1 to AFRISO~9 -- not changed; nothing written");
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
        crate::kprintln!("bootcopy: a read FAILED -- nothing written");
        return;
    };
    if !agreed("bootcopy", given, want) {
        return;
    }
    // 1: the file's clusters, 2: the folder's, both written before their FAT entries change;
    // 3: the folder's entries; 4: \EFI's entry last, so a cut-off copy leaves only clusters
    // no folder points at (Windows' chkdsk frees them), never an entry pointing at nothing
    let fc = match fat.take(nfile, file) {
        Ok(c) => c,
        Err(why) => {
            crate::kprintln!("bootcopy: {} -- \\EFI was not changed; chkdsk in Windows frees any clusters already taken", why);
            return;
        }
    };
    let dc = match fat.take(1, &[]) {
        Ok(c) => c,
        Err(why) => {
            crate::kprintln!("bootcopy: {} -- \\EFI was not changed; chkdsk in Windows frees any clusters already taken", why);
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
        && fat.fsinfo(-((nfile + 1) as i64));
    if !ok {
        crate::kprintln!("bootcopy: a write FAILED part way -- bootcopy remove (or chkdsk in Windows) puts it right");
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
            crate::kprintln!("bootcopy: no boot entry was added -- the PC starts exactly as before; bootcopy remove takes the file out again");
        }
        Err(why) => crate::kprintln!("bootcopy: reading back FAILED: {} -- bootcopy remove takes it out again", why),
    }
}

/// `bootcopy remove [yes <code>]`: \EFI\afrisovera taken out again -- only when it holds
/// afrisovera.efi and nothing else (what bootcopy put there).
pub fn remove(given: Option<&str>) {
    let (_, there) = scan(false);
    let Some(s) = there else {
        crate::kprintln!("bootcopy remove: no EFI system partition holds \\EFI\\afrisovera that may be changed (bootplan says why) -- nothing written");
        return;
    };
    let lname = gpt::label_str(&s.label);
    let fat = match Fat::open(s.dk, s.part, s.len) {
        Ok(f) => f,
        Err(why) => {
            crate::kprintln!("bootcopy remove: the EFI partition {} -- nothing written", why);
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
        crate::kprintln!("bootcopy remove: {} -- nothing written", why);
        return;
    }
    if others > 0 {
        crate::kprintln!("bootcopy remove: \\EFI\\afrisovera holds other files too -- not removed (only what bootcopy put there is taken out); nothing written");
        return;
    }
    let cb = fat.cluster_bytes();
    let (fc, size) = file.unwrap_or((0, 0));
    let flen = if fc == 0 { Some(0) } else { fat.chain_len(fc) };
    let dlen = fat.chain_len(s.ours);
    let (Some(flen), Some(dlen)) = (flen, dlen) else {
        crate::kprintln!("bootcopy remove: a cluster chain is damaged -- not changed until Windows has checked it; nothing written");
        return;
    };
    if flen != (size as u64).div_ceil(cb) {
        crate::kprintln!("bootcopy remove: afrisovera.efi's chain does not fit its size -- not changed until Windows has checked it; nothing written");
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
        crate::kprintln!("bootcopy remove: \\EFI\\afrisovera's entry could not be found again -- nothing written");
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
        crate::kprintln!("bootcopy remove: a read FAILED -- nothing written");
        return;
    };
    if !agreed("bootcopy remove", given, want) {
        return;
    }
    // the entry first (marked deleted, its long-name pieces too), then its clusters freed
    let mut ok = (0..found).rev().all(|i| fat.put_slot(at[i].0, at[i].1, &[0xE5]));
    ok = ok && (fc == 0 || fat.release(fc)) && fat.release(s.ours) && fat.fsinfo((flen + dlen) as i64);
    if !ok {
        crate::kprintln!("bootcopy remove: a write FAILED part way -- chkdsk in Windows puts the partition right");
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
