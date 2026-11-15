//! The file system against the model: fixed scripts, seeded random scripts, the rules of names and folders, space,
//! big files, many names, inode-tree growth, a refused flush, and the fuzz entry points on random inputs.
mod common;

use afrifs::fuzzing::*;
use afrifs::ramdisk::RamDisk;
use afrifs::*;
use common::{env_num, Rng};

fn p(parts: &[&str]) -> Vec<Vec<u8>> {
    parts.iter().map(|s| s.as_bytes().to_vec()).collect()
}

fn fresh() -> Fs<RamDisk> {
    Fs::format(RamDisk::new(TEST_SECTORS), [3; 16], T0).expect("format")
}

fn remount(fs: Fs<RamDisk>) -> Fs<RamDisk> {
    Fs::mount(fs.into_disk()).expect("mount").0
}

#[test]
fn fixed_scripts_match_the_model() {
    let w = |f: &[&str], off: u64, len: usize, seed: u8| Op::Write { file: p(f), off, len, seed };
    let scripts: Vec<Vec<Op>> = vec![
        // git's lock and rename, twice
        vec![
            Op::Create { dir: p(&["home"]), name: b"index".to_vec(), folder: false, exclusive: true },
            w(&["home", "index"], 0, 3000, 1),
            Op::Sync,
            Op::Create { dir: p(&["home"]), name: b"index.lock".to_vec(), folder: false, exclusive: true },
            w(&["home", "index.lock"], 0, 5000, 2),
            Op::Rename { sdir: p(&["home"]), sname: b"index.lock".to_vec(), ddir: p(&["home"]), dname: b"index".to_vec() },
            Op::Sync,
            Op::Create { dir: p(&["home"]), name: b"index.lock".to_vec(), folder: false, exclusive: true },
            Op::Create { dir: p(&["home"]), name: b"index.lock".to_vec(), folder: false, exclusive: true },
            w(&["home", "index.lock"], 0, 10, 3),
            Op::Rename { sdir: p(&["home"]), sname: b"index.lock".to_vec(), ddir: p(&["home"]), dname: b"index".to_vec() },
            Op::Sync,
        ],
        // folders: make, fill, refuse to remove, move, move into itself, empty and remove
        vec![
            Op::Create { dir: p(&["home"]), name: b"a".to_vec(), folder: true, exclusive: true },
            Op::Create { dir: p(&["home", "a"]), name: b"b".to_vec(), folder: true, exclusive: true },
            Op::Create { dir: p(&["home", "a", "b"]), name: b"notes.txt".to_vec(), folder: false, exclusive: false },
            w(&["home", "a", "b", "notes.txt"], 100, 50, 4),
            Op::Remove { dir: p(&["home"]), name: b"a".to_vec() },
            Op::Rename { sdir: p(&["home"]), sname: b"a".to_vec(), ddir: p(&["home", "a", "b"]), dname: b"D".to_vec() },
            Op::Rename { sdir: p(&["home"]), sname: b"a".to_vec(), ddir: p(&["tmp"]), dname: b"D".to_vec() },
            Op::Sync,
            Op::Remove { dir: p(&["tmp", "D", "b"]), name: b"notes.txt".to_vec() },
            Op::Remove { dir: p(&["tmp", "D"]), name: b"b".to_vec() },
            Op::Remove { dir: p(&["tmp"]), name: b"D".to_vec() },
            Op::Sync,
        ],
        // sizes: grow, cut mid-block, grow again (zeros), cut to nothing, past the direct pointers
        vec![
            Op::Create { dir: p(&["apps"]), name: b"b".to_vec(), folder: false, exclusive: true },
            w(&["apps", "b"], 0, 9000, 5),
            Op::SetSize { file: p(&["apps", "b"]), size: 5000 },
            Op::SetSize { file: p(&["apps", "b"]), size: 12000 },
            Op::Sync,
            Op::SetSize { file: p(&["apps", "b"]), size: 4000 },
            Op::Sync,
            Op::SetSize { file: p(&["apps", "b"]), size: 0 },
            w(&["apps", "b"], 40000, 3000, 6),
            Op::Sync,
            w(&["apps", "b"], 0, 600_000, 7),
            Op::Sync,
            Op::SetSize { file: p(&["apps", "b"]), size: 70_000 },
            Op::Sync,
        ],
        // the errors
        vec![
            Op::Create { dir: p(&["home"]), name: b"a".to_vec(), folder: false, exclusive: true },
            Op::Create { dir: p(&["home"]), name: b"a".to_vec(), folder: true, exclusive: false },
            Op::Create { dir: p(&["home", "a"]), name: b"b".to_vec(), folder: false, exclusive: false },
            Op::Write { file: p(&["home"]), off: 0, len: 5, seed: 0 },
            Op::Remove { dir: p(&["home"]), name: b"nothing".to_vec() },
            Op::Create { dir: p(&["home"]), name: b"D".to_vec(), folder: true, exclusive: true },
            Op::Rename { sdir: p(&["home"]), sname: b"a".to_vec(), ddir: p(&["home"]), dname: b"D".to_vec() },
            Op::Rename { sdir: p(&["home"]), sname: b"D".to_vec(), ddir: p(&["home"]), dname: b"a".to_vec() },
            Op::Rename { sdir: p(&["home"]), sname: b"a".to_vec(), ddir: p(&["home"]), dname: b"a".to_vec() },
            Op::Sync,
        ],
    ];
    for (i, s) in scripts.iter().enumerate() {
        for cap in [DEFAULT_DIRTY_CAP, 4] {
            let (fs, synced) = run_script(RamDisk::new(TEST_SECTORS), s, cap);
            let mut again = remount(fs);
            assert_eq!(Some(&dump(&mut again).unwrap()), synced.last(), "script {} cap {}", i, cap);
            assert!(again.check().clean(), "script {}: {:?}", i, again.check().problems);
        }
    }
}

#[test]
fn random_scripts_match_the_model() {
    let n = env_num("AFRIFS_SCRIPTS", 60);
    let mut rng = Rng(0x5eed_0011);
    for i in 0..n {
        let bytes = rng.bytes(1500);
        let mut b = Bytes::new(&bytes);
        let mut model = Node::fresh();
        let mut ops = Vec::new();
        while !b.done() && ops.len() < 120 {
            let op = next_op(&mut b, &model, i % 5 == 0);
            if let Ok(m) = model_apply(&model, &op) {
                model = m;
            }
            ops.push(op);
        }
        ops.push(Op::Sync);
        let cap = if i % 2 == 0 { DEFAULT_DIRTY_CAP } else { 8 };
        let (fs, synced) = run_script(RamDisk::new(TEST_SECTORS), &ops, cap);
        let mut again = remount(fs);
        assert_eq!(Some(&dump(&mut again).unwrap()), synced.last(), "script {}", i);
    }
}

#[test]
fn names_and_walks() {
    let mut fs = fresh();
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    let long = vec![b'n'; 255];
    assert!(fs.create(home, &long, KIND_FILE, true).is_ok());
    assert_eq!(fs.create(home, &vec![b'n'; 256], KIND_FILE, true).err(), Some(Error::NameTooLong));
    let bad_names: [&[u8]; 5] = [b"", b".", b"..", b"a/b", b"a\0b"];
    for bad in bad_names {
        assert_eq!(fs.create(home, bad, KIND_FILE, true).err(), Some(Error::Invalid), "{:?}", bad);
    }
    let a = fs.create(home, b"a", KIND_FOLDER, true).unwrap();
    let f = fs.create(a, b"f", KIND_FILE, true).unwrap();
    assert_eq!(fs.walk(home, b"a/f").unwrap(), f);
    let bad_paths: [&[u8]; 6] = [b"/a", b"a//f", b"a/../a", b"./a", b"a/", b""];
    for bad in bad_paths {
        assert_eq!(fs.walk(home, bad).err(), Some(Error::Invalid), "{:?}", bad);
    }
    assert_eq!(fs.walk(a, b"f/x").err(), Some(Error::NotFolder));
    assert_eq!(fs.walk(home, &vec![b'a'; MAX_PATH + 1]).err(), Some(Error::NameTooLong));
    // names are bytes, case-sensitive
    assert_eq!(fs.lookup(home, b"A").err(), Some(Error::NotFound));
    assert!(fs.create(home, b"A", KIND_FILE, true).is_ok());
    // a folder cannot move under itself
    let b = fs.create(a, b"b", KIND_FOLDER, true).unwrap();
    assert_eq!(fs.rename(home, b"a", b, b"x").err(), Some(Error::Invalid));
    assert_eq!(fs.rename(home, b"a", a, b"x").err(), Some(Error::Invalid));
    fs.sync().unwrap();
    assert!(fs.check().clean());
}

#[test]
fn inode_numbers_are_never_reused_and_the_inode_tree_grows() {
    let mut fs = fresh();
    let tmp = fs.lookup(ROOT_INO, b"tmp").unwrap();
    let mut last = 0;
    // 5,000 creates: the inode tree grows from height 0 to height 2
    for i in 0..5000u32 {
        let name = format!("f{}", i);
        let ino = fs.create(tmp, name.as_bytes(), KIND_FILE, true).unwrap();
        assert!(ino > last);
        last = ino;
        if i % 3 == 0 {
            fs.remove(tmp, name.as_bytes()).unwrap();
        }
        if i % 1000 == 999 {
            fs.sync().unwrap();
        }
    }
    fs.sync().unwrap();
    let left = fs.fs_info().inode_numbers_left;
    assert_eq!(left, MAX_INODE - (last + 1));
    let mut fs = remount(fs);
    assert!(fs.check().clean(), "{:?}", fs.check().problems);
    let again = fs.create(tmp, b"again", KIND_FILE, true).unwrap();
    assert_eq!(again, last + 1);
    assert_eq!(fs.list(tmp).unwrap().len(), 5000 - 1667 + 1);
}

#[test]
fn ten_thousand_names_in_one_folder() {
    let mut fs = Fs::format(RamDisk::new(1 << 17), [4; 16], T0).unwrap(); // 64 MiB
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    for i in 0..10_000u32 {
        fs.create(home, format!("object-{:05}", i).as_bytes(), KIND_FILE, true).unwrap();
    }
    fs.sync().unwrap();
    let mut fs = remount(fs);
    let t = std::time::Instant::now();
    for i in (0..10_000u32).step_by(97) {
        fs.lookup(home, format!("object-{:05}", i).as_bytes()).unwrap();
    }
    let per = t.elapsed() / (10_000 / 97 + 1);
    println!("[afrifs] lookup in a folder of 10,000 names: {:?} each (host)", per);
    assert_eq!(fs.list(home).unwrap().len(), 10_000);
    assert!(fs.check().clean());
}

fn big_file_round_trip(mib: u64) {
    let sectors = (mib + 16) * 2048 + AREA_START;
    let mut fs = Fs::format(RamDisk::new(sectors), [5; 16], T0).unwrap();
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    let f = fs.create(home, b"big.bin", KIND_FILE, true).unwrap();
    let chunk: Vec<u8> = (0..1 << 20).map(|i: u32| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
    for k in 0..mib {
        let mut c = chunk.clone();
        c[0] = k as u8;
        fs.write_at(f, k << 20, &c).unwrap();
    }
    fs.sync().unwrap();
    let mut fs = remount(fs);
    assert_eq!(fs.stat(f).unwrap().size, mib << 20);
    let mut buf = vec![0u8; 1 << 20];
    for k in 0..mib {
        assert_eq!(fs.read_at(f, k << 20, &mut buf).unwrap(), 1 << 20);
        assert_eq!(buf[0], k as u8);
        assert_eq!(&buf[1..], &chunk[1..], "MiB {}", k);
    }
    assert!(fs.check().clean(), "{:?}", fs.check().problems);
    // replace it whole, smaller: the old blocks come back
    let before = fs.fs_info().free_blocks;
    fs.write_all(f, b"small now").unwrap();
    fs.sync().unwrap();
    assert!(fs.fs_info().free_blocks > before + (mib << 8) - 64);
    assert!(fs.check().clean());
}

#[test]
fn a_16_mib_file() {
    big_file_round_trip(16);
}

#[test]
fn a_256_mib_file_when_asked() {
    // the cloud job sets AFRIFS_BIG=1 (about 300 MB of RAM for the sparse disk)
    if env_num("AFRIFS_BIG", 0) == 1 {
        big_file_round_trip(256);
    }
}

#[test]
fn no_space_changes_nothing() {
    let mut fs = fresh();
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    let f = fs.create(home, b"f", KIND_FILE, true).unwrap();
    fs.write_at(f, 0, b"keep me").unwrap();
    fs.sync().unwrap();
    // far more than the 8 MiB volume holds
    assert_eq!(fs.write_at(f, 0, &vec![1u8; 9 << 20]).err(), Some(Error::NoSpace));
    let mut buf = [0u8; 7];
    fs.read_at(f, 0, &mut buf).unwrap();
    assert_eq!(&buf, b"keep me");
    // fill what the person may use; the system's reserve is still there for its own files
    let g = fs.create(home, b"g", KIND_FILE, true).unwrap();
    let mut wrote = 0u64;
    while fs.write_at(g, wrote, &vec![2u8; 65536]).is_ok() {
        wrote += 65536;
        if wrote % (1 << 20) == 0 {
            fs.sync().unwrap();
        }
    }
    fs.sync().unwrap();
    let sys = fs.lookup(ROOT_INO, b"system").unwrap();
    let conf = fs.create(sys, b"afrisovera.conf", KIND_FILE, true).unwrap();
    fs.write_at(conf, 0, &vec![3u8; 100_000]).unwrap();
    fs.sync().unwrap();
    assert!(fs.check().clean(), "{:?}", fs.check().problems);
    let mut fs = remount(fs);
    assert_eq!(fs.stat(g).unwrap().size, wrote);
    assert_eq!(fs.stat(conf).unwrap().size, 100_000);
}

#[test]
fn a_refused_flush_changes_nothing() {
    let mut fs = fresh();
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    let f = fs.create(home, b"f", KIND_FILE, true).unwrap();
    fs.write_at(f, 0, b"one").unwrap();
    fs.sync().unwrap();
    fs.write_all(f, b"two").unwrap();
    fs.disk().fail_flush = true;
    assert_eq!(fs.sync().err(), Some(Error::Io));
    // the file system now refuses until it is mounted again
    assert_eq!(fs.stat(f).err(), Some(Error::Io));
    let mut d = fs.into_disk();
    d.fail_flush = false;
    let (mut fs, m) = Fs::mount(d).unwrap();
    assert_eq!(m.generation, 2);
    let mut buf = [0u8; 3];
    fs.read_at(f, 0, &mut buf).unwrap();
    assert_eq!(&buf, b"one");
    assert!(fs.check().clean());
}

#[test]
fn a_failed_write_changes_nothing() {
    // fail each write of the commit in turn, until the commit has no write left to fail
    let mut failed = 0;
    for after in 0usize.. {
        let mut fs = fresh();
        let home = fs.lookup(ROOT_INO, b"home").unwrap();
        let f = fs.create(home, b"f", KIND_FILE, true).unwrap();
        fs.write_at(f, 0, b"one").unwrap();
        fs.sync().unwrap();
        fs.write_all(f, &vec![9u8; 20_000]).unwrap();
        let w = fs.disk().writes;
        fs.disk().fail_after_writes = Some(w + after);
        match fs.sync() {
            Ok(()) => break,
            Err(e) => assert_eq!(e, Error::Io),
        }
        failed += 1;
        let mut d = fs.into_disk();
        d.fail_after_writes = None;
        let (mut fs, _) = Fs::mount(d).unwrap();
        let mut buf = [0u8; 3];
        fs.read_at(f, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"one", "failing after {} writes", after);
        assert!(fs.check().clean());
    }
    // the commit wrote its data, the inode block, the free map and the root record
    assert!(failed >= 4, "only {} writes to fail", failed);
}

#[test]
fn fuzz_entry_points_on_seeds_and_random_inputs() {
    let n = env_num("AFRIFS_FUZZ_ROUNDS", 300);
    let mut rng = Rng(0xf022_0011);
    // a valid image passes through the image target untouched
    let img = sample_image();
    let mut whole = img.sector(0).to_vec();
    for b in 0..64u64 {
        for k in 0..8 {
            whole.extend_from_slice(&img.sector(AREA_START + b * 8 + k));
        }
    }
    fuzz_image(&whole);
    fuzz_meta(&[]);
    fuzz_walk(b"\x04docs/a.txt");
    fuzz_walk(b"\x00home/docs/../docs");
    fuzz_ops(&[]);
    for _ in 0..n {
        let len = rng.below(3000) as usize;
        let data = rng.bytes(len);
        fuzz_image(&data);
        let mut mutated = whole.clone();
        for _ in 0..rng.below(8) + 1 {
            let at = rng.below(mutated.len() as u64) as usize;
            mutated[at] ^= rng.next() as u8 | 1;
        }
        fuzz_image(&mutated);
        let k = rng.below(40) as usize;
        fuzz_meta(&rng.bytes(k));
        let k = rng.below(80) as usize;
        fuzz_walk(&rng.bytes(k));
        let k = rng.below(400) as usize;
        fuzz_ops(&rng.bytes(k));
    }
}

/// With DUMP_SEEDS=<folder> (scripts/fuzz-cloud.sh), writes a starting corpus for the afrifs_* fuzz targets.
#[test]
fn write_fuzz_seeds() {
    let Ok(dir) = std::env::var("DUMP_SEEDS") else { return };
    let img = sample_image();
    let mut whole = img.sector(0).to_vec();
    for b in 0..64u64 {
        for k in 0..8 {
            whole.extend_from_slice(&img.sector(AREA_START + b * 8 + k));
        }
    }
    let seeds: Vec<(&str, &str, Vec<u8>)> = vec![
        ("afrifs_image", "valid", whole),
        ("afrifs_meta", "none", Vec::new()),
        ("afrifs_meta", "label", vec![0, 20, 0, 1]),
        ("afrifs_meta", "root", vec![1, 8, 0, 9]),
        ("afrifs_walk", "docs", b"\x03docs/a.txt".to_vec()),
        ("afrifs_walk", "dotdot", b"\x03docs/../docs".to_vec()),
        ("afrifs_ops", "mix", (0u8..=255).collect()),
    ];
    for (t, name, data) in seeds {
        let d = std::path::Path::new(&dir).join(t);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(name), data).unwrap();
    }
}
