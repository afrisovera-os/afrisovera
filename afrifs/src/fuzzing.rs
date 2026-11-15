//! A plain model of the file system, operation scripts made from bytes, and the fuzz entry points
//! (fuzz-host/fuzz/fuzz_targets/afrifs_*.rs call these; the tests run them on seeds and random inputs).
//!
//! Every entry point panics only when the file system is wrong: a panic inside the file system itself, a mount
//! that accepts what it must refuse, a walk that leaves its folder, or a state that differs from the model.

use crate::ramdisk::RamDisk;
use crate::*;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

/// The fixed time the tests format with (2025-11-14 00:00 UTC, as the vectors).
pub const T0: u64 = 1_763_078_400 * 1_000_000_000;
/// The 8 MiB test volume.
pub const TEST_SECTORS: u64 = 16384;

// ---------------------------------------------------------------------------------------------------------------
// The model

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    File(Vec<u8>),
    Folder(BTreeMap<Vec<u8>, Node>),
}

impl Node {
    pub fn fresh() -> Node {
        let mut top = BTreeMap::new();
        for n in [&b"system"[..], &b"apps"[..], &b"home"[..], &b"tmp"[..]] {
            top.insert(n.to_vec(), Node::Folder(BTreeMap::new()));
        }
        Node::Folder(top)
    }

    fn get(&self, path: &[Vec<u8>]) -> Option<&Node> {
        let mut n = self;
        for p in path {
            match n {
                Node::Folder(m) => n = m.get(p)?,
                Node::File(_) => return None,
            }
        }
        Some(n)
    }

    fn get_mut(&mut self, path: &[Vec<u8>]) -> Option<&mut Node> {
        let mut n = self;
        for p in path {
            match n {
                Node::Folder(m) => n = m.get_mut(p)?,
                Node::File(_) => return None,
            }
        }
        Some(n)
    }

    /// Every folder's path, in order (the top folder is the empty path).
    pub fn folders(&self) -> Vec<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        fn go(n: &Node, path: &mut Vec<Vec<u8>>, out: &mut Vec<Vec<Vec<u8>>>) {
            if let Node::Folder(m) = n {
                out.push(path.clone());
                for (k, v) in m {
                    path.push(k.clone());
                    go(v, path, out);
                    path.pop();
                }
            }
        }
        go(self, &mut Vec::new(), &mut out);
        out
    }

    /// Every file's path, in order.
    pub fn files(&self) -> Vec<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        fn go(n: &Node, path: &mut Vec<Vec<u8>>, out: &mut Vec<Vec<Vec<u8>>>) {
            match n {
                Node::File(_) => out.push(path.clone()),
                Node::Folder(m) => {
                    for (k, v) in m {
                        path.push(k.clone());
                        go(v, path, out);
                        path.pop();
                    }
                }
            }
        }
        go(self, &mut Vec::new(), &mut out);
        out
    }

    fn children(&self, path: &[Vec<u8>]) -> Vec<Vec<u8>> {
        match self.get(path) {
            Some(Node::Folder(m)) => m.keys().cloned().collect(),
            _ => Vec::new(),
        }
    }
}

/// The whole tree as the file system shows it (contents and kinds only).
pub fn dump<D: Disk>(fs: &mut Fs<D>) -> Result<Node> {
    fn go<D: Disk>(fs: &mut Fs<D>, ino: u64, depth: usize) -> Result<Node> {
        let st = fs.stat(ino)?;
        if st.kind == KIND_FILE {
            if st.size > 64 << 20 {
                return Err(Error::TooBig);
            }
            let mut data = vec![0u8; st.size as usize];
            let n = fs.read_at(ino, 0, &mut data)?;
            if n != data.len() {
                return Err(Error::Damaged);
            }
            return Ok(Node::File(data));
        }
        if depth > MAX_DEPTH + 1 {
            return Err(Error::Damaged);
        }
        let mut m = BTreeMap::new();
        for e in fs.list(ino)? {
            let child = go(fs, e.ino, depth + 1)?;
            if m.insert(e.name, child).is_some() {
                return Err(Error::Damaged);
            }
        }
        Ok(Node::Folder(m))
    }
    go(fs, ROOT_INO, 0)
}

fn resolve<D: Disk>(fs: &mut Fs<D>, path: &[Vec<u8>]) -> Result<u64> {
    let mut cur = ROOT_INO;
    for p in path {
        cur = fs.lookup(cur, p)?;
    }
    Ok(cur)
}

// ---------------------------------------------------------------------------------------------------------------
// Operations

#[derive(Clone, Debug)]
pub enum Op {
    Create { dir: Vec<Vec<u8>>, name: Vec<u8>, folder: bool, exclusive: bool },
    Write { file: Vec<Vec<u8>>, off: u64, len: usize, seed: u8 },
    SetSize { file: Vec<Vec<u8>>, size: u64 },
    Remove { dir: Vec<Vec<u8>>, name: Vec<u8> },
    Rename { sdir: Vec<Vec<u8>>, sname: Vec<u8>, ddir: Vec<Vec<u8>>, dname: Vec<u8> },
    Sync,
}

/// A byte reader for making operations from fuzz input or a seeded generator.
pub struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Bytes<'a> {
        Bytes { data, at: 0 }
    }
    pub fn done(&self) -> bool {
        self.at >= self.data.len()
    }
    pub fn u8(&mut self) -> u8 {
        let v = self.data.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        v
    }
    pub fn u16(&mut self) -> u16 {
        u16::from_le_bytes([self.u8(), self.u8()])
    }
}

const NAMES: [&[u8]; 6] = [b"a", b"b", b"notes.txt", b"index", b"index.lock", b"D"];

/// The next operation, chosen from what the model holds now. `big` allows writes past the dirty cap.
pub fn next_op(b: &mut Bytes, model: &Node, big: bool) -> Op {
    let folders = model.folders();
    let files = model.files();
    let pick_name = |x: u8| NAMES[x as usize % NAMES.len()].to_vec();
    match b.u8() % 8 {
        0 | 1 => Op::Create {
            dir: folders[b.u8() as usize % folders.len()].clone(),
            name: pick_name(b.u8()),
            folder: b.u8() % 4 == 0,
            exclusive: b.u8() % 2 == 0,
        },
        2 | 3 if !files.is_empty() => {
            let file = files[b.u8() as usize % files.len()].clone();
            let off = (b.u16() as u64) % 20_000;
            let len = if big && b.u8() % 8 == 0 { 600_000 + b.u16() as usize } else { b.u16() as usize % 9000 };
            Op::Write { file, off, len, seed: b.u8() }
        }
        4 if !files.is_empty() => {
            let file = files[b.u8() as usize % files.len()].clone();
            Op::SetSize { file, size: (b.u16() as u64) % 30_000 }
        }
        5 => {
            let dir = folders[b.u8() as usize % folders.len()].clone();
            let kids = model.children(&dir);
            let name = if kids.is_empty() { pick_name(b.u8()) } else { kids[b.u8() as usize % kids.len()].clone() };
            Op::Remove { dir, name }
        }
        6 => {
            let sdir = folders[b.u8() as usize % folders.len()].clone();
            let kids = model.children(&sdir);
            let sname = if kids.is_empty() { pick_name(b.u8()) } else { kids[b.u8() as usize % kids.len()].clone() };
            let ddir = folders[b.u8() as usize % folders.len()].clone();
            Op::Rename { sdir, sname, ddir, dname: pick_name(b.u8()) }
        }
        _ => Op::Sync,
    }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed) ^ ((i >> 9) as u8)).collect()
}

fn is_prefix(a: &[Vec<u8>], b: &[Vec<u8>]) -> bool {
    a.len() <= b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
}

/// Follows `path` from the top as the file system's lookups do: a missing name is NotFound, a name under a file is
/// NotFolder.
fn model_at<'a>(m: &'a Node, path: &[Vec<u8>]) -> core::result::Result<&'a Node, Error> {
    let mut n = m;
    for p in path {
        match n {
            Node::Folder(d) => n = d.get(p).ok_or(Error::NotFound)?,
            Node::File(_) => return Err(Error::NotFolder),
        }
    }
    Ok(n)
}

fn model_folder<'a>(m: &'a Node, path: &[Vec<u8>]) -> core::result::Result<&'a BTreeMap<Vec<u8>, Node>, Error> {
    match model_at(m, path)? {
        Node::Folder(d) => Ok(d),
        Node::File(_) => Err(Error::NotFolder),
    }
}

fn model_folder_mut<'a>(m: &'a mut Node, path: &[Vec<u8>]) -> &'a mut BTreeMap<Vec<u8>, Node> {
    match m.get_mut(path) {
        Some(Node::Folder(d)) => d,
        _ => unreachable!("checked by model_folder first"),
    }
}

/// What the model says the operation does: the new model, or the error the file system must give (checked in the
/// file system's own order).
pub fn model_apply(model: &Node, op: &Op) -> core::result::Result<Node, Error> {
    let mut m = model.clone();
    match op {
        Op::Create { dir, name, folder, exclusive } => {
            let d = model_folder(&m, dir)?;
            match d.get(name) {
                Some(existing) => {
                    if *exclusive {
                        return Err(Error::Exists);
                    }
                    match (existing, folder) {
                        (Node::Folder(_), false) => return Err(Error::IsFolder),
                        (Node::File(_), true) => return Err(Error::NotFolder),
                        _ => {}
                    }
                }
                None => {
                    let fresh = if *folder { Node::Folder(BTreeMap::new()) } else { Node::File(Vec::new()) };
                    model_folder_mut(&mut m, dir).insert(name.clone(), fresh);
                }
            }
        }
        Op::Write { file, off, len, seed } => {
            if let Node::Folder(_) = model_at(&m, file)? {
                return Err(Error::IsFolder);
            }
            if let Some(Node::File(data)) = m.get_mut(file) {
                if *len > 0 {
                    let end = *off as usize + len;
                    if data.len() < end {
                        data.resize(end, 0);
                    }
                    data[*off as usize..end].copy_from_slice(&pattern(*len, *seed));
                }
            }
        }
        Op::SetSize { file, size } => {
            if let Node::Folder(_) = model_at(&m, file)? {
                return Err(Error::IsFolder);
            }
            if let Some(Node::File(data)) = m.get_mut(file) {
                data.resize(*size as usize, 0);
            }
        }
        Op::Remove { dir, name } => {
            let d = model_folder(&m, dir)?;
            match d.get(name) {
                None => return Err(Error::NotFound),
                Some(Node::Folder(k)) if !k.is_empty() => return Err(Error::NotEmpty),
                Some(_) => {
                    model_folder_mut(&mut m, dir).remove(name);
                }
            }
        }
        Op::Rename { sdir, sname, ddir, dname } => {
            // the file system resolves both folders, then checks that both are folders, then finds the source
            model_at(&m, sdir)?;
            model_at(&m, ddir)?;
            let src = model_folder(&m, sdir)?.get(sname).cloned();
            let dd = model_folder(&m, ddir)?;
            let src = src.ok_or(Error::NotFound)?;
            if sdir == ddir && sname == dname {
                return Ok(m);
            }
            let mut moved = sdir.clone();
            moved.push(sname.clone());
            if matches!(src, Node::Folder(_)) && is_prefix(&moved, ddir) {
                return Err(Error::Invalid);
            }
            if let Some(dst) = dd.get(dname) {
                match (&src, dst) {
                    (Node::Folder(_), Node::File(_)) => return Err(Error::NotFolder),
                    (Node::File(_), Node::Folder(_)) => return Err(Error::IsFolder),
                    (_, Node::Folder(k)) if !k.is_empty() => return Err(Error::NotEmpty),
                    _ => {}
                }
            }
            model_folder_mut(&mut m, sdir).remove(sname);
            model_folder_mut(&mut m, ddir).insert(dname.clone(), src);
        }
        Op::Sync => {}
    }
    Ok(m)
}

/// Does `op` on the file system. Its result must match the model's, except that NoSpace is allowed (the model has
/// no idea of space) and must change nothing.
pub fn fs_apply<D: Disk>(fs: &mut Fs<D>, op: &Op) -> Result<()> {
    match op {
        Op::Create { dir, name, folder, exclusive } => {
            let d = resolve(fs, dir)?;
            fs.create(d, name, if *folder { KIND_FOLDER } else { KIND_FILE }, *exclusive).map(|_| ())
        }
        Op::Write { file, off, len, seed } => {
            let f = resolve(fs, file)?;
            fs.write_at(f, *off, &pattern(*len, *seed)).map(|_| ())
        }
        Op::SetSize { file, size } => {
            let f = resolve(fs, file)?;
            fs.set_size(f, *size)
        }
        Op::Remove { dir, name } => {
            let d = resolve(fs, dir)?;
            fs.remove(d, name)
        }
        Op::Rename { sdir, sname, ddir, dname } => {
            let s = resolve(fs, sdir)?;
            let d = resolve(fs, ddir)?;
            fs.rename(s, sname, d, dname)
        }
        Op::Sync => fs.sync(),
    }
}

/// Runs `ops` against a fresh 8 MiB volume and the model, checking each result and, after each sync, the whole
/// tree and `vfscheck`. Returns the disk and the model after each successful sync (index 0: after format).
pub fn run_script(disk: RamDisk, ops: &[Op], dirty_cap: usize) -> (Fs<RamDisk>, Vec<Node>) {
    let mut fs = Fs::format(disk, [7; 16], T0).expect("format");
    fs.dirty_cap = dirty_cap;
    let mut model = Node::fresh();
    let mut synced = vec![model.clone()];
    for (i, op) in ops.iter().enumerate() {
        fs.now_ns = T0 + i as u64;
        let want = model_apply(&model, op);
        let got = fs_apply(&mut fs, op);
        match (&want, &got) {
            (Ok(m), Ok(())) => model = m.clone(),
            (Err(w), Err(g)) if w == g => {}
            (_, Err(Error::NoSpace)) => {}
            _ => panic!("op {} {:?}: model {:?}, file system {:?}", i, op, want.as_ref().err(), got),
        }
        if matches!(op, Op::Sync) {
            let seen = dump(&mut fs).expect("dump after sync");
            assert!(seen == model, "op {}: the tree differs from the model after sync", i);
            let r = fs.check();
            assert!(r.clean(), "op {}: vfscheck: {:?} leaked {}", i, r.problems, r.leaked);
            synced.push(model.clone());
        }
    }
    (fs, synced)
}

// ---------------------------------------------------------------------------------------------------------------
// Fuzz entry points

/// A populated, committed test image: a few folders and files, one of them past the direct pointers.
pub fn sample_image() -> RamDisk {
    let mut fs = Fs::format(RamDisk::new(TEST_SECTORS), [9; 16], T0).expect("format");
    let home = fs.lookup(ROOT_INO, b"home").expect("home");
    let docs = fs.create(home, b"docs", KIND_FOLDER, true).expect("docs");
    let a = fs.create(docs, b"a.txt", KIND_FILE, true).expect("a");
    fs.write_at(a, 0, b"hello").expect("write a");
    let big = fs.create(home, b"big.bin", KIND_FILE, true).expect("big");
    fs.write_at(big, 0, &pattern(300 * 1024, 3)).expect("write big");
    fs.sync().expect("sync");
    let apps = fs.lookup(ROOT_INO, b"apps").expect("apps");
    let x = fs.create(apps, b"x", KIND_FOLDER, true).expect("x");
    let c = fs.create(x, b"c", KIND_FILE, true).expect("c");
    fs.write_at(c, 0, &pattern(5000, 1)).expect("write c");
    fs.sync().expect("sync 2");
    fs.into_disk()
}

/// Reads what a mounted volume will give, within a byte budget, then writes and commits once. A refusal is fine;
/// a panic is a finding.
fn exercise(fs: &mut Fs<RamDisk>) {
    let _ = fs.check();
    let _ = fs.fs_info();
    let mut budget: u64 = 4 << 20;
    let mut stack = vec![(ROOT_INO, 0usize)];
    let mut seen = BTreeSet::new();
    let mut buf = vec![0u8; 65536];
    while let Some((ino, depth)) = stack.pop() {
        if !seen.insert(ino) || depth > MAX_DEPTH || budget == 0 {
            continue;
        }
        let Ok(list) = fs.list(ino) else { continue };
        for e in list {
            if e.kind == KIND_FOLDER {
                stack.push((e.ino, depth + 1));
                continue;
            }
            let mut off = 0u64;
            while budget > 0 {
                match fs.read_at(e.ino, off, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        off += n as u64;
                        budget = budget.saturating_sub(n as u64);
                    }
                }
            }
        }
    }
    if let Ok(home) = fs.lookup(ROOT_INO, b"home") {
        if let Ok(f) = fs.create(home, b"fuzz", KIND_FILE, false) {
            let _ = fs.write_at(f, 0, b"x");
            let _ = fs.sync();
        }
    }
}

/// Target `afrifs_image`: any bytes as a whole image (sector 0, then area blocks 0, 1, 2 and so on).
pub fn fuzz_image(data: &[u8]) {
    let mut d = RamDisk::new(TEST_SECTORS);
    let mut chunks = Vec::new();
    if !data.is_empty() {
        let mut s = [0u8; SECTOR];
        let n = data.len().min(SECTOR);
        s[..n].copy_from_slice(&data[..n]);
        chunks.push((0u64, s.to_vec()));
        let mut blk = 0u64;
        let mut at = n;
        while at < data.len() && blk < 64 {
            let mut b = vec![0u8; BLOCK];
            let k = (data.len() - at).min(BLOCK);
            b[..k].copy_from_slice(&data[at..at + k]);
            chunks.push((AREA_START + blk * (BLOCK / SECTOR) as u64, b));
            at += k;
            blk += 1;
        }
    }
    for (s, b) in chunks {
        let _ = d.write(s, &b);
    }
    if let Ok((mut fs, _)) = Fs::mount(d) {
        exercise(&mut fs);
    }
}

/// Target `afrifs_meta`: edits to a valid image's structures. Each edit is 4 bytes: which structure, an offset
/// (u16) and a value; the structure is the label, root record A or B, or one of the image's first 40 blocks.
pub fn fuzz_meta(data: &[u8]) {
    let mut d = sample_image();
    let mut b = Bytes::new(data);
    let mut edits = 0;
    while !b.done() && edits < 64 {
        edits += 1;
        let which = b.u8() % 42;
        let off = b.u16() as u64;
        let val = b.u8();
        let (sector, within) = if which == 0 {
            (0, off % SECTOR as u64)
        } else {
            let blk = (which - 1) as u64;
            let o = off % BLOCK as u64;
            (AREA_START + blk * (BLOCK / SECTOR) as u64 + o / SECTOR as u64, o % SECTOR as u64)
        };
        let mut s = d.sector(sector);
        s[within as usize] = val;
        d.set_sector(sector, s);
    }
    match Fs::mount(d) {
        Ok((mut fs, _)) => exercise(&mut fs),
        Err(_) => {}
    }
}

/// Target `afrifs_walk`: a path from a folder handle must end at or under that folder, or be refused.
pub fn fuzz_walk(data: &[u8]) {
    let (mut fs, _) = Fs::mount(sample_image()).expect("sample mounts");
    let starts = [ROOT_INO, 2, 3, 4];
    let start = starts[data.first().copied().unwrap_or(0) as usize % starts.len()];
    let path: Vec<u8> = data.iter().skip(1).copied().collect();
    // also the edge lengths: 4,096 bytes and one more
    let mut tries = vec![path.clone()];
    let mut long = Vec::new();
    while long.len() < MAX_PATH {
        let part: &[u8] = if long.is_empty() { &b"docs"[..] } else { &b"/docs"[..] };
        long.extend_from_slice(part);
    }
    long.truncate(MAX_PATH);
    tries.push(long.clone());
    long.push(b'x');
    tries.push(long);
    // the folders under `start` (and itself)
    let mut under: BTreeSet<u64> = BTreeSet::new();
    let mut stack = vec![start];
    while let Some(f) = stack.pop() {
        if !under.insert(f) {
            continue;
        }
        if let Ok(list) = fs.list(f) {
            for e in list {
                if e.kind == KIND_FOLDER {
                    stack.push(e.ino);
                } else {
                    under.insert(e.ino);
                }
            }
        }
    }
    for p in tries {
        match fs.walk(start, &p) {
            Ok(ino) => assert!(under.contains(&ino), "walk left its folder: {:?}", p),
            Err(e) => {
                if p.len() > MAX_PATH {
                    assert_eq!(e, Error::NameTooLong);
                }
            }
        }
    }
}

/// Target `afrifs_ops`: an operation script against the model, with a remount at the end.
pub fn fuzz_ops(data: &[u8]) {
    let mut b = Bytes::new(data);
    let mut model = Node::fresh();
    let mut ops = Vec::new();
    while !b.done() && ops.len() < 200 {
        let op = next_op(&mut b, &model, false);
        if let Ok(m) = model_apply(&model, &op) {
            model = m;
        }
        ops.push(op);
    }
    ops.push(Op::Sync);
    let (fs, synced) = run_script(RamDisk::new(TEST_SECTORS), &ops, 16);
    let disk = fs.into_disk();
    let (mut again, _) = Fs::mount(disk).expect("remount after the script");
    let seen = dump(&mut again).expect("dump after remount");
    assert!(Some(&seen) == synced.last(), "the remounted tree differs from the last sync");
}
