//! The foreign program's memory calls of L1.1 (COMPAT_L1_PLAN 4, 8 and 9): brk, mmap (anonymous and private), munmap
//! and mprotect, as pure bookkeeping over a `Mapper` (the kernel's fproc_map, fproc_unmap and fproc_protect, or a
//! fake one in the tests), so every rule is tested on the host.
//!
//! Layout inside the foreign area (FOREIGN_AREA, 1 GiB):
//! - the program's image, from its lowest segment to `image_end`;
//! - the brk heap, from `image_end` (page-rounded) up, at most `BRK_MAX`;
//! - mmap regions, placed top-down from just below the stack's guard page;
//! - the stack (1 MiB) and its guard page at the top (the bridge maps the stack itself).
//!
//! The architect's rules (COMPAT_L1_PLAN 8 point 1 and section 9):
//! - after the start a protection change may only remove rights; adding EXEC, or adding WRITE to an R or RX page,
//!   answers EACCES; write and execute are never together;
//! - PROT_NONE frees the frames, so rights never come back from it: ENOMEM plus the named line;
//! - MAP_FIXED is munmap then a fresh map (zeroed frames, nothing old leaks).
//!
//! Written from the Linux manual pages brk(2), mmap(2), munmap(2), mprotect(2) and errno(3). No Linux or C library
//! source was read (the clean-room rule).

pub const PAGE: u64 = 4096;
/// The brk heap's limit (Linux's default rlimit is far larger; the pair's 32 MiB cap is the real bound).
pub const BRK_MAX: u64 = 64 << 20;
/// Mappings tracked one by one (each mmap, and each piece a munmap or mprotect splits off).
pub const MAX_MAPS: usize = 64;

pub const PROT_READ: u64 = 1;
pub const PROT_WRITE: u64 = 2;
pub const PROT_EXEC: u64 = 4;
pub const MAP_PRIVATE: u64 = 0x02;
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;

pub const EACCES: i64 = 13;
pub const EINVAL: i64 = 22;
pub const ENOMEM: i64 = 12;
pub const ENODEV: i64 = 19;

/// What the bridge can do to the foreign process's pages.
pub trait Mapper {
    /// Map zeroed pages read-write (fproc_map); Err when the kernel refuses (the cap, a bad range).
    fn map(&mut self, addr: u64, len: u64) -> Result<(), ()>;
    fn unmap(&mut self, addr: u64, len: u64) -> Result<(), ()>;
    /// Set the pages' rights (fproc_protect); `prot` is PROT_* bits. Err when the kernel refuses.
    fn protect(&mut self, addr: u64, len: u64, prot: u64) -> Result<(), ()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Map {
    start: u64,
    len: u64,
    prot: u64,
}

/// The program's memory, as the bridge knows it.
pub struct Memory {
    brk_start: u64,
    brk: u64,
    mmap_top: u64,
    area_start: u64,
    maps: [Map; MAX_MAPS],
    used: usize,
    /// Set when an mprotect was refused because it asked rights back from PROT_NONE; the bridge prints
    /// "mprotect back from PROT_NONE is not supported: the pages were freed" and clears it.
    pub back_from_none: bool,
}

fn up(v: u64) -> Option<u64> {
    v.checked_add(PAGE - 1).map(|x| x & !(PAGE - 1))
}

impl Memory {
    /// `image_end`: the end of the program's highest segment; `mmap_top`: the lowest address of the stack's guard
    /// page (nothing is placed at or above it); `area_start`: the foreign area's start.
    pub fn new(area_start: u64, image_end: u64, mmap_top: u64) -> Self {
        let b = up(image_end).unwrap_or(mmap_top);
        Memory { brk_start: b, brk: b, mmap_top, area_start, maps: [Map::default(); MAX_MAPS], used: 0, back_from_none: false }
    }

    /// Pages the bridge mapped itself before the start (the program's segments, its stack), with their rights, so
    /// that the program's mprotect and munmap on them follow the same rules (first the cloud runner run of L1.1, 21 Nov: mprot1's
    /// "add WRITE to the code page" was ENOMEM, the page unknown here, instead of EACCES). No kernel call: the pages
    /// are mapped already. Whole pages; false when the table is full or the run is not whole pages.
    pub fn known(&mut self, start: u64, len: u64, prot: u64) -> bool {
        if start % PAGE != 0 || len == 0 || len % PAGE != 0 || self.used == MAX_MAPS {
            return false;
        }
        self.maps[self.used] = Map { start, len, prot };
        self.used += 1;
        true
    }

    /// brk(2) as the kernel call (not the C wrapper): the new break, or the old one when refused.
    pub fn brk(&mut self, m: &mut dyn Mapper, want: u64) -> u64 {
        if want == 0 || want < self.brk_start || want - self.brk_start > BRK_MAX {
            return self.brk;
        }
        let (old_end, new_end) = (up(self.brk).unwrap_or(u64::MAX), up(want).unwrap_or(u64::MAX));
        if new_end > old_end {
            // the heap may not reach any mmap region
            if self.overlaps(old_end, new_end - old_end) || new_end > self.mmap_top {
                return self.brk;
            }
            if m.map(old_end, new_end - old_end).is_err() {
                return self.brk;
            }
        } else if new_end < old_end && m.unmap(new_end, old_end - new_end).is_err() {
            return self.brk;
        }
        self.brk = want;
        self.brk
    }

    fn overlaps(&self, a: u64, len: u64) -> bool {
        self.maps[..self.used].iter().any(|r| a < r.start + r.len && r.start < a + len)
    }

    /// A free run of `len` bytes, top-down below mmap_top and above the brk heap's limit.
    fn find(&self, len: u64) -> Option<u64> {
        let floor = self.brk_start.saturating_add(BRK_MAX);
        let mut top = self.mmap_top;
        loop {
            let start = top.checked_sub(len)?;
            if start < floor {
                return None;
            }
            // the lowest region that overlaps [start, top), if any: go below it
            match self.maps[..self.used].iter().filter(|r| start < r.start + r.len && r.start < top).map(|r| r.start).min() {
                None => return Some(start),
                Some(s) => top = s,
            }
        }
    }

    /// mmap(2), anonymous and private only in L1.1 (private file maps come with L1.2): the address, or -errno.
    pub fn mmap(&mut self, m: &mut dyn Mapper, addr: u64, len: u64, prot: u64, flags: u64, fd: i64) -> i64 {
        if len == 0 || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
            return -EINVAL;
        }
        if flags & MAP_PRIVATE == 0 || flags & MAP_ANONYMOUS == 0 || fd != -1 {
            return -ENODEV; // shared maps and file maps: not in L1.1
        }
        if prot & PROT_EXEC != 0 {
            // W^X, and EXECUTE only before the start (AFR-0001 7.7): the program runs already, so no new code pages
            return -EACCES;
        }
        let Some(len) = up(len) else { return -ENOMEM };
        let start = if flags & MAP_FIXED != 0 {
            match addr.checked_add(len) {
                Some(e) if addr % PAGE == 0 && addr >= self.brk_start.saturating_add(BRK_MAX) && e <= self.mmap_top => {}
                _ => return -EINVAL,
            }
            // MAP_FIXED replaces what was there: munmap, then a fresh map (the architect's verdict)
            let r = self.munmap(m, addr, len);
            if r != 0 {
                return r;
            }
            addr
        } else {
            match self.find(len) {
                Some(s) => s,
                None => return -ENOMEM,
            }
        };
        if self.used == MAX_MAPS {
            return -ENOMEM;
        }
        if m.map(start, len).is_err() {
            return -ENOMEM;
        }
        if prot != PROT_READ | PROT_WRITE && m.protect(start, len, prot).is_err() {
            let _ = m.unmap(start, len);
            return -ENOMEM;
        }
        self.maps[self.used] = Map { start, len, prot };
        self.used += 1;
        start as i64
    }

    /// Split the tracked region containing `at` so that `at` is a boundary; false when the table is full.
    fn split_at(&mut self, at: u64) -> bool {
        if let Some(i) = self.maps[..self.used].iter().position(|r| r.start < at && at < r.start + r.len) {
            if self.used == MAX_MAPS {
                return false;
            }
            let r = self.maps[i];
            self.maps[i].len = at - r.start;
            self.maps[self.used] = Map { start: at, len: r.start + r.len - at, prot: r.prot };
            self.used += 1;
        }
        true
    }

    /// munmap(2): 0 or -errno. Unmapping a range with nothing in it is fine, as on Linux.
    pub fn munmap(&mut self, m: &mut dyn Mapper, addr: u64, len: u64) -> i64 {
        let Some(len) = up(len) else { return -EINVAL };
        if addr % PAGE != 0 || len == 0 || addr.checked_add(len).is_none() || addr < self.area_start {
            return -EINVAL;
        }
        if !self.split_at(addr) || !self.split_at(addr + len) {
            return -ENOMEM;
        }
        let mut i = 0;
        while i < self.used {
            let r = self.maps[i];
            if r.start >= addr && r.start + r.len <= addr + len {
                if m.unmap(r.start, r.len).is_err() {
                    return -EINVAL;
                }
                self.used -= 1;
                self.maps[i] = self.maps[self.used];
            } else {
                i += 1;
            }
        }
        0
    }

    /// mprotect(2) after the start: only removing rights (the architect's rule). 0 or -errno.
    pub fn mprotect(&mut self, m: &mut dyn Mapper, addr: u64, len: u64, prot: u64) -> i64 {
        let Some(len) = up(len) else { return -EINVAL };
        if addr % PAGE != 0 || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
            return -EINVAL;
        }
        if len == 0 {
            return 0;
        }
        let end = match addr.checked_add(len) {
            Some(e) => e,
            None => return -ENOMEM,
        };
        // every page of the range must be one of our regions (Linux: ENOMEM for an unmapped page)
        let mut covered = 0;
        for r in &self.maps[..self.used] {
            let (s, e) = (r.start.max(addr), (r.start + r.len).min(end));
            if s < e {
                covered += e - s;
                if r.prot == 0 && prot != 0 {
                    // rule 1: PROT_NONE gave the frames back, so rights never return; ENOMEM, and the bridge prints
                    // the named line (never fresh zeroed pages where old data is expected)
                    self.back_from_none = true;
                    return -ENOMEM;
                }
                if prot & !r.prot != 0 {
                    return -EACCES; // a right that is not there now: adding is refused
                }
            }
        }
        if covered != len {
            return -ENOMEM;
        }
        // nothing changes: no kernel call (the kernel never takes WRITE as a target, even when it is already there)
        if self.maps[..self.used].iter().all(|r| r.start >= end || r.start + r.len <= addr || r.prot == prot) {
            return 0;
        }
        if !self.split_at(addr) || !self.split_at(end) {
            return -ENOMEM;
        }
        if m.protect(addr, len, prot).is_err() {
            return -EACCES;
        }
        for r in self.maps[..self.used].iter_mut() {
            if r.start >= addr && r.start + r.len <= end {
                r.prot = prot;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    const AREA: u64 = 0x100_0000_0000;
    const IMAGE_END: u64 = AREA + 0x40_5123;
    const TOP: u64 = AREA + 0x3FE0_0000; // below a 1 MiB stack and its guard page

    /// Every page's rights in a sorted map, so overlaps and holes show.
    #[derive(Default)]
    struct Fake {
        pages: std::collections::BTreeMap<u64, u64>,
        cap_pages: usize,
    }
    impl Mapper for Fake {
        fn map(&mut self, a: u64, len: u64) -> Result<(), ()> {
            assert_eq!((a % PAGE, len % PAGE), (0, 0));
            if self.pages.len() + (len / PAGE) as usize > self.cap_pages {
                return Err(());
            }
            for p in (a..a + len).step_by(PAGE as usize) {
                assert!(self.pages.insert(p, PROT_READ | PROT_WRITE).is_none(), "mapped twice: {:#x}", p);
            }
            Ok(())
        }
        fn unmap(&mut self, a: u64, len: u64) -> Result<(), ()> {
            for p in (a..a + len).step_by(PAGE as usize) {
                assert!(self.pages.remove(&p).is_some(), "unmapped a hole: {:#x}", p);
            }
            Ok(())
        }
        fn protect(&mut self, a: u64, len: u64, prot: u64) -> Result<(), ()> {
            assert!(prot & PROT_WRITE == 0 || prot & PROT_EXEC == 0, "W and X together reached the kernel");
            for p in (a..a + len).step_by(PAGE as usize) {
                *self.pages.get_mut(&p).expect("protect on a hole") = prot;
            }
            Ok(())
        }
    }
    fn fake() -> Fake {
        Fake { cap_pages: 8192, ..Default::default() }
    }
    const ANON: u64 = MAP_PRIVATE | MAP_ANONYMOUS;

    #[test]
    fn brk_grows_shrinks_and_refuses_past_its_limit() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        let b0 = m.brk(&mut f, 0);
        assert_eq!(b0, AREA + 0x40_6000);
        assert_eq!(m.brk(&mut f, b0 + 10), b0 + 10);
        assert_eq!(f.pages.len(), 1);
        assert_eq!(m.brk(&mut f, b0 + 3 * PAGE), b0 + 3 * PAGE);
        assert_eq!(f.pages.len(), 3);
        assert_eq!(m.brk(&mut f, b0 + PAGE), b0 + PAGE);
        assert_eq!(f.pages.len(), 1);
        assert_eq!(m.brk(&mut f, b0 + BRK_MAX + 1), b0 + PAGE); // past the limit: the old break
        assert_eq!(m.brk(&mut f, b0 - PAGE), b0 + PAGE); // below the start: the old break
        let mut small = Fake { cap_pages: 2, ..Default::default() };
        let mut m2 = Memory::new(AREA, IMAGE_END, TOP);
        assert_eq!(m2.brk(&mut small, b0 + 5 * PAGE), b0); // the kernel's cap refused: the old break
    }

    #[test]
    fn mmap_places_top_down_without_overlap_and_munmap_gives_back() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        let a = m.mmap(&mut f, 0, 3 * PAGE, PROT_READ | PROT_WRITE, ANON, -1);
        let b = m.mmap(&mut f, 0, 5000, PROT_READ, ANON, -1);
        assert_eq!(a as u64, TOP - 3 * PAGE);
        assert_eq!(b as u64, TOP - 5 * PAGE);
        assert_eq!(f.pages[&(b as u64)], PROT_READ);
        assert_eq!(m.munmap(&mut f, a as u64 + PAGE, PAGE), 0);
        assert_eq!(f.pages.len(), 4);
        assert_eq!(m.munmap(&mut f, AREA + 0x1000_0000, PAGE), 0); // unmapping nothing is fine
        assert_eq!(m.munmap(&mut f, a as u64 + 1, PAGE), -EINVAL);
        assert_eq!(m.munmap(&mut f, b as u64, 7 * PAGE), 0);
        assert!(f.pages.is_empty());
    }

    #[test]
    fn mmap_refusals() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        assert_eq!(m.mmap(&mut f, 0, 0, PROT_READ, ANON, -1), -EINVAL);
        assert_eq!(m.mmap(&mut f, 0, PAGE, PROT_READ, MAP_ANONYMOUS | 0x01, -1), -ENODEV); // MAP_SHARED
        assert_eq!(m.mmap(&mut f, 0, PAGE, PROT_READ, MAP_PRIVATE, 3), -ENODEV); // a file: L1.2
        assert_eq!(m.mmap(&mut f, 0, PAGE, PROT_READ | PROT_WRITE | PROT_EXEC, ANON, -1), -EACCES);
        assert_eq!(m.mmap(&mut f, 0, PAGE, PROT_READ | PROT_EXEC, ANON, -1), -EACCES); // no new code after the start
        assert_eq!(m.mmap(&mut f, 0, u64::MAX - 10, PROT_READ, ANON, -1), -ENOMEM);
        assert_eq!(m.mmap(&mut f, 0, 2 << 30, PROT_READ, ANON, -1), -ENOMEM); // larger than the area
        assert_eq!(m.mmap(&mut f, AREA + 0x123, PAGE, PROT_READ, ANON | MAP_FIXED, -1), -EINVAL);
        assert_eq!(m.mmap(&mut f, AREA + 0x40_0000, PAGE, PROT_READ, ANON | MAP_FIXED, -1), -EINVAL); // the image
        assert_eq!(m.mmap(&mut f, TOP, PAGE, PROT_READ, ANON | MAP_FIXED, -1), -EINVAL); // the stack's guard
        let mut small = Fake { cap_pages: 1, ..Default::default() };
        assert_eq!(m.mmap(&mut small, 0, 2 * PAGE, PROT_READ, ANON, -1), -ENOMEM); // the kernel's cap
        assert!(f.pages.is_empty() && small.pages.is_empty());
    }

    #[test]
    fn map_fixed_replaces() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        let a = m.mmap(&mut f, 0, 4 * PAGE, PROT_READ, ANON, -1) as u64;
        assert_eq!(m.mmap(&mut f, a + PAGE, PAGE, PROT_READ | PROT_WRITE, ANON | MAP_FIXED, -1), (a + PAGE) as i64);
        assert_eq!(f.pages.len(), 4);
        assert_eq!(f.pages[&(a + PAGE)], PROT_READ | PROT_WRITE);
        assert_eq!(f.pages[&a], PROT_READ);
    }

    #[test]
    fn mprotect_only_removes_rights() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        // a code page the bridge mapped before the start (known, no kernel call): adding WRITE is EACCES, as on any
        // other page; dropping EXEC to R is allowed (first L1.1 the cloud runner run: this page was unknown, so ENOMEM)
        let code = AREA + 0x40_0000;
        f.pages.insert(code, PROT_READ | PROT_EXEC);
        assert!(m.known(code, PAGE, PROT_READ | PROT_EXEC));
        assert!(!m.known(code + 1, PAGE, PROT_READ));
        assert_eq!(m.mprotect(&mut f, code, PAGE, PROT_READ | PROT_WRITE | PROT_EXEC), -EACCES);
        assert_eq!(m.mprotect(&mut f, code, PAGE, PROT_READ), 0);
        assert_eq!(f.pages[&code], PROT_READ);
        let rw = m.mmap(&mut f, 0, 3 * PAGE, PROT_READ | PROT_WRITE, ANON, -1) as u64;
        let ro = m.mmap(&mut f, 0, PAGE, PROT_READ, ANON, -1) as u64;
        assert_eq!(m.mprotect(&mut f, rw + PAGE, PAGE, PROT_READ), 0);
        assert_eq!((f.pages[&rw], f.pages[&(rw + PAGE)], f.pages[&(rw + 2 * PAGE)]), (3, 1, 3));
        assert_eq!(m.mprotect(&mut f, rw + 2 * PAGE, PAGE, 0), 0);
        // back from PROT_NONE: ENOMEM and the flag for the named line (rule 1); none to none is fine
        assert_eq!(m.mprotect(&mut f, rw + 2 * PAGE, PAGE, PROT_READ), -ENOMEM);
        assert!(m.back_from_none);
        m.back_from_none = false;
        assert_eq!(m.mprotect(&mut f, rw + 2 * PAGE, PAGE, 0), 0);
        assert!(!m.back_from_none);
        // adding: refused, nothing changes
        assert_eq!(m.mprotect(&mut f, ro, PAGE, PROT_READ | PROT_WRITE), -EACCES);
        assert_eq!(m.mprotect(&mut f, rw, PAGE, PROT_READ | PROT_WRITE | PROT_EXEC), -EACCES);
        assert_eq!(m.mprotect(&mut f, rw + PAGE, PAGE, PROT_READ | PROT_WRITE), -EACCES);
        assert_eq!(f.pages[&ro], PROT_READ);
        assert_eq!(m.mprotect(&mut f, rw, PAGE, PROT_READ | PROT_WRITE), 0); // the same rights: allowed
        assert_eq!(m.mprotect(&mut f, rw - 8 * PAGE, PAGE, PROT_READ), -ENOMEM); // an unmapped page
        assert_eq!(m.mprotect(&mut f, ro, 2 * PAGE, PROT_READ), 0); // across two regions, removing W
        assert_eq!(m.mprotect(&mut f, rw + 1, PAGE, PROT_READ), -EINVAL);
        assert_eq!(m.munmap(&mut f, ro, 4 * PAGE), 0); // a split region still gives every page back
        assert_eq!(f.pages.keys().copied().collect::<Vec<_>>(), [code]); // only the known code page is left
    }

    #[test]
    fn random_calls_keep_the_books_and_the_pages_equal() {
        let (mut f, mut m) = (fake(), Memory::new(AREA, IMAGE_END, TOP));
        let mut x: u64 = 0x5EED_D00D_1234_5678;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut live: Vec<u64> = Vec::new();
        for _ in 0..20_000 {
            match next() % 4 {
                0 => {
                    let r = m.mmap(&mut f, 0, (next() % 8 + 1) * PAGE, [1, 3, 5, 0][(next() % 4) as usize], ANON, -1);
                    if r > 0 {
                        live.push(r as u64);
                    }
                }
                1 if !live.is_empty() => {
                    let a = live.swap_remove((next() % live.len() as u64) as usize);
                    // ENOMEM only when a split would need a 65th entry; the books stay consistent either way
                    let r = m.munmap(&mut f, a, (next() % 3 + 1) * PAGE);
                    assert!(r == 0 || (r == -ENOMEM && m.used == MAX_MAPS), "munmap {}", r);
                }
                2 if !live.is_empty() => {
                    let a = live[(next() % live.len() as u64) as usize];
                    let _ = m.mprotect(&mut f, a, PAGE, next() % 8);
                }
                _ => {
                    let b = m.brk(&mut f, 0);
                    let _ = m.brk(&mut f, b.wrapping_add((next() % 5) * PAGE).wrapping_sub(2 * PAGE));
                }
            }
            for r in &m.maps[..m.used] {
                for p in (r.start..r.start + r.len).step_by(PAGE as usize) {
                    assert_eq!(f.pages.get(&p).copied(), Some(r.prot), "page {:#x}", p);
                }
                assert!(r.start >= m.brk_start + BRK_MAX && r.start + r.len <= TOP);
            }
        }
    }
}
