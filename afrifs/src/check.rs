//! `vfscheck` (spec 7.3): walks every block the last commit reaches, checks every pointer, every inode and every
//! folder record, and compares the blocks reached with the free map. It never writes.

use crate::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub files: u64,
    pub folders: u64,
    pub blocks_reached: u64,
    /// Blocks the free map marks used that nothing reaches (space lost, not damage).
    pub leaked: u64,
    /// Every problem found, in words. Empty: the volume is clean.
    pub problems: Vec<String>,
}

impl Report {
    pub fn clean(&self) -> bool {
        self.problems.is_empty() && self.leaked == 0
    }
}

struct Walk<'a, D: Disk> {
    fs: &'a mut Fs<D>,
    reached: BTreeSet<u64>,
    r: Report,
}

impl<'a, D: Disk> Walk<'a, D> {
    fn problem(&mut self, s: String) {
        if self.r.problems.len() < 1000 {
            self.r.problems.push(s);
        }
    }

    /// Marks a block reached and reads it; a second reach or a bad checksum is a problem.
    fn reach(&mut self, p: Ptr, what: &str) -> Option<Blk> {
        if p.blk < 2 || p.blk >= self.fs.label.area_blocks {
            self.problem(format!("{}: pointer to block {} outside the area", what, p.blk));
            return None;
        }
        if !self.reached.insert(p.blk) {
            self.problem(format!("{}: block {} is reached twice", what, p.blk));
            return None;
        }
        match self.fs.read_ptr(p) {
            Ok(b) => Some(b),
            Err(e) => {
                self.problem(format!("{}: block {} {:?}", what, p.blk, e));
                None
            }
        }
    }

    /// Walks a radix tree; calls `leaf(index, block)` for each leaf that reads.
    fn tree(&mut self, p: Ptr, h: u32, base: u64, what: &str, leaves: &mut Vec<(u64, Blk)>) {
        if p.is_null() {
            return;
        }
        let b = match self.reach(p, what) {
            Some(b) => b,
            None => return,
        };
        if h == 0 {
            leaves.push((base, b));
            return;
        }
        for slot in 0..PTRS {
            let c = Ptr::decode(&b, 16 * slot);
            self.tree(c, h - 1, base + slot as u64 * span(h - 1), what, leaves);
        }
    }

    /// A file's or folder's data blocks, in order (index, block).
    fn data(&mut self, n: &Inode, what: &str) -> Vec<(u64, Blk)> {
        let mut out = Vec::new();
        for s in 0..8usize {
            let p = n.ptrs[s];
            if !p.is_null() {
                if let Some(b) = self.reach(p, what) {
                    out.push((s as u64, b));
                }
            }
        }
        for (slot, h, lo) in [(8usize, 1u32, DIRECT), (9, 2, SINGLE_END), (10, 3, DOUBLE_END)] {
            let mut leaves = Vec::new();
            self.tree(n.ptrs[slot], h, 0, what, &mut leaves);
            for (i, b) in leaves {
                out.push((lo + i, b));
            }
        }
        let nb = blocks_for(n.size);
        if out.iter().any(|(i, _)| *i >= nb) {
            self.problem(format!("{}: a block past its size", what));
        }
        out
    }
}

/// Checks the last commit (the open transaction is not looked at).
pub fn check<D: Disk>(fs: &mut Fs<D>) -> Report {
    let root = fs.root;
    let area = fs.label.area_blocks;
    let mut w = Walk { fs, reached: BTreeSet::new(), r: Report::default() };
    w.reached.insert(0);
    w.reached.insert(1);
    // the free map
    let mut map_leaves_read = Vec::new();
    w.tree(root.map_root, root.map_height as u32, 0, "free map", &mut map_leaves_read);
    if map_leaves_read.len() as u64 != map_leaves(area) {
        w.problem(format!("free map: {} of {} leaves read", map_leaves_read.len(), map_leaves(area)));
    }
    // the inode tree
    let mut ileaves = Vec::new();
    w.tree(root.inode_root, root.inode_height as u32, 0, "inode tree", &mut ileaves);
    let mut inodes: BTreeMap<u64, Inode> = BTreeMap::new();
    for (leaf, b) in ileaves.iter() {
        for k in 0..INODES_PER_BLOCK {
            let ino = leaf * INODES_PER_BLOCK + k;
            let o = k as usize * 256;
            let n = Inode::decode(&b[o..o + 256]);
            if n.kind == KIND_FREE {
                if b[o..o + 256].iter().any(|&x| x != 0) {
                    w.problem(format!("inode {}: free but not zero", ino));
                }
                continue;
            }
            if ino == 0 || n.ino != ino || (n.kind != KIND_FILE && n.kind != KIND_FOLDER) || ino >= root.next_ino {
                w.problem(format!("inode {}: bad inode", ino));
                continue;
            }
            if n.size > MAX_FILE {
                w.problem(format!("inode {}: size past 4 GiB", ino));
            }
            inodes.insert(ino, n);
        }
    }
    if inodes.len() as u64 != root.inodes {
        w.problem(format!("{} inodes in use, the root record says {}", inodes.len(), root.inodes));
    }
    // the folders from the top: every inode reached exactly once
    let mut named: BTreeMap<u64, u32> = BTreeMap::new();
    let mut stack: Vec<(u64, usize)> = vec![(ROOT_INO, 0)];
    match inodes.get(&ROOT_INO) {
        Some(n) if n.kind == KIND_FOLDER => {}
        _ => w.problem(String::from("the top folder is missing")),
    }
    named.insert(ROOT_INO, 1);
    let mut seen_data: BTreeSet<u64> = BTreeSet::new();
    while let Some((ino, depth)) = stack.pop() {
        let n = match inodes.get(&ino) {
            Some(n) => *n,
            None => continue,
        };
        if !seen_data.insert(ino) {
            continue;
        }
        let what = format!("inode {}", ino);
        let blocks = w.data(&n, &what);
        if n.kind == KIND_FILE {
            w.r.files += 1;
            continue;
        }
        w.r.folders += 1;
        if n.size % BLOCK as u64 != 0 {
            w.problem(format!("folder {}: size not whole blocks", ino));
        }
        let mut names: BTreeSet<Vec<u8>> = BTreeSet::new();
        for (_, b) in blocks.iter() {
            let mut found: Vec<(u64, u8, Vec<u8>)> = Vec::new();
            if scan_records(b, |_, i, k, nm, _| {
                if i != 0 {
                    found.push((i, k, nm.to_vec()));
                }
                true
            })
            .is_err()
            {
                w.problem(format!("folder {}: a bad record", ino));
                continue;
            }
            for (child, kind, name) in found {
                if !names.insert(name.clone()) {
                    w.problem(format!("folder {}: a name appears twice", ino));
                }
                match inodes.get(&child) {
                    Some(c) if c.kind == kind => {}
                    Some(_) => w.problem(format!("folder {}: record kind differs from inode {}", ino, child)),
                    None => {
                        w.problem(format!("folder {}: names inode {}, which is not in use", ino, child));
                        continue;
                    }
                }
                let c = named.entry(child).or_insert(0);
                *c += 1;
                if *c > 1 {
                    w.problem(format!("inode {} is named twice", child));
                    continue;
                }
                if depth + 1 > MAX_DEPTH + 1 {
                    w.problem(format!("folder {}: deeper than 64", ino));
                    continue;
                }
                stack.push((child, depth + 1));
            }
        }
    }
    for ino in inodes.keys() {
        if !named.contains_key(ino) {
            w.problem(format!("inode {} is in use but no folder names it", ino));
        }
    }
    // the blocks reached against the free map
    let mut bits: BTreeMap<u64, Blk> = BTreeMap::new();
    for (l, b) in map_leaves_read {
        bits.insert(l, b);
    }
    let mut marked = 0u64;
    let mut reached_free = 0u64;
    for blk in 0..area {
        let used = match bits.get(&(blk / BITS_PER_LEAF)) {
            Some(b) => {
                let i = blk % BITS_PER_LEAF;
                b[(i >> 3) as usize] & (1 << (i & 7)) != 0
            }
            None => true,
        };
        let reached = w.reached.contains(&blk);
        if used {
            marked += 1;
            if !reached {
                w.r.leaked += 1;
            }
        } else if reached {
            reached_free += 1;
        }
    }
    if reached_free > 0 {
        w.problem(format!("{} blocks in use are marked free", reached_free));
    }
    if marked != root.used {
        w.problem(format!("the free map marks {} blocks used, the root record says {}", marked, root.used));
    }
    // bits past the area's end must be 1
    if let Some(b) = bits.get(&((area - 1) / BITS_PER_LEAF)) {
        let last = area / BITS_PER_LEAF;
        if last == (area - 1) / BITS_PER_LEAF {
            for i in (area % BITS_PER_LEAF)..BITS_PER_LEAF {
                if b[(i >> 3) as usize] & (1 << (i & 7)) == 0 {
                    w.problem(String::from("free map: a bit past the area's end is 0"));
                    break;
                }
            }
        }
    }
    w.r.blocks_reached = w.reached.len() as u64;
    w.r
}

impl<D: Disk> Fs<D> {
    /// `vfscheck`: see [`check`].
    pub fn check(&mut self) -> Report {
        check(self)
    }
}
