//! The crash harness (VFS_V1_PLAN.md 7.1, spec 14): a script runs once on a disk that logs every write and flush.
//! Then, at every write, the disks a power cut could leave are rebuilt:
//! - everything up to that write (the writes since the last flush may all have landed);
//! - the last write torn: only its first 1 to 7 sectors new (a disk with 512-byte sectors tearing a 4 KiB block);
//! - just before each flush, seeded subsets of the writes since the previous flush (a disk may reorder them), some
//!   with one write torn as well.
//! Each must mount, pass vfscheck, and hold exactly the tree of the commit before the cut or the commit in flight.
mod common;

use afrifs::fuzzing::*;
use afrifs::ramdisk::{Event, RamDisk};
use afrifs::*;
use common::{env_num, Rng};
use std::collections::BTreeMap;

type Overlay = BTreeMap<u64, [u8; SECTOR]>;

fn apply(o: &mut Overlay, sector: u64, data: &[u8], sectors: usize) {
    for i in 0..sectors.min(data.len() / SECTOR) {
        let mut s = [0u8; SECTOR];
        s.copy_from_slice(&data[i * SECTOR..(i + 1) * SECTOR]);
        o.insert(sector + i as u64, s);
    }
}

struct Run {
    base: RamDisk,
    log: Vec<Event>,
    /// For each event: how many syncs had completed before it.
    epoch: Vec<usize>,
    /// The tree after format (0) and after each sync.
    states: Vec<Node>,
}

fn record(ops: &[Op], dirty_cap: usize) -> Run {
    let mut fs = Fs::format(RamDisk::new(TEST_SECTORS), [7; 16], T0).expect("format");
    fs.dirty_cap = dirty_cap;
    let base = fs.disk().clone();
    fs.disk().record = true;
    let mut model = Node::fresh();
    let mut states = vec![model.clone()];
    let mut epoch = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        fs.now_ns = T0 + i as u64;
        let want = model_apply(&model, op);
        let before = fs.disk().log.len();
        let got = fs_apply(&mut fs, op);
        let done = states.len() - 1;
        for _ in before..fs.disk().log.len() {
            epoch.push(done);
        }
        match (&want, &got) {
            (Ok(m), Ok(())) => model = m.clone(),
            (Err(w), Err(g)) if w == g => {}
            (_, Err(Error::NoSpace)) => {}
            _ => panic!("op {} {:?}: model {:?}, file system {:?}", i, op, want.as_ref().err(), got),
        }
        if matches!(op, Op::Sync) && got.is_ok() {
            states.push(model.clone());
        }
    }
    let log = fs.disk().log.clone();
    Run { base, log, epoch, states }
}

struct Stats {
    disks: u64,
    fell_back: u64,
}

/// Mounts one possible disk and checks it against the commits it may hold.
fn judge(run: &Run, o: &Overlay, ep: usize, what: &str, st: &mut Stats) {
    st.disks += 1;
    let d = run.base.with_sectors(o);
    let (mut fs, m) = Fs::mount(d).unwrap_or_else(|e| panic!("{}: mount {:?}", what, e));
    if m.fell_back {
        st.fell_back += 1;
    }
    let r = fs.check();
    assert!(r.clean(), "{}: vfscheck {:?} leaked {}", what, r.problems, r.leaked);
    let seen = dump(&mut fs).unwrap_or_else(|e| panic!("{}: dump {:?}", what, e));
    let old = &run.states[ep];
    let new = run.states.get(ep + 1);
    assert!(seen == *old || Some(&seen) == new, "{}: the tree is neither the commit before the cut nor the next", what);
}

fn crash_check(name: &str, ops: &[Op], dirty_cap: usize, reorders: usize, seed: u64) -> Stats {
    let run = record(ops, dirty_cap);
    let mut st = Stats { disks: 0, fell_back: 0 };
    let mut rng = Rng(seed);
    let mut durable: Overlay = Overlay::new();
    let mut pending: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut now: Overlay = Overlay::new(); // durable plus every pending write, in order
    for (i, ev) in run.log.iter().enumerate() {
        let ep = run.epoch[i];
        match ev {
            Event::Write { sector, data } => {
                // a torn write: only its first k sectors new
                let n = data.len() / SECTOR;
                for k in 1..n {
                    let mut o = now.clone();
                    apply(&mut o, *sector, data, k);
                    judge(&run, &o, ep, &format!("{} event {} torn at {} sectors", name, i, k), &mut st);
                }
                apply(&mut now, *sector, data, n);
                pending.push((*sector, data.clone()));
                judge(&run, &now, ep, &format!("{} after event {}", name, i), &mut st);
            }
            Event::Flush => {
                // just before the flush: the writes since the last one, in any subset
                for r in 0..reorders {
                    let mut o = durable.clone();
                    let torn = r % 2 == 1;
                    for (sector, data) in pending.iter() {
                        if rng.below(2) == 0 {
                            continue;
                        }
                        let n = data.len() / SECTOR;
                        let k = if torn && n > 1 && rng.below(4) == 0 { 1 + rng.below(n as u64 - 1) as usize } else { n };
                        apply(&mut o, *sector, data, k);
                    }
                    judge(&run, &o, ep, &format!("{} before flush {} reorder {}", name, i, r), &mut st);
                }
                durable = now.clone();
                pending.clear();
            }
        }
    }
    println!("[afrifs crash] {}: {} events, {} disks mounted and judged, {} fell back to the older root", name, run.log.len(), st.disks, st.fell_back);
    st
}

fn p(parts: &[&str]) -> Vec<Vec<u8>> {
    parts.iter().map(|s| s.as_bytes().to_vec()).collect()
}

fn w(f: &[&str], off: u64, len: usize, seed: u8) -> Op {
    Op::Write { file: p(f), off, len, seed }
}

#[test]
fn crash_small_writes_overwrite_and_rename() {
    let ops = vec![
        Op::Create { dir: p(&["home"]), name: b"index".to_vec(), folder: false, exclusive: true },
        w(&["home", "index"], 0, 6000, 1),
        Op::Sync,
        w(&["home", "index"], 2000, 3000, 2),
        Op::Sync,
        Op::Create { dir: p(&["home"]), name: b"index.lock".to_vec(), folder: false, exclusive: true },
        w(&["home", "index.lock"], 0, 9000, 3),
        Op::Rename { sdir: p(&["home"]), sname: b"index.lock".to_vec(), ddir: p(&["home"]), dname: b"index".to_vec() },
        Op::Sync,
    ];
    crash_check("lock-and-rename", &ops, DEFAULT_DIRTY_CAP, 64, 1);
}

#[test]
fn crash_folders_and_removes() {
    let ops = vec![
        Op::Create { dir: p(&["home"]), name: b"a".to_vec(), folder: true, exclusive: true },
        Op::Create { dir: p(&["home", "a"]), name: b"b".to_vec(), folder: false, exclusive: true },
        w(&["home", "a", "b"], 0, 100, 1),
        Op::Sync,
        Op::Rename { sdir: p(&["home"]), sname: b"a".to_vec(), ddir: p(&["tmp"]), dname: b"D".to_vec() },
        Op::Sync,
        Op::Remove { dir: p(&["tmp", "D"]), name: b"b".to_vec() },
        Op::Remove { dir: p(&["tmp"]), name: b"D".to_vec() },
        Op::Sync,
    ];
    crash_check("folders", &ops, DEFAULT_DIRTY_CAP, 64, 2);
}

#[test]
fn crash_a_save_past_the_dirty_cap() {
    // 2 MiB with a cap of 32 blocks: most data is written early, before the commit (spec 8.3)
    let ops = vec![
        Op::Create { dir: p(&["home"]), name: b"big".to_vec(), folder: false, exclusive: true },
        w(&["home", "big"], 0, 300_000, 1),
        Op::Sync,
        w(&["home", "big"], 0, 2 << 20, 2),
        Op::Sync,
        Op::SetSize { file: p(&["home", "big"]), size: 10_000 },
        Op::Sync,
    ];
    crash_check("past-the-cap", &ops, 32, 32, 3);
}

#[test]
fn crash_cuts_and_growth() {
    let ops = vec![
        Op::Create { dir: p(&["apps"]), name: b"f".to_vec(), folder: false, exclusive: true },
        w(&["apps", "f"], 0, 1_200_000, 1),
        Op::Sync,
        Op::SetSize { file: p(&["apps", "f"]), size: 5000 },
        w(&["apps", "f"], 300_000, 10, 2),
        Op::Sync,
    ];
    crash_check("cuts", &ops, DEFAULT_DIRTY_CAP, 32, 4);
}

#[test]
fn crash_a_torn_root_record() {
    // the architect's case: a 4 KiB root-record write torn into sectors on a 512-byte disk. The record lives in the
    // block's first sector, so a tear either carries the whole record or none of it.
    let ops = vec![
        Op::Create { dir: p(&["home"]), name: b"f".to_vec(), folder: false, exclusive: true },
        w(&["home", "f"], 0, 100, 1),
        Op::Sync,
        w(&["home", "f"], 0, 100, 2),
        Op::Sync,
    ];
    let run = record(&ops, DEFAULT_DIRTY_CAP);
    let roots: Vec<usize> = run
        .log
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match e {
            Event::Write { sector, .. } if *sector == AREA_START || *sector == AREA_START + 8 => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(roots.len(), 2, "one root record per commit");
    for &ri in roots.iter() {
        let mut o = Overlay::new();
        for e in run.log[..ri].iter() {
            if let Event::Write { sector, data } = e {
                apply(&mut o, *sector, data, data.len() / SECTOR);
            }
        }
        let Event::Write { sector, data } = &run.log[ri] else { unreachable!() };
        for k in 0..=8 {
            let mut t = o.clone();
            apply(&mut t, *sector, data, k);
            let (_, m) = Fs::mount(run.base.with_sectors(&t)).expect("mount");
            let ep = run.epoch[ri];
            let expect_gen = if k == 0 { ep as u64 + 1 } else { ep as u64 + 2 };
            assert_eq!(m.generation, expect_gen, "root write {} torn at {} sectors", ri, k);
        }
    }
}

#[test]
fn crash_random_scripts() {
    let n = env_num("AFRIFS_CRASH_SCRIPTS", 12);
    let reorders = env_num("AFRIFS_REORDERS", 16) as usize;
    let mut rng = Rng(0xc4a5_0011);
    let mut total = 0;
    for s in 0..n {
        let bytes = rng.bytes(400);
        let mut b = Bytes::new(&bytes);
        let mut model = Node::fresh();
        let mut ops = Vec::new();
        while !b.done() && ops.len() < 40 {
            let op = next_op(&mut b, &model, false);
            if let Ok(m) = model_apply(&model, &op) {
                model = m;
            }
            ops.push(op);
        }
        ops.push(Op::Sync);
        let cap = if s % 2 == 0 { DEFAULT_DIRTY_CAP } else { 6 };
        total += crash_check(&format!("random {}", s), &ops, cap, reorders, 100 + s).disks;
    }
    println!("[afrifs crash] random scripts: {} disks in all", total);
}
