//! Safe updates U3 (AFR-0010 section 8, architect 14 Nov 2026): the spare boot file in `\EFI\afrisovera`.
//!
//! Two boot files live side by side: `afrisovera.efi` and `afrisovera-b.efi`. The main boot entry names one; the
//! other is the spare, and only the spare is ever written here. Nothing outside `\EFI\afrisovera` is touched.
//!
//! Write order for a new spare (the architect's Q2 (b)):
//! 1. its directory entries are written first, MARKED DELETED (0xE5), naming our short name, the first cluster and
//!    the size, so a power cut leaves a chain a later `updatestage` can recognise as its own;
//! 2. the file's clusters (data, then both FAT copies; `Fat::take`);
//! 3. FSInfo;
//! 4. the entries turned live, in one sector write: the last write.
//!
//! An orphaned chain is freed only when ALL hold: it is named by such a deleted entry in `\EFI\afrisovera` with one
//! of our short names; every cluster in it is allocated and the chain ends (at an end mark, or at a free cluster when
//! the cut came between two FAT sectors of `take`) within the recorded size; and no live
//! entry anywhere in the partition (not only that folder) references any of its clusters. The last rule is
//! stricter than the folder alone, so clusters Windows took over after a chkdsk are never freed.

use super::{code, lfn, scan, short_sum, Fat, Source, Spot};
use crate::gpt;

/// The two boot files' long names, in `\EFI\afrisovera`.
pub(crate) const SLOT_A: &str = "afrisovera.efi";
pub(crate) const SLOT_B: &str = "afrisovera-b.efi";
/// The architect's stage cap (Q4): the format allows 64 MiB, staging takes at most 32 MiB.
pub(crate) const STAGE_MAX: u64 = 32 << 20;
/// Room that must stay free in the EFI partition after staging (shared with Windows).
pub(crate) const KEEP_FREE: u64 = 2 << 20;

/// The EFI partition holding `\EFI\afrisovera`, opened for staging.
pub(crate) struct Esp {
    s: Spot,
    fat: Fat,
    dir: u32,
}

/// A live file in `\EFI\afrisovera`: its 8.3 name, first cluster and size.
#[derive(Clone, Copy)]
pub(crate) struct Live {
    pub short: [u8; 11],
    pub clus: u32,
    pub size: u32,
}

/// The EFI partition the main entry names (`guid`), with `\EFI\afrisovera` in it, closed cleanly and its two FAT
/// copies equal (scan keeps only such partitions).
pub(crate) fn open(guid: &[u8; 16]) -> Result<Esp, &'static str> {
    let (_, there) = scan(false);
    let Some(s) = there else {
        return Err("no EFI partition holding \\EFI\\afrisovera may be changed now (it was not closed cleanly, or its two FAT copies differ: let Windows check this disk; bootplan says why)");
    };
    if s.guid != *guid {
        return Err("the EFI partition holding \\EFI\\afrisovera is not the one AfriSovera's boot entry names");
    }
    let fat = Fat::open(s.dk, s.part, s.len).map_err(|_| "the EFI partition could not be opened again")?;
    Ok(Esp { s, fat, dir: s.ours })
}

/// The 8.3 name AFRISO~N.EFI.
fn short_of(n: u8) -> [u8; 11] {
    let mut s = *b"AFRISO~0EFI";
    s[7] = b'0' + n;
    s
}

/// Whether a slot is one of OUR file entries marked deleted: "?FRISO~N" + "EFI", a file.
fn ours_deleted(e: &[u8]) -> bool {
    e[0] == 0xE5 && &e[1..7] == b"FRISO~" && (b'1'..=b'9').contains(&e[7]) && &e[8..11] == b"EFI" && e[11] == 0x20
}

fn clus_of(e: &[u8]) -> u32 {
    (u16::from_le_bytes([e[20], e[21]]) as u32) << 16 | u16::from_le_bytes([e[26], e[27]]) as u32
}

/// Clusters marked by the orphan check (one bit each); a partition with more clusters is not reclaimed.
static mut MARK: [u8; 65536] = [0; 65536];

impl Esp {
    pub fn cluster_bytes(&self) -> u64 {
        self.fat.cluster_bytes()
    }

    /// Free clusters now; Err when the FAT copies differ or it is not clean any more.
    pub fn free(&self) -> Result<u64, &'static str> {
        match self.fat.free()? {
            (f, true, true) => Ok(f),
            (_, false, _) => Err("its two FAT copies differ (let Windows check this disk, or run bootplan to see it)"),
            _ => Err("it was not closed cleanly (let Windows check this disk)"),
        }
    }

    /// The owner's code for `tag`: tied, as bootcopy's, to the partition, its free room and `\EFI` as it is now,
    /// and to the partition's GUID and the update's version and SHA-256, so a code typed for one update never
    /// stages another (architect's Q3).
    pub fn code(&self, tag: &[u8], version: u32, sha: &[u8; 32]) -> Option<u32> {
        let mut c = gpt::crc_feed(0xFFFF_FFFF, &self.s.guid);
        c = gpt::crc_feed(c, &version.to_le_bytes());
        c = gpt::crc_feed(c, sha);
        code(tag, &self.s, &self.fat, c as u64)
    }

    /// The live file `long` in `\EFI\afrisovera`, if it is there.
    pub fn live(&self, long: &str) -> Result<Option<Live>, &'static str> {
        let mut got = None;
        self.fat.dir(self.dir, &mut |e| {
            if e.is(long) && e.attr & 0x18 == 0 {
                got = Some(Live { short: e.short, clus: e.clus, size: e.size });
                return true;
            }
            false
        })?;
        Ok(got)
    }

    /// Takes `long` out of `\EFI\afrisovera`: its 8.3 entry marked deleted FIRST (the file is gone from then on),
    /// its long-name pieces, then its clusters freed in both FAT copies, FSInfo, and the deleted entry's cluster
    /// and size cleared last. Ok(clusters freed; 0 when it was not there).
    pub fn free_file(&self, long: &str) -> Result<u64, &'static str> {
        let Some(f) = self.live(long)? else { return Ok(0) };
        let n = if f.clus == 0 { 0 } else { self.fat.chain_len(f.clus).ok_or("the old spare file's cluster chain is damaged; not changed")? };
        let sum = short_sum(&f.short);
        let (mut pieces, mut np, mut entry) = ([(0u64, 0usize); 4], 0usize, None);
        self.fat.slots(self.dir, &mut |sn, o, e| {
            if e[0] == 0 {
                return true;
            }
            if e[0] != 0xE5 && e[11] == 0x0F && e[13] == sum {
                if np < pieces.len() {
                    pieces[np] = (sn, o);
                    np += 1;
                }
                return false;
            }
            if e[0] != 0xE5 && e[11] != 0x0F && e[..11] == f.short {
                entry = Some((sn, o));
                return true;
            }
            np = 0;
            false
        })?;
        let Some((sn, o)) = entry else { return Err("the old spare file's entry could not be found again; not changed") };
        let mut ok = self.fat.put_slot(sn, o, &[0xE5]);
        for &(pn, po) in &pieces[..np] {
            ok = ok && self.fat.put_slot(pn, po, &[0xE5]);
        }
        if !ok {
            return Err("a write FAILED while marking the old spare file deleted");
        }
        if n > 0 && !(self.fat.release(f.clus) && self.fat.fsinfo(n as i64)) {
            return Err("a write FAILED while freeing the old spare file's clusters");
        }
        // the deleted entry no longer names any cluster, so it can never be taken for an orphan
        let mut s = [0u8; 512];
        if !self.fat.dk.read(sn, &mut s) {
            return Err("could not read the folder again");
        }
        s[o + 20..o + 22].fill(0);
        s[o + 26..o + 32].fill(0);
        if !(self.fat.dk.write(sn, &s) && self.fat.dk.settle()) {
            return Err("a write FAILED while clearing the old entry");
        }
        Ok(n)
    }

    /// Frees chains a power cut left behind (see the module comment for the three rules). Ok((clusters freed,
    /// chains freed, chains left alone)).
    pub fn reclaim(&self) -> Result<(u64, u32, u32), &'static str> {
        let cb = self.fat.cluster_bytes();
        let mut cands = [(0u64, 0usize, 0u32, 0u32); 8];
        let mut nc = 0usize;
        self.fat.slots(self.dir, &mut |sn, o, e| {
            if e[0] == 0 {
                return true;
            }
            if ours_deleted(e) && clus_of(e) >= 2 && nc < cands.len() {
                cands[nc] = (sn, o, clus_of(e), u32::from_le_bytes([e[28], e[29], e[30], e[31]]));
                nc += 1;
            }
            false
        })?;
        let (mut freed, mut chains, mut left) = (0u64, 0u32, 0u32);
        for &(sn, o, first, size) in &cands[..nc] {
            let most = (size as u64).div_ceil(cb).max(1);
            match self.orphan(first, most)? {
                Some(n) => {
                    if !self.release_orphan(first) {
                        return Err("a write FAILED while freeing a chain a power cut left");
                    }
                    let mut s = [0u8; 512];
                    if !self.fat.dk.read(sn, &mut s) {
                        return Err("could not read the folder again");
                    }
                    s[o + 20..o + 22].fill(0);
                    s[o + 26..o + 32].fill(0);
                    if !self.fat.dk.write(sn, &s) {
                        return Err("a write FAILED while clearing an old entry");
                    }
                    freed += n;
                    chains += 1;
                }
                None => left += 1,
            }
        }
        if chains > 0 && !(self.fsinfo_exact() && self.fat.dk.settle()) {
            return Err("a write FAILED");
        }
        Ok((freed, chains, left))
    }

    /// Some(its length) when the chain from `first` is a true orphan of ours: every cluster allocated, it ends within
    /// `most` clusters, and no live entry anywhere in the partition references any of its clusters.
    fn orphan(&self, first: u32, most: u64) -> Result<Option<u64>, &'static str> {
        let last = self.fat.clusters + 1;
        if last >= 65536 * 8 {
            return Ok(None); // too many clusters to mark: left alone
        }
        // SAFETY: shell context only; nothing else uses MARK.
        let mark = unsafe { &mut *core::ptr::addr_of_mut!(MARK) };
        mark.fill(0);
        let mut cache = (u64::MAX, [0u8; 512]);
        let (mut c, mut n) = (first, 0u64);
        loop {
            if c < 2 || c as u64 > last || n >= most {
                return Ok(None);
            }
            mark[c as usize / 8] |= 1 << (c % 8);
            n += 1;
            match self.fat.next_c(c, &mut cache).ok_or("could not read the FAT")? {
                // `c` itself is free: the chain ran into it (a cut between `take` linking the previous FAT sector's
                // last cluster and writing the next FAT sector). The allocated clusters before it are the orphan.
                0 => {
                    mark[c as usize / 8] &= !(1 << (c % 8));
                    n -= 1;
                    if n == 0 {
                        return Ok(None); // the entry names a free cluster: nothing of ours is there
                    }
                    break;
                }
                v if v >= 0x0FFF_FFF8 => break,
                v => c = v,
            }
        }
        // every live entry of the whole partition, its chain checked against the marks
        let hit = |c: u32| c >= 2 && c as u64 <= last && mark[c as usize / 8] & (1 << (c % 8)) != 0;
        let mut stack = [0u32; 32];
        let (mut sp, mut seen) = (1usize, 0u64);
        stack[0] = self.fat.root;
        let mut touched = false;
        while sp > 0 && !touched {
            sp -= 1;
            let d = stack[sp];
            // the folder's own chain
            let (mut k, mut steps) = (d, 0u64);
            while k >= 2 && (k as u64) <= last && steps <= self.fat.clusters {
                if hit(k) {
                    touched = true;
                    break;
                }
                steps += 1;
                match self.fat.next_c(k, &mut cache) {
                    Some(v) if v >= 2 && v < 0x0FFF_FFF8 => k = v,
                    _ => break,
                }
            }
            let mut deep = false;
            self.fat.dir(d, &mut |e| {
                seen += 1;
                if seen > 1_000_000 {
                    deep = true;
                    return true;
                }
                if e.clus < 2 {
                    return false;
                }
                if e.attr & 0x10 != 0 {
                    if sp < stack.len() {
                        stack[sp] = e.clus;
                        sp += 1;
                    } else {
                        deep = true;
                        return true;
                    }
                    return false;
                }
                let (mut k, mut steps) = (e.clus, 0u64);
                while k >= 2 && (k as u64) <= last && steps <= self.fat.clusters {
                    if hit(k) {
                        touched = true;
                        return true;
                    }
                    steps += 1;
                    match self.fat.next_c(k, &mut cache) {
                        Some(v) if v >= 2 && v < 0x0FFF_FFF8 => k = v,
                        _ => break,
                    }
                }
                false
            })?;
            if deep {
                return Ok(None); // too deep or too large to be sure: left alone
            }
        }
        Ok(if touched { None } else { Some(n) })
    }

    /// Frees the orphan chain from `first` in every FAT copy: as Fat::release, but a link into a free cluster (a cut
    /// part way through `take`) ends it too.
    fn release_orphan(&self, mut c: u32) -> bool {
        let f = &self.fat;
        let mut k = u64::MAX;
        let mut a = [0u8; 512];
        for _ in 0..=f.clusters {
            let ck = c as u64 / 128;
            if ck != k {
                if k != u64::MAX && !f.fat_put(k, &a) {
                    return false;
                }
                if !f.dk.read(f.fat0 + ck, &mut a) {
                    return false;
                }
                k = ck;
            }
            let o = (c as usize % 128) * 4;
            let v = u32::from_le_bytes([a[o], a[o + 1], a[o + 2], a[o + 3]]);
            a[o..o + 4].copy_from_slice(&(v & 0xF000_0000).to_le_bytes());
            match v & 0x0FFF_FFFF {
                n if n >= 0x0FFF_FFF8 || n == 0 => return f.fat_put(k, &a),
                n if n >= 2 && n as u64 <= f.clusters + 1 => c = n,
                _ => return false,
            }
        }
        false
    }

    /// FSInfo's free count set to what the FAT says now (after a cut it is stale: `take` writes it last), when the
    /// partition keeps one.
    fn fsinfo_exact(&self) -> bool {
        let f = &self.fat;
        let Ok((free, _, _)) = f.free() else { return false };
        let mut s = [0u8; 512];
        if !f.dk.read(f.part, &mut s) {
            return false;
        }
        let fsi = u16::from_le_bytes([s[48], s[49]]) as u64;
        if fsi == 0 || fsi == 0xFFFF {
            return true;
        }
        let n = f.part + fsi * (f.bps / 512);
        if !f.dk.read(n, &mut s) {
            return false;
        }
        let sig = |o: usize| u32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
        if sig(0) != 0x4161_5252 || sig(484) != 0x6141_7272 || sig(508) != 0xAA55_0000 || sig(488) == 0xFFFF_FFFF {
            return true;
        }
        s[488..492].copy_from_slice(&(free as u32).to_le_bytes());
        f.dk.write(n, &s)
    }

    /// Writes `long` (its bytes from `src`, `size` bytes) into `\EFI\afrisovera`, in the order of the module comment.
    /// Ok(its first cluster).
    pub fn write(&self, long: &str, size: u64, src: &mut impl Source) -> Result<u32, &'static str> {
        let cb = self.fat.cluster_bytes();
        let need = size.div_ceil(cb).max(1);
        // an 8.3 name no live entry in the folder has
        let mut used = [false; 10];
        self.fat.slots(self.dir, &mut |_, _, e| {
            if e[0] == 0 {
                return true;
            }
            if e[0] != 0xE5 && e[11] != 0x0F && &e[..7] == b"AFRISO~" && (b'1'..=b'9').contains(&e[7]) {
                used[(e[7] - b'0') as usize] = true;
            }
            false
        })?;
        let tilde = (1..=9u8).find(|&i| !used[i as usize]).ok_or("\\EFI\\afrisovera already has AFRISO~1 to AFRISO~9; not changed")?;
        let short = short_of(tilde);
        let mut pieces = [[0u8; 32]; 4];
        let np = lfn(long, &short, &mut pieces);
        // np + 1 free slots side by side in ONE sector, so turning them live is one write
        let want = np + 1;
        let (mut run_s, mut run_o, mut rn, mut ended) = (0u64, 0usize, 0usize, false);
        self.fat.slots(self.dir, &mut |sn, o, e| {
            if e[0] == 0 {
                ended = true;
            }
            let free = ended || e[0] == 0xE5;
            if !free || (rn > 0 && sn != run_s) {
                rn = 0;
            }
            if free {
                if rn == 0 {
                    run_s = sn;
                    run_o = o;
                }
                rn += 1;
            }
            rn == want
        })?;
        if rn < want {
            return Err("\\EFI\\afrisovera has no room for the entries in one sector (it would have to grow); not changed");
        }
        // the first cluster take() will use: the lowest free one
        let first = self.lowest_free()?.ok_or("no free cluster")?;
        let mut live = [0u8; 160];
        for (i, p) in pieces[..np].iter().enumerate() {
            live[32 * i..32 * i + 32].copy_from_slice(p);
        }
        gpt::dirent(&mut live[32 * np..32 * np + 32], &short, 0x20, first, size as u32);
        let mut dead = live;
        for i in 0..want {
            dead[32 * i] = 0xE5;
        }
        // 1: the entries, marked deleted
        let mut s = [0u8; 512];
        if !self.fat.dk.read(run_s, &mut s) {
            return Err("could not read the folder");
        }
        s[run_o..run_o + 32 * want].copy_from_slice(&dead[..32 * want]);
        if !(self.fat.dk.write(run_s, &s) && self.fat.dk.settle()) {
            return Err("a write FAILED; nothing points at the spare file");
        }
        // 2: the file's clusters, then 3: FSInfo
        let fc = self.fat.take(need, src)?;
        if fc != first {
            // not expected (take uses the lowest free cluster); the deleted entry is set right before going live
            gpt::dirent(&mut live[32 * np..32 * np + 32], &short, 0x20, fc, size as u32);
        }
        if !self.fat.fsinfo(-(need as i64)) {
            return Err("a write FAILED (FSInfo)");
        }
        // 4: live, one sector write, the last
        if !self.fat.dk.read(run_s, &mut s) {
            return Err("could not read the folder again");
        }
        s[run_o..run_o + 32 * want].copy_from_slice(&live[..32 * want]);
        if !(self.fat.dk.write(run_s, &s) && self.fat.dk.settle()) {
            return Err("a write FAILED turning the spare file's entry live");
        }
        Ok(fc)
    }

    /// The lowest free cluster.
    fn lowest_free(&self) -> Result<Option<u32>, &'static str> {
        let last = self.fat.clusters + 1;
        let mut a = [0u8; 512];
        for k in 0..self.fat.fat512 {
            if k * 128 > last {
                break;
            }
            if !self.fat.dk.read(self.fat.fat0 + k, &mut a) {
                return Err("could not read the FAT");
            }
            for j in 0..128u64 {
                let c = k * 128 + j;
                if c >= 2 && c <= last && u32::from_le_bytes([a[4 * j as usize], a[4 * j as usize + 1], a[4 * j as usize + 2], a[4 * j as usize + 3]]) & 0x0FFF_FFFF == 0 {
                    return Ok(Some(c as u32));
                }
            }
        }
        Ok(None)
    }

    /// The SHA-256 of `long` as read back from the partition, opened afresh: its size must be `size` and its chain
    /// exactly that long, and both FAT copies equal. Ok((sha, free clusters now)).
    pub fn read_back(&self, long: &str, size: u64) -> Result<([u8; 32], u64), &'static str> {
        let f = Fat::open(self.s.dk, self.s.part, self.s.len).map_err(|_| "the EFI partition could not be opened again")?;
        let again = Esp { s: self.s, fat: f, dir: self.dir };
        let live = again.live(long)?.ok_or("the spare file is not there when read back")?;
        if live.size as u64 != size {
            return Err("the spare file has the wrong size when read back");
        }
        let fat = &again.fat;
        let mut h = crate::sha256::Sha256::new();
        let (mut c, mut off) = (live.clus, 0u64);
        let mut cache = (u64::MAX, [0u8; 512]);
        let mut s = [0u8; 512];
        loop {
            if c < 2 || c as u64 > fat.clusters + 1 {
                return Err("the spare file's cluster chain is damaged when read back");
            }
            for q in 0..fat.c512() {
                if off >= size {
                    break;
                }
                if !fat.dk.read(fat.at(c) + q, &mut s) {
                    return Err("could not read the spare file back");
                }
                let k = (size - off).min(512) as usize;
                h.update(&s[..k]);
                off += k as u64;
            }
            let n = fat.next_c(c, &mut cache).ok_or("could not read the FAT")?;
            if off >= size {
                if n < 0x0FFF_FFF8 {
                    return Err("the spare file's chain is longer than its size");
                }
                break;
            }
            if n >= 0x0FFF_FFF8 || n == 0 {
                return Err("the spare file's chain is shorter than its size");
            }
            c = n;
        }
        let free = again.free()?;
        Ok((h.finish(), free))
    }
}
