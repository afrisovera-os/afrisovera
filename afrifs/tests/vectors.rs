//! AFR-0011's vectors (docs/spec/vectors/afr0011_vectors.py): the Rust writer makes the same bytes as the Python
//! reference, and the reader refuses what the reference checker refuses.
mod common;

use afrifs::ramdisk::RamDisk;
use afrifs::*;
use common::sha256_hex;

const T: u64 = 1_763_078_400 * 1_000_000_000;

fn vector_volume() -> RamDisk {
    let id: [u8; 16] = core::array::from_fn(|i| i as u8);
    Fs::format(RamDisk::new(16384), id, T).expect("format").into_disk()
}

fn image(d: &RamDisk) -> Vec<u8> {
    let mut img = vec![0u8; 16384 * SECTOR];
    for (s, v) in d.written() {
        let at = *s as usize * SECTOR;
        img[at..at + SECTOR].copy_from_slice(v);
    }
    img
}

fn block(img: &[u8], b: usize) -> &[u8] {
    let at = AREA_START as usize * SECTOR + b * BLOCK;
    &img[at..at + BLOCK]
}

#[test]
fn vector1_crc32c_check_value() {
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
}

#[test]
fn vector3_fresh_volume_byte_for_byte() {
    let img = image(&vector_volume());
    assert_eq!(sha256_hex(&img[..SECTOR]), "70c2e9852b02c53073234b6e6cae359bd1f23d9a9bc97389739939b220d6d695", "label");
    assert_eq!(sha256_hex(block(&img, 0)), "4d0a7ce689aac679e8bb78d1cbf21c3a1e6eee99b45b07a6882ad11afe67104e", "root A");
    assert_eq!(sha256_hex(block(&img, 2)), "06c40f8d761e7136dc16053c8522bd669a6fde5a8b79d6b8041d8f9d64ec022a", "inodes");
    assert_eq!(sha256_hex(block(&img, 3)), "1b5de154ee61be3af706595f3c0de96194772c9bc759c32ba3143475cbab078d", "bitmap");
    assert_eq!(sha256_hex(block(&img, 4)), "07a754e67dd1e767b3b349890fa3f8afd5e0a98c7f8fa4d3b4415e8951755da7", "top folder");
    assert_eq!(sha256_hex(&img), "37f46664e0af7fb908cde581f2084b6efa1659e4de8d2f9b3776ea6d0abcac3a", "whole image");
    // vector 2: the top folder's pointer
    let p = Ptr::decode(block(&img, 2), 256 + 48);
    assert_eq!(p, Ptr { blk: 4, crc: 0x0e22_6e5a, gen: 1 });
}

#[test]
fn vector4_mount_and_refusals() {
    let d = vector_volume();
    let (mut fs, m) = Fs::mount(d.clone()).expect("mount");
    assert_eq!(m, Mounted { generation: 1, fell_back: false, read_only: false });
    let names: Vec<Vec<u8>> = fs.list(ROOT_INO).unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, vec![b"system".to_vec(), b"apps".to_vec(), b"home".to_vec(), b"tmp".to_vec()]);
    assert!(fs.check().clean());
    let flip = |sector: u64, byte: usize| {
        let mut x = d.clone();
        let mut s = x.sector(sector);
        s[byte] ^= 1;
        x.set_sector(sector, s);
        x
    };
    let area = |blk: u64, byte: usize| (AREA_START + blk * 8 + (byte / SECTOR) as u64, byte % SECTOR);
    // the label, and root record A with B empty: mount refuses
    assert_eq!(Fs::mount(flip(0, 20)).err(), Some(Error::Damaged));
    let (s, b) = area(0, 10);
    assert_eq!(Fs::mount(flip(s, b)).err(), Some(Error::Damaged));
    // the inode block: mount refuses (the top of the inode tree)
    let (s, b) = area(2, 300);
    assert_eq!(Fs::mount(flip(s, b)).err(), Some(Error::Damaged));
    // the free map: mount succeeds read-only
    let (s, b) = area(3, 100);
    let (mut fs, m) = Fs::mount(flip(s, b)).expect("mount with a bad free map");
    assert!(m.read_only);
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    assert_eq!(fs.create(home, b"x", KIND_FILE, true).err(), Some(Error::ReadOnly));
    // the top folder's block: mount succeeds, the folder does not list, vfscheck names it
    let (s, b) = area(4, 13);
    let (mut fs, _) = Fs::mount(flip(s, b)).expect("mount with a bad folder block");
    assert_eq!(fs.list(ROOT_INO).err(), Some(Error::Damaged));
    assert!(!fs.check().clean());
    // a torn root record B (generation 2, bad checksum): A is used
    let mut x = d.clone();
    let rb = Root { gen: 2, ..Root::default() }.encode();
    let mut sec = [0u8; SECTOR];
    sec.copy_from_slice(&rb[..SECTOR]);
    sec[40] ^= 0xFF;
    x.set_sector(AREA_START + 8, sec);
    let (_, m) = Fs::mount(x).expect("mount with a torn B");
    assert_eq!(m.generation, 1);
    assert!(m.fell_back);
}

#[test]
fn small_partitions_and_foreign_disks_are_refused() {
    assert_eq!(Fs::format(RamDisk::new(16383), [0; 16], T).err(), Some(Error::NoSpace));
    assert_eq!(Fs::mount(RamDisk::new(16384)).err(), Some(Error::Damaged));
    // an AFRIVFS1 disk is not format 3
    let mut d = RamDisk::new(16384);
    let mut s = [0u8; SECTOR];
    s[..8].copy_from_slice(b"AFRIVFS1");
    d.set_sector(0, s);
    assert_eq!(Fs::mount(d).err(), Some(Error::Damaged));
}

#[test]
fn bigger_volumes_have_taller_free_maps() {
    // 1 GiB: 261,632 blocks, 8 bitmap leaves, a free map of height 1
    let fs = Fs::format(RamDisk::new(2 * 1024 * 1024), [1; 16], T).expect("format 1 GiB");
    let mut d = fs.into_disk();
    let (mut fs, _) = Fs::mount(d.clone()).expect("mount 1 GiB");
    assert!(fs.check().clean(), "{:?}", fs.check().problems);
    let home = fs.lookup(ROOT_INO, b"home").unwrap();
    let f = fs.create(home, b"f", KIND_FILE, true).unwrap();
    fs.write_at(f, 0, &vec![5u8; 200_000]).unwrap();
    fs.sync().unwrap();
    assert!(fs.check().clean(), "{:?}", fs.check().problems);
    d = fs.into_disk();
    let (mut fs, _) = Fs::mount(d).expect("remount");
    let mut buf = vec![0u8; 200_000];
    assert_eq!(fs.read_at(f, 0, &mut buf).unwrap(), 200_000);
    assert!(buf.iter().all(|&x| x == 5));
}
