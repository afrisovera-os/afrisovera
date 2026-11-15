//! AFR-0011, VFS v1: the AfriSovera file system, format 3 (`AFRIVFS2`).
//!
//! A copy-on-write tree with two checksummed root records (docs/spec/0011-afrifs.md). Nothing the committed tree
//! uses is ever overwritten: a commit writes new blocks into free space, flushes, writes the root record into the
//! slot the last commit did not use, and flushes again (spec 8.1). Every block is checked against the CRC-32C in
//! the pointer that names it (spec 5.1).
//!
//! The crate is `no_std` with `alloc` (the kernel has a heap) and has no unsafe code. It reaches a disk only
//! through [`Disk`]. All numbers on disk are little-endian.
//!
//! What a caller sees:
//! - [`Fs::format`] and [`Fs::mount`];
//! - inode numbers: `1` is the top folder `/`; every call takes the folder's inode number a name is under (the
//!   kernel's handles, step V5, hold those numbers and the rights);
//! - changes collect in one open transaction until [`Fs::sync`] commits them; a cut before the commit's root
//!   record leaves the previous commit whole.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;

pub mod check;
pub mod fuzzing;
pub mod ramdisk;

// ---------------------------------------------------------------------------------------------------------------
// Constants (spec 4 to 6)

pub const SECTOR: usize = 512;
pub const BLOCK: usize = 4096;
const SPB: u64 = (BLOCK / SECTOR) as u64; // sectors per block
/// The sector where the area starts (2 MiB).
pub const AREA_START: u64 = 4096;
/// The smallest partition format 3 accepts (8 MiB).
pub const MIN_SECTORS: u64 = 16384;
/// Format 1 and 2's sectors (1 to 871): never written by format 3 after a migration.
pub const OLD_AREA_END: u64 = 872;
pub const FORMAT: u16 = 3;
pub const LABEL_MAGIC: &[u8; 8] = b"AFRIVFS2";
pub const ROOT_MAGIC: &[u8; 8] = b"AFRIROOT";
const PTRS: usize = BLOCK / 16; // 256 pointers in a pointer block
const INODES_PER_BLOCK: u64 = (BLOCK / 256) as u64; // 16
const BITS_PER_LEAF: u64 = (BLOCK * 8) as u64; // 32768 blocks per bitmap block
const DIRECT: u64 = 8;
const SINGLE_END: u64 = DIRECT + 256; // 264
const DOUBLE_END: u64 = SINGLE_END + 65536; // 65800
/// The largest file (spec 5.3).
pub const MAX_FILE: u64 = 1 << 32;
pub const MAX_NAME: usize = 255;
pub const MAX_PATH: usize = 4096;
pub const MAX_DEPTH: usize = 64;
/// Inode numbers stop here (spec 5.4: an inode tree of height 3).
pub const MAX_INODE: u64 = 1 << 28;
/// The area is capped at 8 TiB (a free map of height 2).
pub const MAX_AREA_BLOCKS: u64 = BITS_PER_LEAF * 256 * 256;

pub const KIND_FREE: u8 = 0;
pub const KIND_FILE: u8 = 1;
pub const KIND_FOLDER: u8 = 2;

pub const FLAG_READ_ONLY: u8 = 1;
pub const FLAG_HIDDEN: u8 = 2;
pub const FLAG_EXEC: u8 = 4;
pub const FLAG_CASEFOLD: u8 = 8;
pub const FLAG_DAMAGED: u8 = 16;

pub const OWNER_PERSON: u32 = 0;
pub const OWNER_SYSTEM: u32 = 1;

/// The top folder.
pub const ROOT_INO: u64 = 1;

/// Dirty data blocks the open transaction keeps in RAM before it writes them early (spec 8.3): 2 MiB.
pub const DEFAULT_DIRTY_CAP: usize = 512;
/// Committed blocks kept in the read cache: 1 MiB.
const CACHE_BLOCKS: usize = 256;

// ---------------------------------------------------------------------------------------------------------------
// The disk

/// A disk (or partition) in 512-byte sectors, as the kernel's block layer gives it. `buf` is a whole number of
/// sectors. Each call returns false when the device failed.
pub trait Disk {
    fn sectors(&self) -> u64;
    fn read(&mut self, sector: u64, buf: &mut [u8]) -> bool;
    fn write(&mut self, sector: u64, buf: &[u8]) -> bool;
    /// Every write acknowledged before this call is on the medium (spec 9).
    fn flush(&mut self) -> bool;
}

// ---------------------------------------------------------------------------------------------------------------
// Errors (spec 11.6)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotFound = 1,
    Exists = 2,
    NotFolder = 3,
    IsFolder = 4,
    NotEmpty = 5,
    NoSpace = 6,
    NameTooLong = 7,
    ReadOnly = 8,
    BadHandle = 9,
    Io = 10,
    Damaged = 11,
    Invalid = 12,
    Quota = 13,
    TooBig = 14,
    Denied = 15,
}

impl Error {
    /// The spec's code (1 to 15).
    pub fn code(self) -> u32 {
        self as u32
    }
}

pub type Result<T> = core::result::Result<T, Error>;

// ---------------------------------------------------------------------------------------------------------------
// CRC-32C (Castagnoli, reflected 0x82F63B78): check value 0xE3069283 for "123456789"

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
            k += 1;
        }
        t[n] = c;
        n += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

pub fn crc32c(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------------------------------------------
// Little-endian helpers

fn get16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn get64(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}
fn put16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// A 4 KiB block on the heap (never on a kernel stack).
pub type Blk = Box<[u8]>;

pub fn zero_block() -> Blk {
    vec![0u8; BLOCK].into_boxed_slice()
}

// ---------------------------------------------------------------------------------------------------------------
// Block pointer (spec 5.1)

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ptr {
    pub blk: u64,
    pub crc: u32,
    pub gen: u32,
}

impl Ptr {
    pub const NULL: Ptr = Ptr { blk: 0, crc: 0, gen: 0 };
    pub fn is_null(&self) -> bool {
        self.blk == 0
    }
    pub fn decode(b: &[u8], o: usize) -> Ptr {
        Ptr { blk: get64(b, o), crc: get32(b, o + 8), gen: get32(b, o + 12) }
    }
    pub fn encode(&self, b: &mut [u8], o: usize) {
        put64(b, o, self.blk);
        put32(b, o + 8, self.crc);
        put32(b, o + 12, self.gen);
    }
    fn to(blk: u64, data: &[u8], gen: u64) -> Ptr {
        Ptr { blk, crc: crc32c(data), gen: gen as u32 }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Inode (spec 5.3)

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inode {
    pub kind: u8,
    pub flags: u8,
    pub owner: u32,
    pub size: u64,
    pub ino: u64,
    pub created: u64,
    pub modified: u64,
    pub links: u32,
    /// direct 0 to 7, then single (8), double (9), triple (10)
    pub ptrs: [Ptr; 11],
}

impl Inode {
    pub fn decode(b: &[u8]) -> Inode {
        let mut ptrs = [Ptr::NULL; 11];
        for (i, p) in ptrs.iter_mut().enumerate() {
            *p = Ptr::decode(b, 48 + 16 * i);
        }
        Inode {
            kind: b[0],
            flags: b[1],
            owner: get32(b, 4),
            size: get64(b, 8),
            ino: get64(b, 16),
            created: get64(b, 24),
            modified: get64(b, 32),
            links: get32(b, 40),
            ptrs,
        }
    }
    pub fn encode(&self, b: &mut [u8]) {
        for x in b[..256].iter_mut() {
            *x = 0;
        }
        if self.kind == KIND_FREE {
            return;
        }
        b[0] = self.kind;
        b[1] = self.flags;
        put32(b, 4, self.owner);
        put64(b, 8, self.size);
        put64(b, 16, self.ino);
        put64(b, 24, self.created);
        put64(b, 32, self.modified);
        put32(b, 40, self.links);
        for (i, p) in self.ptrs.iter().enumerate() {
            p.encode(b, 48 + 16 * i);
        }
    }
    fn new(kind: u8, owner: u32, ino: u64, now: u64) -> Inode {
        Inode { kind, flags: 0, owner, size: 0, ino, created: now, modified: now, links: 1, ptrs: [Ptr::NULL; 11] }
    }
}

/// Which tree and which leaf inside it a file block lives in: (pointer slot 0 to 10, height, leaf index).
fn file_slot(b: u64) -> (usize, u32, u64) {
    if b < DIRECT {
        (b as usize, 0, 0)
    } else if b < SINGLE_END {
        (8, 1, b - DIRECT)
    } else if b < DOUBLE_END {
        (9, 2, b - SINGLE_END)
    } else {
        (10, 3, b - DOUBLE_END)
    }
}

fn blocks_for(size: u64) -> u64 {
    size.div_ceil(BLOCK as u64)
}

// ---------------------------------------------------------------------------------------------------------------
// Folder records (spec 5.5)

/// One folder record as read: where it is and what it names.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: Vec<u8>,
    pub ino: u64,
    pub kind: u8,
}

fn rec_need(nlen: usize) -> usize {
    (12 + nlen + 3) & !3
}

/// Walks one folder block's records. Calls `f(offset, inode, kind, name, record length)`; stops at the first
/// record that breaks the rules (Damaged).
fn scan_records(blk: &[u8], mut f: impl FnMut(usize, u64, u8, &[u8], usize) -> bool) -> Result<()> {
    let mut off = 0usize;
    while off < BLOCK {
        if off + 12 > BLOCK {
            return Err(Error::Damaged);
        }
        let ino = get64(blk, off);
        let kind = blk[off + 8];
        let nlen = blk[off + 9] as usize;
        let rec = get16(blk, off + 10) as usize;
        if rec < 12 || rec % 4 != 0 || off + rec > BLOCK || 12 + nlen > rec {
            return Err(Error::Damaged);
        }
        let name = &blk[off + 12..off + 12 + nlen];
        if ino != 0 && (check_name(name).is_err() || (kind != KIND_FILE && kind != KIND_FOLDER)) {
            return Err(Error::Damaged);
        }
        if !f(off, ino, kind, name, rec) {
            return Ok(());
        }
        off += rec;
    }
    Ok(())
}

/// A name's rules (spec 5.5).
pub fn check_name(n: &[u8]) -> Result<()> {
    if n.len() > MAX_NAME {
        return Err(Error::NameTooLong);
    }
    if n.is_empty() || n == b"." || n == b".." || n.iter().any(|&c| c == b'/' || c == 0) {
        return Err(Error::Invalid);
    }
    Ok(())
}

/// A fresh folder block holding `entries` (the last record runs to the block's end).
fn folder_block(entries: &[(u64, u8, &[u8])]) -> Blk {
    let mut b = zero_block();
    let mut off = 0;
    for (n, (ino, kind, name)) in entries.iter().enumerate() {
        let mut rec = rec_need(name.len());
        if n == entries.len() - 1 {
            rec = BLOCK - off;
        }
        put64(&mut b, off, *ino);
        b[off + 8] = *kind;
        b[off + 9] = name.len() as u8;
        put16(&mut b, off + 10, rec as u16);
        b[off + 12..off + 12 + name.len()].copy_from_slice(name);
        off += rec;
    }
    if entries.is_empty() {
        put16(&mut b, 10, BLOCK as u16);
    }
    b
}

// ---------------------------------------------------------------------------------------------------------------
// Label and root record (spec 4.1, 4.2)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label {
    pub area_blocks: u64,
    pub volume_id: [u8; 16],
    pub compat: u32,
    pub ro_compat: u32,
    pub incompat: u32,
    pub fold_version: u16,
    pub migrated: bool,
    pub old_sb: [u8; 64],
}

impl Label {
    pub fn encode(&self) -> [u8; SECTOR] {
        let mut s = [0u8; SECTOR];
        s[0..8].copy_from_slice(LABEL_MAGIC);
        put16(&mut s, 8, FORMAT);
        put16(&mut s, 10, BLOCK as u16);
        put64(&mut s, 16, AREA_START);
        put64(&mut s, 24, self.area_blocks);
        s[32..48].copy_from_slice(&self.volume_id);
        put32(&mut s, 48, self.compat);
        put32(&mut s, 52, self.ro_compat);
        put32(&mut s, 56, self.incompat);
        put16(&mut s, 60, self.fold_version);
        if self.migrated {
            s[128..192].copy_from_slice(&self.old_sb);
            s[192] = 1;
        }
        let c = crc32c(&s[..508]);
        put32(&mut s, 508, c);
        s
    }
    /// Checks every rule of spec 4.1 against a partition of `sectors`.
    pub fn decode(s: &[u8], sectors: u64) -> Result<Label> {
        if s.len() < SECTOR || &s[0..8] != LABEL_MAGIC || get32(s, 508) != crc32c(&s[..508]) {
            return Err(Error::Damaged);
        }
        let area_blocks = get64(s, 24);
        if get16(s, 8) != FORMAT
            || get16(s, 10) as usize != BLOCK
            || get64(s, 16) != AREA_START
            || get32(s, 56) != 0
            || area_blocks < 8
            || area_blocks > MAX_AREA_BLOCKS
            || sectors < AREA_START
            || area_blocks > (sectors - AREA_START) / SPB
        {
            return Err(Error::Damaged);
        }
        let mut volume_id = [0u8; 16];
        volume_id.copy_from_slice(&s[32..48]);
        let mut old_sb = [0u8; 64];
        old_sb.copy_from_slice(&s[128..192]);
        Ok(Label {
            area_blocks,
            volume_id,
            compat: get32(s, 48),
            ro_compat: get32(s, 52),
            incompat: 0,
            fold_version: get16(s, 60),
            migrated: s[192] == 1,
            old_sb,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Root {
    pub gen: u64,
    pub inode_root: Ptr,
    pub map_root: Ptr,
    pub inode_height: u8,
    pub map_height: u8,
    pub used: u64,
    pub inodes: u64,
    pub next_ino: u64,
    pub time: u64,
    pub compat: u32,
    pub ro_compat: u32,
}

impl Root {
    pub fn encode(&self) -> Blk {
        let mut r = zero_block();
        r[0..8].copy_from_slice(ROOT_MAGIC);
        put64(&mut r, 8, self.gen);
        self.inode_root.encode(&mut r, 16);
        self.map_root.encode(&mut r, 32);
        r[48] = self.inode_height;
        r[49] = self.map_height;
        put64(&mut r, 56, self.used);
        put64(&mut r, 64, self.inodes);
        put64(&mut r, 72, self.next_ino);
        put64(&mut r, 80, self.time);
        put32(&mut r, 88, self.compat);
        put32(&mut r, 92, self.ro_compat);
        let c = crc32c(&r[..124]);
        put32(&mut r, 124, c);
        r
    }
    /// A valid root record (spec 4.2), or None.
    pub fn decode(r: &[u8], area_blocks: u64) -> Option<Root> {
        if &r[0..8] != ROOT_MAGIC || get32(r, 124) != crc32c(&r[..124]) || get32(r, 96) != 0 {
            return None;
        }
        let root = Root {
            gen: get64(r, 8),
            inode_root: Ptr::decode(r, 16),
            map_root: Ptr::decode(r, 32),
            inode_height: r[48],
            map_height: r[49],
            used: get64(r, 56),
            inodes: get64(r, 64),
            next_ino: get64(r, 72),
            time: get64(r, 80),
            compat: get32(r, 88),
            ro_compat: get32(r, 92),
        };
        let ok = |p: Ptr| p.blk >= 2 && p.blk < area_blocks;
        if !ok(root.inode_root)
            || !ok(root.map_root)
            || root.inode_height > 3
            || root.map_height > 2
            || root.map_height != map_height_for(area_blocks)
            || root.gen == 0
            || root.next_ino < 2
            || root.next_ino > MAX_INODE
            || root.used > area_blocks
        {
            return None;
        }
        Some(root)
    }
}

fn map_leaves(area_blocks: u64) -> u64 {
    area_blocks.div_ceil(BITS_PER_LEAF)
}

fn map_height_for(area_blocks: u64) -> u8 {
    let l = map_leaves(area_blocks);
    if l <= 1 {
        0
    } else if l <= 256 {
        1
    } else {
        2
    }
}

fn span(h: u32) -> u64 {
    1u64 << (8 * h)
}

// ---------------------------------------------------------------------------------------------------------------
// The file system

/// What a stat returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub ino: u64,
    pub kind: u8,
    pub flags: u8,
    pub owner: u32,
    pub size: u64,
    pub created: u64,
    pub modified: u64,
}

/// What `fs_info` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub generation: u64,
    pub area_blocks: u64,
    pub free_blocks: u64,
    pub inodes: u64,
    /// Inode numbers left over the volume's life (spec 5.4).
    pub inode_numbers_left: u64,
    pub read_only: bool,
}

/// What mount found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mounted {
    pub generation: u64,
    /// The newer root record was not valid and the older one was used (a cut during a commit's root record).
    pub fell_back: bool,
    /// The free map could not be read: the volume is read-only (spec 7.2).
    pub read_only: bool,
}

pub struct Fs<D: Disk> {
    dev: D,
    label: Label,
    /// The last commit.
    root: Root,
    /// The working numbers of the open transaction.
    inodes: u64,
    next_ino: u64,
    read_only: bool,
    /// A commit failed part way: the disk is whole (no root record was written for it) but RAM is not; every call
    /// answers Io until the volume is mounted again.
    poisoned: bool,
    // the open transaction
    inode_dirty: BTreeMap<u64, Blk>,
    data_dirty: BTreeMap<(u64, u64), Blk>,
    early: BTreeMap<(u64, u64), u64>,
    cut: BTreeMap<u64, u64>,
    touched: BTreeSet<u64>,
    doomed: Vec<[Ptr; 11]>,
    allocated: BTreeSet<u64>,
    freed: BTreeSet<u64>,
    pool: Vec<u64>,
    use_pool: bool,
    alloc_hint: u64,
    // committed state in RAM
    map_cache: BTreeMap<u64, Blk>,
    cache: BTreeMap<u64, Blk>,
    cache_order: VecDeque<u64>,
    /// The time stamped on changes (ns since 1970 UTC). The caller keeps it current.
    pub now_ns: u64,
    /// Dirty data blocks kept in RAM before they are written early (spec 8.3).
    pub dirty_cap: usize,
}

impl<D: Disk> Fs<D> {
    // --- format and mount -------------------------------------------------------------------------------------

    /// Formats a fresh volume (spec 5.7) and mounts it. Sectors 1 to 871 are zeroed; the label is written last.
    pub fn format(mut dev: D, volume_id: [u8; 16], now_ns: u64) -> Result<Fs<D>> {
        let sectors = dev.sectors();
        if sectors < MIN_SECTORS {
            return Err(Error::NoSpace);
        }
        let area_blocks = ((sectors - AREA_START) / SPB).min(MAX_AREA_BLOCKS);
        let label = Label {
            area_blocks,
            volume_id,
            compat: 0,
            ro_compat: 0,
            incompat: 0,
            fold_version: 0,
            migrated: false,
            old_sb: [0; 64],
        };
        Self::write_tree0(&mut dev, &label, now_ns, true)?;
        Self::mount(dev).map(|(fs, _)| fs)
    }

    /// Writes generation 1 (an empty tree with /system, /apps, /home, /tmp) and then the label. With `zero_old`,
    /// sectors 1 to 871 are zeroed first (a fresh volume); a migration (step V4) leaves them.
    fn write_tree0(dev: &mut D, label: &Label, now: u64, zero_old: bool) -> Result<()> {
        let area_blocks = label.area_blocks;
        let leaves = map_leaves(area_blocks);
        let mh = map_height_for(area_blocks);
        let gen = 1u64;
        // lowest free block first: 0 and 1 are the root records, then the inode block, the bitmap leaves, the
        // map's pointer blocks, the top folder's block
        let inode_blk = 2u64;
        let leaf0 = 3u64;
        let mut next = leaf0 + leaves;
        let mid_count = if mh == 2 { leaves.div_ceil(256) } else { 0 };
        let top_count = if mh >= 1 { 1 } else { 0 };
        let mid0 = next;
        next += mid_count;
        let top_blk = next;
        next += top_count;
        let dir_blk = next;
        next += 1;
        let used = next;
        if used >= area_blocks {
            return Err(Error::NoSpace);
        }
        let ok = |b: bool| if b { Ok(()) } else { Err(Error::Io) };
        if zero_old {
            let z = [0u8; SECTOR];
            for s in 1..OLD_AREA_END {
                ok(dev.write(s, &z))?;
            }
        }
        fn wr<D: Disk>(dev: &mut D, blk: u64, data: &[u8]) -> Result<()> {
            if dev.write(AREA_START + blk * SPB, data) {
                Ok(())
            } else {
                Err(Error::Io)
            }
        }
        // the top folder and the four below it
        let top: [(u64, &[u8], u32); 4] = [
            (2, &b"system"[..], OWNER_SYSTEM),
            (3, &b"apps"[..], OWNER_SYSTEM),
            (4, &b"home"[..], OWNER_PERSON),
            (5, &b"tmp"[..], OWNER_SYSTEM),
        ];
        let recs: Vec<(u64, u8, &[u8])> = top.iter().map(|(i, n, _)| (*i, KIND_FOLDER, *n)).collect();
        let dir = folder_block(&recs);
        let mut ib = zero_block();
        let mut rootdir = Inode::new(KIND_FOLDER, OWNER_SYSTEM, ROOT_INO, now);
        rootdir.size = BLOCK as u64;
        rootdir.ptrs[0] = Ptr::to(dir_blk, &dir, gen);
        rootdir.encode(&mut ib[256..512]);
        for (i, _, owner) in top.iter() {
            let n = Inode::new(KIND_FOLDER, *owner, *i, now);
            let o = (*i as usize) * 256;
            n.encode(&mut ib[o..o + 256]);
        }
        // the bitmap leaves
        let is_used = |b: u64| b < used || b >= area_blocks;
        let mut leaf_ptrs = Vec::new();
        for l in 0..leaves {
            let mut bm = zero_block();
            for i in 0..BITS_PER_LEAF {
                if is_used(l * BITS_PER_LEAF + i) {
                    bm[(i >> 3) as usize] |= 1 << (i & 7);
                }
            }
            wr(dev, leaf0 + l, &bm)?;
            leaf_ptrs.push(Ptr::to(leaf0 + l, &bm, gen));
        }
        let map_root = match mh {
            0 => leaf_ptrs[0],
            1 => {
                let mut pb = zero_block();
                for (i, p) in leaf_ptrs.iter().enumerate() {
                    p.encode(&mut pb, 16 * i);
                }
                wr(dev, top_blk, &pb)?;
                Ptr::to(top_blk, &pb, gen)
            }
            _ => {
                let mut tb = zero_block();
                for m in 0..mid_count {
                    let mut pb = zero_block();
                    for i in 0..256u64 {
                        let l = m * 256 + i;
                        if l < leaves {
                            leaf_ptrs[l as usize].encode(&mut pb, 16 * i as usize);
                        }
                    }
                    wr(dev, mid0 + m, &pb)?;
                    Ptr::to(mid0 + m, &pb, gen).encode(&mut tb, 16 * m as usize);
                }
                wr(dev, top_blk, &tb)?;
                Ptr::to(top_blk, &tb, gen)
            }
        };
        wr(dev, inode_blk, &ib)?;
        wr(dev, dir_blk, &dir)?;
        let root = Root {
            gen,
            inode_root: Ptr::to(inode_blk, &ib, gen),
            map_root,
            inode_height: 0,
            map_height: mh,
            used,
            inodes: 5,
            next_ino: 6,
            time: now,
            compat: 0,
            ro_compat: 0,
        };
        wr(dev, 0, &root.encode())?;
        wr(dev, 1, &zero_block())?;
        ok(dev.flush())?;
        ok(dev.write(0, &label.encode()))?;
        ok(dev.flush())
    }

    /// Mounts (spec 7.1). Never writes.
    pub fn mount(mut dev: D) -> Result<(Fs<D>, Mounted)> {
        let sectors = dev.sectors();
        let mut s0 = [0u8; SECTOR];
        if !dev.read(0, &mut s0) {
            return Err(Error::Io);
        }
        let label = Label::decode(&s0, sectors)?;
        let mut best: Option<Root> = None;
        let mut newest_gen_seen = 0u64;
        for slot in 0..2u64 {
            let mut r = zero_block();
            if !dev.read(AREA_START + slot * SPB, &mut r) {
                continue;
            }
            // the generation even of an invalid record, to tell a fall-back from a fresh volume
            if &r[0..8] == ROOT_MAGIC {
                newest_gen_seen = newest_gen_seen.max(get64(&r, 8));
            }
            if let Some(root) = Root::decode(&r, label.area_blocks) {
                if best.map_or(true, |b| root.gen > b.gen) {
                    best = Some(root);
                }
            }
        }
        let root = best.ok_or(Error::Damaged)?;
        let mut fs = Fs {
            dev,
            label,
            root,
            inodes: root.inodes,
            next_ino: root.next_ino,
            read_only: label.ro_compat != 0,
            poisoned: false,
            inode_dirty: BTreeMap::new(),
            data_dirty: BTreeMap::new(),
            early: BTreeMap::new(),
            cut: BTreeMap::new(),
            touched: BTreeSet::new(),
            doomed: Vec::new(),
            allocated: BTreeSet::new(),
            freed: BTreeSet::new(),
            pool: Vec::new(),
            use_pool: false,
            alloc_hint: 2,
            map_cache: BTreeMap::new(),
            cache: BTreeMap::new(),
            cache_order: VecDeque::new(),
            now_ns: root.time,
            dirty_cap: DEFAULT_DIRTY_CAP,
        };
        // the tops of both trees must read (spec 7.1 step 3)
        fs.read_ptr(root.inode_root)?;
        let mut read_only = fs.read_only;
        if fs.read_ptr(root.map_root).is_err() {
            read_only = true;
        }
        fs.read_only = read_only;
        // the top folder must be a folder
        let top = fs.inode(ROOT_INO)?;
        if top.kind != KIND_FOLDER || top.ino != ROOT_INO {
            return Err(Error::Damaged);
        }
        let fell_back = newest_gen_seen > root.gen;
        Ok((fs, Mounted { generation: root.gen, fell_back, read_only }))
    }

    pub fn disk(&mut self) -> &mut D {
        &mut self.dev
    }
    pub fn into_disk(self) -> D {
        self.dev
    }
    pub fn label(&self) -> &Label {
        &self.label
    }
    pub fn generation(&self) -> u64 {
        self.root.gen
    }

    // --- block reads ----------------------------------------------------------------------------------------

    fn read_raw(&mut self, blk: u64) -> Result<Blk> {
        let mut b = zero_block();
        if !self.dev.read(AREA_START + blk * SPB, &mut b) {
            return Err(Error::Io);
        }
        Ok(b)
    }

    /// A committed block, checked against its pointer (spec 5.1).
    fn read_ptr(&mut self, p: Ptr) -> Result<Blk> {
        if p.blk < 2 || p.blk >= self.label.area_blocks {
            return Err(Error::Damaged);
        }
        if let Some(b) = self.cache.get(&p.blk) {
            if crc32c(b) == p.crc {
                return Ok(b.clone());
            }
            return Err(Error::Damaged);
        }
        let b = self.read_raw(p.blk)?;
        if crc32c(&b) != p.crc {
            return Err(Error::Damaged);
        }
        if self.cache.len() >= CACHE_BLOCKS {
            if let Some(old) = self.cache_order.pop_front() {
                self.cache.remove(&old);
            }
        }
        self.cache.insert(p.blk, b.clone());
        self.cache_order.push_back(p.blk);
        Ok(b)
    }

    /// The leaf pointer at `idx` of a committed radix tree of height `h` under `root` (None: a hole).
    fn tree_get(&mut self, root: Ptr, h: u32, idx: u64) -> Result<Option<Ptr>> {
        let mut p = root;
        let mut level = h;
        while level > 0 {
            if p.is_null() {
                return Ok(None);
            }
            let b = self.read_ptr(p)?;
            let slot = ((idx >> (8 * (level - 1))) & 0xFF) as usize;
            p = Ptr::decode(&b, 16 * slot);
            level -= 1;
        }
        Ok(if p.is_null() { None } else { Some(p) })
    }

    // --- inodes ---------------------------------------------------------------------------------------------

    fn inode_leaf(&mut self, leaf: u64) -> Result<Blk> {
        if let Some(b) = self.inode_dirty.get(&leaf) {
            return Ok(b.clone());
        }
        if leaf >= span(self.root.inode_height as u32) {
            return Ok(zero_block());
        }
        match self.tree_get(self.root.inode_root, self.root.inode_height as u32, leaf)? {
            Some(p) => self.read_ptr(p),
            None => Ok(zero_block()),
        }
    }

    /// The inode as the open transaction sees it.
    pub fn inode(&mut self, ino: u64) -> Result<Inode> {
        if ino == 0 || ino >= self.next_ino {
            return Err(Error::BadHandle);
        }
        let leaf = self.inode_leaf(ino / INODES_PER_BLOCK)?;
        let o = (ino % INODES_PER_BLOCK) as usize * 256;
        let n = Inode::decode(&leaf[o..o + 256]);
        if n.kind == KIND_FREE {
            return Err(Error::BadHandle);
        }
        if n.ino != ino || (n.kind != KIND_FILE && n.kind != KIND_FOLDER) || n.size > MAX_FILE {
            return Err(Error::Damaged);
        }
        Ok(n)
    }

    fn put_inode(&mut self, ino: u64, n: &Inode) -> Result<()> {
        let leaf_idx = ino / INODES_PER_BLOCK;
        let mut leaf = self.inode_leaf(leaf_idx)?;
        let o = (ino % INODES_PER_BLOCK) as usize * 256;
        n.encode(&mut leaf[o..o + 256]);
        self.inode_dirty.insert(leaf_idx, leaf);
        Ok(())
    }

    // --- the free map ---------------------------------------------------------------------------------------

    fn map_leaf(&mut self, leaf: u64) -> Result<Blk> {
        if let Some(b) = self.map_cache.get(&leaf) {
            return Ok(b.clone());
        }
        let p = self
            .tree_get(self.root.map_root, self.root.map_height as u32, leaf)?
            .ok_or(Error::Damaged)?;
        let b = self.read_ptr(p)?;
        self.map_cache.insert(leaf, b.clone());
        Ok(b)
    }

    /// The byte of the last commit's bitmap that holds block `b`'s bit.
    fn map_byte(&mut self, b: u64) -> Result<u8> {
        let l = b / BITS_PER_LEAF;
        if !self.map_cache.contains_key(&l) {
            self.map_leaf(l)?;
        }
        let leaf = self.map_cache.get(&l).ok_or(Error::Damaged)?;
        Ok(leaf[((b % BITS_PER_LEAF) >> 3) as usize])
    }

    /// Whether the last commit uses block `b`.
    fn committed_used(&mut self, b: u64) -> Result<bool> {
        Ok(self.map_byte(b)? & (1 << (b & 7)) != 0)
    }

    /// A block free in the last commit and not yet taken by this transaction (spec 8.1 step 1).
    fn alloc(&mut self) -> Result<u64> {
        if self.use_pool {
            return self.pool.pop().ok_or(Error::NoSpace);
        }
        let n = self.label.area_blocks;
        let start = self.alloc_hint.clamp(2, n);
        let mut b = start;
        let mut wrapped = false;
        loop {
            if b >= n {
                if wrapped {
                    return Err(Error::NoSpace);
                }
                wrapped = true;
                b = 2;
            }
            if wrapped && b >= start {
                return Err(Error::NoSpace);
            }
            // skip a full byte of the bitmap at once
            if b % 8 == 0 && b + 8 <= n {
                if self.map_byte(b)? == 0xFF {
                    b += 8;
                    continue;
                }
            }
            if !self.allocated.contains(&b) && !self.committed_used(b)? {
                self.allocated.insert(b);
                self.alloc_hint = b + 1;
                return Ok(b);
            }
            b += 1;
        }
    }

    /// Marks a block no longer used by the tree being built. A block taken in this transaction is simply free
    /// again; a committed block becomes free only after this commit lands (spec 8.1 step 5).
    fn free(&mut self, b: u64) {
        if b < 2 || b >= self.label.area_blocks {
            return;
        }
        if !self.allocated.remove(&b) {
            self.freed.insert(b);
        }
    }

    fn free_blocks_now(&self) -> u64 {
        // blocks free in the last commit, less those this transaction already took or still needs
        let free_committed = self.label.area_blocks.saturating_sub(self.root.used);
        free_committed.saturating_sub(self.allocated.len() as u64)
    }

    /// Blocks a commit may need beyond the data (pointer blocks, inode blocks, bitmap blocks): a safe estimate.
    fn meta_slack(&self) -> u64 {
        let files = self.touched.len() as u64;
        let data = self.data_dirty.len() as u64 + self.early.len() as u64;
        64 + 8 * files + 2 * self.inode_dirty.len() as u64 + 4 * (data / 256 + 1) + 2 * (data / BITS_PER_LEAF + 1)
    }

    fn reserve_for(&self, owner: u32) -> u64 {
        if owner == OWNER_SYSTEM {
            0
        } else {
            // 2% of the area, at least 1 MiB (256 blocks) (spec 8.4)
            (self.label.area_blocks / 50).max(256).min(self.label.area_blocks / 4)
        }
    }

    /// Refuses with NoSpace before anything changes when `n` more blocks cannot fit (spec 8.4).
    fn need(&self, n: u64, owner: u32) -> Result<()> {
        let pending = self.data_dirty.len() as u64;
        let have = self.free_blocks_now().saturating_sub(pending + self.meta_slack() + self.reserve_for(owner));
        if n > have {
            Err(Error::NoSpace)
        } else {
            Ok(())
        }
    }

    fn writable(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Io)
        } else if self.read_only {
            Err(Error::ReadOnly)
        } else {
            Ok(())
        }
    }

    fn live(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }

    // --- file blocks ----------------------------------------------------------------------------------------

    /// File block `b` of inode `n` as the transaction sees it (zeros for a hole or past a cut).
    fn fblock(&mut self, n: &Inode, b: u64) -> Result<Blk> {
        if let Some(d) = self.data_dirty.get(&(n.ino, b)) {
            return Ok(d.clone());
        }
        if let Some(&e) = self.early.get(&(n.ino, b)) {
            return self.read_raw(e);
        }
        if let Some(&c) = self.cut.get(&n.ino) {
            if b >= c {
                return Ok(zero_block());
            }
        }
        let (slot, h, idx) = file_slot(b);
        let p = if h == 0 {
            let p = n.ptrs[slot];
            if p.is_null() {
                None
            } else {
                Some(p)
            }
        } else {
            self.tree_get(n.ptrs[slot], h, idx)?
        };
        match p {
            Some(p) => self.read_ptr(p),
            None => Ok(zero_block()),
        }
    }

    fn set_dirty(&mut self, ino: u64, b: u64, data: Blk) -> Result<()> {
        self.data_dirty.insert((ino, b), data);
        self.touched.insert(ino);
        if self.data_dirty.len() > self.dirty_cap {
            self.spill()?;
        }
        Ok(())
    }

    /// Writes the dirty data blocks early, into blocks the last commit does not use (spec 8.3).
    fn spill(&mut self) -> Result<()> {
        let keys: Vec<(u64, u64)> = self.data_dirty.keys().copied().collect();
        for k in keys {
            let data = match self.data_dirty.remove(&k) {
                Some(d) => d,
                None => continue,
            };
            let blk = match self.early.get(&k) {
                Some(&b) => b,
                None => self.alloc()?,
            };
            if !self.dev.write(AREA_START + blk * SPB, &data) {
                self.poisoned = true;
                return Err(Error::Io);
            }
            self.early.insert(k, blk);
        }
        Ok(())
    }

    /// Drops the transaction's blocks of `ino` from block `from` on.
    fn drop_blocks_from(&mut self, ino: u64, from: u64) {
        let gone: Vec<(u64, u64)> = self.data_dirty.range((ino, from)..=(ino, u64::MAX)).map(|(k, _)| *k).collect();
        for k in gone {
            self.data_dirty.remove(&k);
        }
        let gone: Vec<((u64, u64), u64)> = self.early.range((ino, from)..=(ino, u64::MAX)).map(|(k, v)| (*k, *v)).collect();
        for (k, b) in gone {
            self.early.remove(&k);
            self.free(b);
        }
    }

    // --- reading and writing files ---------------------------------------------------------------------------

    pub fn stat(&mut self, ino: u64) -> Result<Stat> {
        self.live()?;
        let n = self.inode(ino)?;
        Ok(Stat { ino, kind: n.kind, flags: n.flags, owner: n.owner, size: n.size, created: n.created, modified: n.modified })
    }

    pub fn read_at(&mut self, ino: u64, off: u64, buf: &mut [u8]) -> Result<usize> {
        self.live()?;
        let n = self.inode(ino)?;
        if n.kind == KIND_FOLDER {
            return Err(Error::IsFolder);
        }
        if off >= n.size {
            return Ok(0);
        }
        let len = (buf.len() as u64).min(n.size - off) as usize;
        let mut done = 0usize;
        while done < len {
            let pos = off + done as u64;
            let b = pos / BLOCK as u64;
            let within = (pos % BLOCK as u64) as usize;
            let take = (BLOCK - within).min(len - done);
            let blk = self.fblock(&n, b)?;
            buf[done..done + take].copy_from_slice(&blk[within..within + take]);
            done += take;
        }
        Ok(len)
    }

    pub fn write_at(&mut self, ino: u64, off: u64, data: &[u8]) -> Result<usize> {
        self.writable()?;
        let mut n = self.inode(ino)?;
        if n.kind == KIND_FOLDER {
            return Err(Error::IsFolder);
        }
        if n.flags & FLAG_READ_ONLY != 0 {
            return Err(Error::ReadOnly);
        }
        let end = off.checked_add(data.len() as u64).ok_or(Error::TooBig)?;
        if end > MAX_FILE {
            return Err(Error::TooBig);
        }
        if data.is_empty() {
            return Ok(0);
        }
        let first = off / BLOCK as u64;
        let last = (end - 1) / BLOCK as u64;
        let fresh = (first..=last).filter(|b| !self.data_dirty.contains_key(&(ino, *b))).count() as u64;
        self.need(fresh, n.owner)?;
        let mut done = 0usize;
        while done < data.len() {
            let pos = off + done as u64;
            let b = pos / BLOCK as u64;
            let within = (pos % BLOCK as u64) as usize;
            let take = (BLOCK - within).min(data.len() - done);
            let mut blk = if within == 0 && take == BLOCK { zero_block() } else { self.fblock(&n, b)? };
            blk[within..within + take].copy_from_slice(&data[done..done + take]);
            self.set_dirty(ino, b, blk)?;
            done += take;
        }
        if end > n.size {
            n.size = end;
        }
        n.modified = self.now_ns;
        self.put_inode(ino, &n)?;
        self.touched.insert(ino);
        Ok(data.len())
    }

    /// Makes a file `size` bytes long: cut (the tail of the last block zeroed) or grown with zeros.
    pub fn set_size(&mut self, ino: u64, size: u64) -> Result<()> {
        self.writable()?;
        let mut n = self.inode(ino)?;
        if n.kind == KIND_FOLDER {
            return Err(Error::IsFolder);
        }
        if n.flags & FLAG_READ_ONLY != 0 {
            return Err(Error::ReadOnly);
        }
        if size > MAX_FILE {
            return Err(Error::TooBig);
        }
        if size < n.size {
            let keep = blocks_for(size);
            if size % BLOCK as u64 != 0 {
                // the last kept block, its tail zeroed, so a later grow reads zeros there
                self.need(1, n.owner)?;
                let b = size / BLOCK as u64;
                let mut blk = self.fblock(&n, b)?;
                for x in blk[(size % BLOCK as u64) as usize..].iter_mut() {
                    *x = 0;
                }
                self.drop_blocks_from(ino, keep);
                let c = self.cut.get(&ino).copied().unwrap_or(u64::MAX).min(keep);
                self.cut.insert(ino, c);
                self.set_dirty(ino, b, blk)?;
            } else {
                self.drop_blocks_from(ino, keep);
                let c = self.cut.get(&ino).copied().unwrap_or(u64::MAX).min(keep);
                self.cut.insert(ino, c);
            }
        }
        n.size = size;
        n.modified = self.now_ns;
        self.put_inode(ino, &n)?;
        self.touched.insert(ino);
        Ok(())
    }

    pub fn set_times(&mut self, ino: u64, modified: u64) -> Result<()> {
        self.writable()?;
        let mut n = self.inode(ino)?;
        n.modified = modified;
        self.put_inode(ino, &n)
    }

    /// Sets the read-only, hidden and executable flags (the others are the file system's own).
    pub fn set_flags(&mut self, ino: u64, flags: u8) -> Result<()> {
        self.writable()?;
        let mut n = self.inode(ino)?;
        let user = FLAG_READ_ONLY | FLAG_HIDDEN | FLAG_EXEC;
        n.flags = (n.flags & !user) | (flags & user);
        self.put_inode(ino, &n)
    }

    /// Replaces a file's whole contents in the transaction (the shell's `> name`, Nia's `file_write`).
    pub fn write_all(&mut self, ino: u64, data: &[u8]) -> Result<()> {
        self.set_size(ino, 0)?;
        self.write_at(ino, 0, data)?;
        Ok(())
    }

    // --- folders --------------------------------------------------------------------------------------------

    fn folder(&mut self, dir: u64) -> Result<Inode> {
        let d = self.inode(dir)?;
        if d.kind != KIND_FOLDER {
            return Err(Error::NotFolder);
        }
        Ok(d)
    }

    /// Finds `name` in folder `dir`: (folder block, offset, inode, kind).
    fn find(&mut self, d: &Inode, name: &[u8]) -> Result<Option<(u64, usize, u64, u8)>> {
        let nb = blocks_for(d.size);
        for b in 0..nb {
            let blk = self.fblock(d, b)?;
            let mut hit = None;
            scan_records(&blk, |off, ino, kind, nm, _| {
                if ino != 0 && nm == name {
                    hit = Some((b, off, ino, kind));
                    false
                } else {
                    true
                }
            })?;
            if hit.is_some() {
                return Ok(hit);
            }
        }
        Ok(None)
    }

    pub fn lookup(&mut self, dir: u64, name: &[u8]) -> Result<u64> {
        self.live()?;
        check_name(name)?;
        let d = self.folder(dir)?;
        match self.find(&d, name)? {
            Some((_, _, ino, _)) => Ok(ino),
            None => Err(Error::NotFound),
        }
    }

    /// Walks `path` from folder `dir`, one part at a time (spec 11.2): no leading `/`, no empty part, no `.` or
    /// `..`, at most 4,096 bytes and 64 folders. The result is always at or under `dir`.
    pub fn walk(&mut self, dir: u64, path: &[u8]) -> Result<u64> {
        self.live()?;
        if path.len() > MAX_PATH {
            return Err(Error::NameTooLong);
        }
        if path.is_empty() || path[0] == b'/' {
            return Err(Error::Invalid);
        }
        let mut cur = dir;
        let mut depth = 0usize;
        for part in path.split(|&c| c == b'/') {
            check_name(part)?;
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(Error::NameTooLong);
            }
            cur = self.lookup(cur, part)?;
        }
        Ok(cur)
    }

    pub fn list(&mut self, dir: u64) -> Result<Vec<Entry>> {
        self.live()?;
        let d = self.folder(dir)?;
        let mut out = Vec::new();
        for b in 0..blocks_for(d.size) {
            let blk = self.fblock(&d, b)?;
            scan_records(&blk, |_, ino, kind, nm, _| {
                if ino != 0 {
                    out.push(Entry { name: nm.to_vec(), ino, kind });
                }
                true
            })?;
        }
        Ok(out)
    }

    fn is_empty_folder(&mut self, ino: u64) -> Result<bool> {
        let d = self.inode(ino)?;
        for b in 0..blocks_for(d.size) {
            let blk = self.fblock(&d, b)?;
            let mut any = false;
            scan_records(&blk, |_, i, _, _, _| {
                if i != 0 {
                    any = true;
                }
                !any
            })?;
            if any {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Adds a record to folder `dir` (it must not hold `name`).
    fn insert(&mut self, dir: u64, name: &[u8], ino: u64, kind: u8) -> Result<()> {
        let mut d = self.folder(dir)?;
        let need = rec_need(name.len());
        let nb = blocks_for(d.size);
        for b in 0..nb {
            let mut blk = self.fblock(&d, b)?;
            let mut place: Option<(usize, usize, bool)> = None; // offset, record length, split a used record
            scan_records(&blk, |off, i, _, nm, rec| {
                if i == 0 && rec >= need {
                    place = Some((off, rec, false));
                    false
                } else if i != 0 && rec - rec_need(nm.len()) >= need {
                    place = Some((off, rec, true));
                    false
                } else {
                    true
                }
            })?;
            if let Some((off, rec, split)) = place {
                let (at, len) = if split {
                    let used = rec_need(blk[off + 9] as usize);
                    put16(&mut blk, off + 10, used as u16);
                    (off + used, rec - used)
                } else {
                    (off, rec)
                };
                put64(&mut blk, at, ino);
                blk[at + 8] = kind;
                blk[at + 9] = name.len() as u8;
                put16(&mut blk, at + 10, len as u16);
                blk[at + 12..at + 12 + name.len()].copy_from_slice(name);
                self.set_dirty(dir, b, blk)?;
                d.modified = self.now_ns;
                return self.put_inode(dir, &d);
            }
        }
        // a new block for the folder
        if (nb + 1) * BLOCK as u64 > MAX_FILE {
            return Err(Error::NoSpace);
        }
        let blk = folder_block(&[(ino, kind, name)]);
        self.set_dirty(dir, nb, blk)?;
        d.size = (nb + 1) * BLOCK as u64;
        d.modified = self.now_ns;
        self.put_inode(dir, &d)
    }

    /// Removes the record for `name` from folder `dir`.
    fn unlink_record(&mut self, dir: u64, b: u64, off: usize) -> Result<()> {
        let mut d = self.folder(dir)?;
        let mut blk = self.fblock(&d, b)?;
        // the record before it in the same block takes its space; the block's first record becomes unused
        let mut prev: Option<usize> = None;
        scan_records(&blk, |o, _, _, _, _| {
            if o == off {
                false
            } else {
                prev = Some(o);
                true
            }
        })?;
        let rec = get16(&blk, off + 10) as usize;
        match prev {
            Some(p) => {
                let prec = get16(&blk, p + 10) as usize;
                put16(&mut blk, p + 10, (prec + rec) as u16);
                for x in blk[off..off + rec].iter_mut() {
                    *x = 0;
                }
            }
            None => {
                for x in blk[off..off + rec].iter_mut() {
                    *x = 0;
                }
                put16(&mut blk, off + 10, rec as u16);
            }
        }
        self.set_dirty(dir, b, blk)?;
        d.modified = self.now_ns;
        self.put_inode(dir, &d)
    }

    /// Makes `name` in folder `dir`. With `exclusive`, an existing name is EXISTS; without it, an existing file
    /// of the same kind is returned.
    pub fn create(&mut self, dir: u64, name: &[u8], kind: u8, exclusive: bool) -> Result<u64> {
        self.writable()?;
        check_name(name)?;
        if kind != KIND_FILE && kind != KIND_FOLDER {
            return Err(Error::Invalid);
        }
        let d = self.folder(dir)?;
        if let Some((_, _, ino, k)) = self.find(&d, name)? {
            if exclusive {
                return Err(Error::Exists);
            }
            if k != kind {
                return Err(if k == KIND_FOLDER { Error::IsFolder } else { Error::NotFolder });
            }
            return Ok(ino);
        }
        if self.next_ino >= MAX_INODE {
            return Err(Error::NoSpace);
        }
        self.need(2, d.owner)?;
        let ino = self.next_ino;
        self.next_ino += 1;
        let n = Inode::new(kind, d.owner, ino, self.now_ns);
        self.put_inode(ino, &n)?;
        self.inodes += 1;
        self.insert(dir, name, ino, kind)?;
        Ok(ino)
    }

    /// Frees inode `ino` and, at the commit, every block its committed tree uses.
    fn doom(&mut self, ino: u64) -> Result<()> {
        let n = self.inode(ino)?;
        self.drop_blocks_from(ino, 0);
        self.cut.remove(&ino);
        self.touched.remove(&ino);
        self.doomed.push(n.ptrs);
        self.put_inode(ino, &Inode::default())?;
        self.inodes = self.inodes.saturating_sub(1);
        Ok(())
    }

    /// Removes `name` from folder `dir` (a folder only when empty).
    pub fn remove(&mut self, dir: u64, name: &[u8]) -> Result<()> {
        self.writable()?;
        check_name(name)?;
        let d = self.folder(dir)?;
        let (b, off, ino, kind) = self.find(&d, name)?.ok_or(Error::NotFound)?;
        if kind == KIND_FOLDER && !self.is_empty_folder(ino)? {
            return Err(Error::NotEmpty);
        }
        self.unlink_record(dir, b, off)?;
        self.doom(ino)
    }

    /// Whether folder `inner` is `outer` or lies under it.
    fn is_under(&mut self, inner: u64, outer: u64) -> Result<bool> {
        if inner == outer {
            return Ok(true);
        }
        let mut stack = vec![(outer, 0usize)];
        while let Some((f, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                continue;
            }
            for e in self.list(f)? {
                if e.kind == KIND_FOLDER {
                    if e.ino == inner {
                        return Ok(true);
                    }
                    stack.push((e.ino, depth + 1));
                }
            }
        }
        Ok(false)
    }

    /// Renames `sname` in `sdir` to `dname` in `ddir`, replacing a file (or an empty folder) already there. One
    /// commit holds the whole change, so after a cut the old name or the new one names the file (spec 8.2).
    pub fn rename(&mut self, sdir: u64, sname: &[u8], ddir: u64, dname: &[u8]) -> Result<()> {
        self.writable()?;
        check_name(sname)?;
        check_name(dname)?;
        let sd = self.folder(sdir)?;
        let dd = self.folder(ddir)?;
        let (_, _, sino, skind) = self.find(&sd, sname)?.ok_or(Error::NotFound)?;
        if sdir == ddir && sname == dname {
            return Ok(());
        }
        if skind == KIND_FOLDER && self.is_under(ddir, sino)? {
            return Err(Error::Invalid);
        }
        self.need(2, dd.owner)?;
        if let Some((_, _, dino, dkind)) = self.find(&dd, dname)? {
            if dino == sino {
                return Ok(());
            }
            if skind == KIND_FOLDER && dkind != KIND_FOLDER {
                return Err(Error::NotFolder);
            }
            if skind != KIND_FOLDER && dkind == KIND_FOLDER {
                return Err(Error::IsFolder);
            }
            if dkind == KIND_FOLDER && !self.is_empty_folder(dino)? {
                return Err(Error::NotEmpty);
            }
            let dd = self.folder(ddir)?;
            let (b, off, _, _) = self.find(&dd, dname)?.ok_or(Error::Damaged)?;
            self.unlink_record(ddir, b, off)?;
            self.doom(dino)?;
        }
        let sd = self.folder(sdir)?;
        let (b, off, _, _) = self.find(&sd, sname)?.ok_or(Error::Damaged)?;
        self.unlink_record(sdir, b, off)?;
        self.insert(ddir, dname, sino, skind)
    }

    // --- information ----------------------------------------------------------------------------------------

    pub fn fs_info(&self) -> Info {
        Info {
            generation: self.root.gen,
            area_blocks: self.label.area_blocks,
            free_blocks: self.free_blocks_now().saturating_sub(self.data_dirty.len() as u64),
            inodes: self.inodes,
            inode_numbers_left: MAX_INODE - self.next_ino,
            read_only: self.read_only,
        }
    }

    /// Whether the open transaction holds any change.
    pub fn dirty(&self) -> bool {
        !self.inode_dirty.is_empty()
            || !self.data_dirty.is_empty()
            || !self.early.is_empty()
            || !self.touched.is_empty()
            || !self.doomed.is_empty()
    }

    // --- the commit (spec 8.1) -------------------------------------------------------------------------------

    /// Writes a block the commit made, then returns its pointer.
    fn put_new(&mut self, data: &[u8], gen: u64) -> Result<Ptr> {
        let blk = self.alloc()?;
        if !self.dev.write(AREA_START + blk * SPB, data) {
            return Err(Error::Io);
        }
        Ok(Ptr::to(blk, data, gen))
    }

    /// Frees a committed subtree (a removed file's, or a cut file's tail). A block that no longer reads is left
    /// (vfscheck reports it as leaked); the commit goes on.
    fn free_subtree(&mut self, p: Ptr, h: u32) {
        if p.is_null() {
            return;
        }
        if h > 0 {
            if let Ok(b) = self.read_ptr(p) {
                for i in 0..PTRS {
                    let c = Ptr::decode(&b, 16 * i);
                    self.free_subtree(c, h - 1);
                }
            }
        }
        self.free(p.blk);
    }

    /// Rewrites a radix tree of height `h` under `p`, covering leaves `base` to `base + 256^h - 1`: `changes`
    /// (sorted, inside that range) set leaf pointers (None: a hole), and every leaf at or past `cut` that is not
    /// changed is dropped. Changed pointer blocks go to new blocks and the old ones are freed.
    fn update_tree(&mut self, p: Ptr, h: u32, base: u64, changes: &[(u64, Option<Ptr>)], cut: u64, gen: u64) -> Result<Ptr> {
        if h == 0 {
            let new = match changes.first() {
                Some(c) => c.1.unwrap_or(Ptr::NULL),
                None if base >= cut => Ptr::NULL,
                None => return Ok(p),
            };
            if !p.is_null() && p.blk != new.blk {
                self.free(p.blk);
            }
            return Ok(new);
        }
        let end = base.saturating_add(span(h));
        if changes.is_empty() && end <= cut {
            return Ok(p);
        }
        if changes.is_empty() && base >= cut {
            self.free_subtree(p, h);
            return Ok(Ptr::NULL);
        }
        if changes.is_empty() && p.is_null() {
            return Ok(Ptr::NULL);
        }
        let mut blk = if p.is_null() { zero_block() } else { self.read_ptr(p)? };
        let sub = span(h - 1);
        let mut i = 0usize;
        for slot in 0..PTRS {
            let lo = base + slot as u64 * sub;
            let hi = lo + sub;
            let start = i;
            while i < changes.len() && changes[i].0 < hi {
                i += 1;
            }
            let part = &changes[start..i];
            let child = Ptr::decode(&blk, 16 * slot);
            if part.is_empty() && (hi <= cut || child.is_null()) {
                continue;
            }
            let nc = self.update_tree(child, h - 1, lo, part, cut, gen)?;
            nc.encode(&mut blk, 16 * slot);
        }
        if !p.is_null() {
            self.free(p.blk);
        }
        if blk.iter().all(|&x| x == 0) {
            return Ok(Ptr::NULL);
        }
        self.put_new(&blk, gen)
    }

    /// The new pointers of one file or folder: its data blocks written, its trees rewritten.
    fn commit_file(&mut self, ino: u64, gen: u64) -> Result<()> {
        let mut n = match self.inode(ino) {
            Ok(n) => n,
            Err(Error::BadHandle) => return Ok(()), // removed in this transaction
            Err(e) => return Err(e),
        };
        let mut changes: BTreeMap<u64, Option<Ptr>> = BTreeMap::new();
        let dirty: Vec<u64> = self.data_dirty.range((ino, 0)..=(ino, u64::MAX)).map(|(k, _)| k.1).collect();
        for b in dirty {
            let data = self.data_dirty.remove(&(ino, b)).ok_or(Error::Io)?;
            let p = match self.early.remove(&(ino, b)) {
                Some(blk) => {
                    if !self.dev.write(AREA_START + blk * SPB, &data) {
                        return Err(Error::Io);
                    }
                    Ptr::to(blk, &data, gen)
                }
                None => self.put_new(&data, gen)?,
            };
            changes.insert(b, Some(p));
        }
        let early: Vec<((u64, u64), u64)> = self.early.range((ino, 0)..=(ino, u64::MAX)).map(|(k, v)| (*k, *v)).collect();
        for ((_, b), blk) in early {
            self.early.remove(&(ino, b));
            let data = self.read_raw(blk)?;
            changes.insert(b, Some(Ptr::to(blk, &data, gen)));
        }
        let cut = self.cut.remove(&ino).unwrap_or(u64::MAX);
        // the direct pointers
        for s in 0..DIRECT {
            let new = match changes.get(&s) {
                Some(c) => c.unwrap_or(Ptr::NULL),
                None if s >= cut => Ptr::NULL,
                None => continue,
            };
            let old = n.ptrs[s as usize];
            if !old.is_null() && old.blk != new.blk {
                self.free(old.blk);
            }
            n.ptrs[s as usize] = new;
        }
        // the single, double and triple trees
        for (slot, h, lo, hi) in [(8usize, 1u32, DIRECT, SINGLE_END), (9, 2, SINGLE_END, DOUBLE_END), (10, 3, DOUBLE_END, u64::MAX)] {
            let part: Vec<(u64, Option<Ptr>)> = changes.range(lo..hi).map(|(k, v)| (k - lo, *v)).collect();
            let rel_cut = if cut <= lo { 0 } else { cut - lo };
            if part.is_empty() && (cut >= hi || n.ptrs[slot].is_null()) {
                continue;
            }
            n.ptrs[slot] = self.update_tree(n.ptrs[slot], h, 0, &part, rel_cut, gen)?;
        }
        self.put_inode(ino, &n)
    }

    /// Commits the open transaction (spec 8.1). On success every change is on the medium.
    pub fn sync(&mut self) -> Result<()> {
        self.writable()?;
        if !self.dirty() {
            return Ok(());
        }
        let r = self.commit();
        if r.is_err() {
            self.poisoned = true;
        }
        r
    }

    fn commit(&mut self) -> Result<()> {
        let gen = self.root.gen + 1;
        // removed files: their committed trees are freed
        let doomed = core::mem::take(&mut self.doomed);
        for ptrs in doomed.iter() {
            for (s, p) in ptrs.iter().enumerate() {
                let h = if s < 8 { 0 } else { s as u32 - 7 };
                self.free_subtree(*p, h);
            }
        }
        // files and folders
        let touched: Vec<u64> = self.touched.iter().copied().collect();
        for ino in touched {
            self.commit_file(ino, gen)?;
        }
        self.touched.clear();
        // the inode tree: grow it to hold the highest inode, then write the changed inode blocks
        let mut ih = self.root.inode_height as u32;
        let mut iroot = self.root.inode_root;
        let top_leaf = (self.next_ino.saturating_sub(1)) / INODES_PER_BLOCK;
        while top_leaf >= span(ih) {
            if ih >= 3 {
                return Err(Error::NoSpace);
            }
            let mut pb = zero_block();
            iroot.encode(&mut pb, 0);
            iroot = self.put_new(&pb, gen)?;
            ih += 1;
        }
        let leaves: Vec<u64> = self.inode_dirty.keys().copied().collect();
        let mut ichanges = Vec::new();
        for leaf in leaves {
            let data = self.inode_dirty.remove(&leaf).ok_or(Error::Io)?;
            let p = self.put_new(&data, gen)?;
            ichanges.push((leaf, Some(p)));
        }
        if ih == 0 {
            if let Some((_, Some(p))) = ichanges.first() {
                if !iroot.is_null() && iroot.blk != p.blk {
                    self.free(iroot.blk);
                }
                iroot = *p;
            }
        } else if !ichanges.is_empty() {
            iroot = self.update_tree(iroot, ih, 0, &ichanges, u64::MAX, gen)?;
        }
        // the free map, which must also record its own new blocks: settle which leaves change and how many
        // blocks that takes, then take those blocks, then write
        let mh = self.root.map_height as u32;
        let leaves_total = map_leaves(self.label.area_blocks);
        let mut prepared: BTreeSet<u64> = BTreeSet::new();
        loop {
            let mut touched_leaves: BTreeSet<u64> = BTreeSet::new();
            for b in self.allocated.iter().chain(self.freed.iter()) {
                touched_leaves.insert(b / BITS_PER_LEAF);
            }
            for b in self.pool.iter() {
                touched_leaves.insert(b / BITS_PER_LEAF);
            }
            // the old blocks of those leaves and of the pointer blocks above them are freed by the rewrite
            for &l in touched_leaves.iter() {
                if prepared.contains(&l) {
                    continue;
                }
                prepared.insert(l);
                let mut p = self.root.map_root;
                let mut level = mh;
                while level > 0 {
                    self.free(p.blk);
                    let b = self.read_ptr(p)?;
                    let slot = ((l >> (8 * (level - 1))) & 0xFF) as usize;
                    p = Ptr::decode(&b, 16 * slot);
                    level -= 1;
                }
                self.free(p.blk);
            }
            let mids: BTreeSet<u64> = prepared.iter().map(|l| l / 256).collect();
            let ptr_blocks: u64 = match mh {
                0 => 0,
                1 => 1,
                _ => 1 + mids.len() as u64,
            };
            let need = prepared.len() as u64 + ptr_blocks;
            let mut grew = false;
            while (self.pool.len() as u64) < need {
                let b = self.alloc()?;
                self.pool.push(b);
                if !prepared.contains(&(b / BITS_PER_LEAF)) {
                    grew = true;
                }
            }
            let more = self.freed.iter().chain(self.allocated.iter()).any(|b| !prepared.contains(&(b / BITS_PER_LEAF)));
            if !grew && !more {
                // the pool may hold more than needed: give the rest back
                while (self.pool.len() as u64) > need {
                    if let Some(b) = self.pool.pop() {
                        self.allocated.remove(&b);
                    }
                }
                break;
            }
        }
        // new leaf contents: the last commit's bits, with this commit's blocks set and its freed blocks cleared
        let mut new_leaves: BTreeMap<u64, Blk> = BTreeMap::new();
        for &l in prepared.iter() {
            if l < leaves_total {
                new_leaves.insert(l, self.map_leaf(l)?);
            }
        }
        let set: Vec<u64> = self.allocated.iter().copied().collect();
        for b in set {
            if let Some(leaf) = new_leaves.get_mut(&(b / BITS_PER_LEAF)) {
                let i = b % BITS_PER_LEAF;
                leaf[(i >> 3) as usize] |= 1 << (i & 7);
            }
        }
        let clear: Vec<u64> = self.freed.iter().copied().collect();
        for b in clear {
            if let Some(leaf) = new_leaves.get_mut(&(b / BITS_PER_LEAF)) {
                let i = b % BITS_PER_LEAF;
                leaf[(i >> 3) as usize] &= !(1 << (i & 7));
            }
        }
        self.use_pool = true;
        let mut mchanges = Vec::new();
        let mut result = Ok(());
        for (l, data) in new_leaves.iter() {
            match self.put_new(data, gen) {
                Ok(p) => mchanges.push((*l, Some(p))),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        let mroot = if result.is_ok() && !mchanges.is_empty() {
            if mh == 0 {
                mchanges[0].1.unwrap_or(Ptr::NULL)
            } else {
                // the map's old pointer blocks were already freed above; update_tree frees them again (no change)
                match self.update_tree(self.root.map_root, mh, 0, &mchanges, u64::MAX, gen) {
                    Ok(p) => p,
                    Err(e) => {
                        result = Err(e);
                        Ptr::NULL
                    }
                }
            }
        } else {
            self.root.map_root
        };
        self.use_pool = false;
        result?;
        if !self.pool.is_empty() {
            // every pool block was counted for the map; one left means the count was wrong
            return Err(Error::Io);
        }
        // step 2: flush; step 3: the root record into the other slot; step 4: flush
        if !self.dev.flush() {
            return Err(Error::Io);
        }
        let used = (self.root.used + self.allocated.len() as u64).saturating_sub(self.freed.len() as u64);
        let root = Root {
            gen,
            inode_root: iroot,
            map_root: mroot,
            inode_height: ih as u8,
            map_height: mh as u8,
            used,
            inodes: self.inodes,
            next_ino: self.next_ino,
            time: self.now_ns,
            compat: self.label.compat,
            ro_compat: self.label.ro_compat,
        };
        let slot = (gen + 1) % 2;
        if !self.dev.write(AREA_START + slot * SPB, &root.encode()) || !self.dev.flush() {
            return Err(Error::Io);
        }
        // step 5: the new commit is the tree now
        self.root = root;
        for (l, data) in new_leaves {
            self.map_cache.insert(l, data);
        }
        self.allocated.clear();
        self.freed.clear();
        self.cache.clear();
        self.cache_order.clear();
        self.alloc_hint = 2;
        Ok(())
    }
}
