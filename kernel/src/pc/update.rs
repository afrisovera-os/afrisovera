//! Safe updates, step U1 (AFR-0010, 13 Nov 2026): `updatecheck` reads the update file `update.afu` from the
//! root of a FAT32 USB stick and checks it completely. It WRITES NOTHING: not the stick, not a disk, not a
//! firmware variable. Later steps (`updatestage`, `updatetry`) act only on a file this check accepts.
//!
//! The file (AFR-0010 §4): a 128-byte header, a 64-byte Ed25519 signature, then the new boot file.
//!   0   magic "AFRIUPD1"            8  header length (u32, 128)   12  format (u32, 1)
//!   16  target, 16 bytes ASCII, zero padded ("pc-x86_64")         32  release version (u32)
//!   36  reserved (u32, 0)           40  key id: the first 8 bytes of SHA-256(public key)
//!   48  boot file size (u64)        56  SHA-256 of the boot file  88-127 reserved (zero)
//!   128 signature over "AfriSovera update v1", one zero byte, then header bytes 0-127
//!   192 the boot file, exactly "boot file size" bytes, and nothing after it
//! Every field the device acts on is inside the signed header. A key has ONE purpose: the table below holds
//! keys for updates only, so an app publisher's key (integrity.rs) can never pass an update, and the other
//! way round.
//!
//! The stick is untrusted. Every length is bounded before it is used (header 128 bytes, boot file 64 MiB at
//! most, every cluster chain no longer than the file system's cluster count). The boot file is streamed
//! through SHA-256 a sector at a time, never held in memory, and the header is read twice: the second read
//! must give the very bytes the signature was checked over.

use crate::sha256::{self, Sha256};

/// This system's own release version. An update must carry a higher one (AFR-0010 §6).
pub const RELEASE: u32 = 1;
/// The machine an update must be made for.
pub const TARGET: &[u8] = b"pc-x86_64";

const MAGIC: &[u8; 8] = b"AFRIUPD1";
const HDR: usize = 128;
const SIG: usize = 64;
const BODY: u64 = (HDR + SIG) as u64;
const FORMAT: u32 = 1;
const BOOT_MAX: u64 = 64 << 20;
const CONTEXT: &[u8] = b"AfriSovera update v1\0";
const NAME: &[u8; 11] = b"UPDATE  AFU";

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Update,
}

struct Key {
    public: [u8; 32],
    purpose: Purpose,
    /// a key whose secret is in the repo (scripts/make-update.py): test builds only
    demo: bool,
}

/// The demo release key's public half: Ed25519 seed 21 22 .. 40 (scripts/make-update.py), NOT a secret.
#[rustfmt::skip]
const DEMO_UPDATE_KEY: [u8; 32] = [
    0xe7, 0xf1, 0x62, 0xa1, 0x0b, 0xec, 0x55, 0x9a, 0xfe, 0xa1, 0x95, 0xe4, 0xdc, 0xe8, 0x4b, 0x69,
    0x56, 0x8d, 0x5d, 0x2c, 0xb0, 0x96, 0x3e, 0xb4, 0x46, 0xc0, 0x68, 0x5e, 0x2b, 0x17, 0xf2, 0xf0,
];

/// The keys this system trusts, each for one purpose. Only a demo key today: the real release key is made
/// and kept offline by the owner (AFR-0010 §7) and replaces this entry in a release build.
static KEYS: [Key; 1] = KEY_TABLE;
const KEY_TABLE: [Key; 1] = [Key { public: DEMO_UPDATE_KEY, purpose: Purpose::Update, demo: true }];

/// The demo app publisher key in `integrity.rs` (seed 01 02 .. 20, scripts/sign-app.py), NOT a secret.
#[cfg(feature = "release")]
#[rustfmt::skip]
const DEMO_APP_KEY: [u8; 32] = [
    0x79, 0xb5, 0x56, 0x2e, 0x8f, 0xe6, 0x54, 0xf9, 0x40, 0x78, 0xb1, 0x12, 0xe8, 0xa9, 0x8b, 0xa7,
    0x90, 0x1f, 0x85, 0x3a, 0xe6, 0x95, 0xbe, 0xd7, 0xe0, 0xe3, 0x91, 0x0b, 0xad, 0x04, 0x96, 0x64,
];

/// True when `a` and `b` are the same 32 bytes (usable in a const check).
#[cfg(feature = "release")]
const fn same32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut j = 0;
    while j < 32 {
        if a[j] != b[j] {
            return false;
        }
        j += 1;
    }
    true
}

/// True when some row of `t` holds the demo update key's bytes, whatever its `demo` flag says.
#[cfg(feature = "release")]
const fn holds_demo_update_key(t: &[Key]) -> bool {
    let mut i = 0;
    while i < t.len() {
        if same32(&t[i].public, &DEMO_UPDATE_KEY) {
            return true;
        }
        i += 1;
    }
    false
}

// AFR-0010 §7: a build marked for release (`--features pc,release`) must trust no demo key, update or app.
// Checked as the kernel compiles, so such a build cannot be made at all, in CI or anywhere else; the suite
// pc-releaseguard proves the check is armed. Today it always stops: the owner's keys do not exist yet.
// The update table is checked twice: by the `demo` flag, and by the key's bytes, so a demo key copied into a
// row marked `demo: false` is caught too (architect, 14 Nov 2026).
#[cfg(feature = "release")]
const _: () = {
    let mut i = 0;
    while i < KEY_TABLE.len() {
        assert!(!KEY_TABLE[i].demo, "release build refused: it trusts the DEMO update key (AFR-0010 section 7)");
        i += 1;
    }
};
#[cfg(feature = "release")]
const _: () = assert!(
    !holds_demo_update_key(&KEY_TABLE),
    "release build refused: a key table row holds the DEMO update key's bytes (AFR-0010 section 7)"
);
#[cfg(feature = "release")]
const _: () = assert!(
    !same32(&crate::integrity::PUBLISHER_PUBKEY, &DEMO_APP_KEY),
    "release build refused: it trusts the DEMO app publisher key (AFR-0010 section 7)"
);
// pc-releaseguard's planted row: the demo update key's bytes in a row marked `demo: false`. Built only with the
// test feature `releaseguard-plant`; this message proves the byte check catches what the flag check cannot.
#[cfg(feature = "releaseguard-plant")]
const _: () = assert!(
    !holds_demo_update_key(&[Key { public: DEMO_UPDATE_KEY, purpose: Purpose::Update, demo: false }]),
    "release build refused: the planted row (demo: false) holds the DEMO update key's bytes (AFR-0010 section 7)"
);

/// The trusted key for `purpose` whose id (first 8 bytes of its SHA-256) is `id`.
fn key_for(id: &[u8], purpose: Purpose) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.purpose == purpose && sha256::digest(&k.public)[..8] == *id)
}

/// One boot line, so a test build can never pass for a release one.
pub fn boot_line() {
    if KEYS.iter().any(|k| k.demo) {
        crate::kprintln!("[pc] updates: release {}; trusts the DEMO release key: a test build, not for release", RELEASE);
    } else {
        crate::kprintln!("[pc] updates: release {}", RELEASE);
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64le(b: &[u8], o: usize) -> u64 {
    u32le(b, o) as u64 | (u32le(b, o + 4) as u64) << 32
}

fn rd(n: u64, b: &mut [u8; 512]) -> bool {
    crate::xhci::msc_read_only(n, b)
}

// --- a READ-ONLY FAT32 reader for the stick (512-byte sector numbers throughout) ---

struct Vol {
    spc: u64,      // 512-byte sectors per cluster
    fat0: u64,     // the first FAT's first sector
    data0: u64,    // cluster 2's first sector
    clusters: u64, // data clusters, numbered 2 ..= clusters + 1
    root: u32,
}

fn fat32_at(start: u64, cap: u64) -> Result<Vol, &'static str> {
    let mut s = [0u8; 512];
    if !rd(start, &mut s) {
        return Err("its first sector could not be read");
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        return Err("no file system");
    }
    let bps = u16le(&s, 11) as u64;
    let spc = s[13] as u64;
    let rsvd = u16le(&s, 14) as u64;
    let nfats = s[16] as u64;
    let tot = if u16le(&s, 19) != 0 { u16le(&s, 19) as u64 } else { u32le(&s, 32) as u64 };
    let fatsz = u32le(&s, 36) as u64;
    if !matches!(bps, 512 | 1024 | 2048 | 4096) || spc == 0 || !spc.is_power_of_two() || spc > 128 || rsvd == 0 || !(1..=2).contains(&nfats) {
        return Err("not a FAT file system");
    }
    if u16le(&s, 17) != 0 || u16le(&s, 22) != 0 || fatsz == 0 {
        return Err("FAT12 or FAT16, not FAT32");
    }
    let per = bps / 512;
    let meta = rsvd + nfats * fatsz;
    if meta >= tot || start + tot * per > cap {
        return Err("a damaged FAT32 boot sector");
    }
    let clusters = (tot - meta) / spc;
    if clusters < 65525 || (clusters + 2) * 4 > fatsz * bps {
        return Err("a damaged FAT32 boot sector");
    }
    let root = u32le(&s, 44);
    if root < 2 || root as u64 >= clusters + 2 {
        return Err("a damaged FAT32 boot sector");
    }
    Ok(Vol { spc: spc * per, fat0: start + rsvd * per, data0: start + meta * per, clusters, root })
}

/// The stick's FAT32: the whole stick (no table), the first FAT partition of an MBR, or GPT entry 1.
fn volume() -> Result<Vol, &'static str> {
    let cap = crate::xhci::msc_read_only_capacity();
    let mut s = [0u8; 512];
    if !rd(0, &mut s) {
        return Err("its first sector could not be read");
    }
    if &s[82..90] == b"FAT32   " {
        return fat32_at(0, cap);
    }
    if s[510] != 0x55 || s[511] != 0xAA {
        return Err("no partition table");
    }
    // Partition starts and the GPT's own place are counted in the stick's blocks. Until a stick with blocks
    // other than 512 bytes is read in its own units, it is refused here rather than read in the wrong place.
    if crate::xhci::msc_block_size() != 512 {
        return Err("a partition table on a stick whose blocks are not 512 bytes, not read yet");
    }
    if s[450] == 0xEE {
        let mut h = [0u8; 512];
        if !rd(1, &mut h) || &h[..8] != b"EFI PART" {
            return Err("a damaged GPT");
        }
        let ents = u64le(&h, 72);
        let mut e = [0u8; 512];
        if ents >= cap || !rd(ents, &mut e) {
            return Err("a damaged GPT");
        }
        return fat32_at(u64le(&e, 32), cap);
    }
    for i in 0..4 {
        let o = 446 + 16 * i;
        if matches!(s[o + 4], 0x0B | 0x0C | 0xEF) {
            return fat32_at(u32le(&s, o + 8) as u64, cap);
        }
    }
    Err("no FAT32 partition")
}

/// One FAT sector kept in memory during a chain walk (`at` is u64::MAX when it holds nothing).
struct Fat {
    at: u64,
    s: [u8; 512],
}

impl Vol {
    /// The FAT entry for cluster `c`: the next cluster, or None at the chain's end or on a bad entry. The last
    /// FAT sector read stays in `f`, so a walk along a chain reads each FAT sector once, not once per cluster.
    fn next_in(&self, c: u32, f: &mut Fat) -> Option<u32> {
        let at = c as u64 * 4;
        let sec = self.fat0 + at / 512;
        if f.at != sec {
            f.at = u64::MAX;
            if !rd(sec, &mut f.s) {
                return None;
            }
            f.at = sec;
        }
        let n = u32le(&f.s, (at % 512) as usize) & 0x0FFF_FFFF;
        if n >= 2 && (n as u64) < self.clusters + 2 { Some(n) } else { None }
    }
    fn sector(&self, c: u32) -> u64 {
        self.data0 + (c as u64 - 2) * self.spc
    }
    /// `update.afu` in the root folder: (first cluster, size). FAT32 allows at most 65,536 entries (2 MiB) in a
    /// folder, so the walk stops there: a looping root chain on a big stick is a refusal, not a hang.
    fn find(&self) -> Result<Option<(u32, u32)>, &'static str> {
        const MAX_SECTORS: u64 = 65_536 * 32 / 512;
        let mut c = self.root;
        let mut read = 0u64;
        let mut fat = Fat { at: u64::MAX, s: [0; 512] };
        loop {
            for k in 0..self.spc {
                if read == MAX_SECTORS {
                    return Err("the root folder's cluster chain is damaged");
                }
                read += 1;
                let mut s = [0u8; 512];
                if !rd(self.sector(c) + k, &mut s) {
                    return Err("the root folder could not be read");
                }
                for e in s.chunks(32) {
                    if e[0] == 0 {
                        return Ok(None);
                    }
                    if e[0] == 0xE5 || e[11] == 0x0F || e[11] & 0x18 != 0 {
                        continue;
                    }
                    if &e[..11] == NAME {
                        let clus = (u16le(e, 20) as u32) << 16 | u16le(e, 26) as u32;
                        return Ok(Some((clus, u32le(e, 28))));
                    }
                }
            }
            match self.next_in(c, &mut fat) {
                Some(n) => c = n,
                None => return Ok(None),
            }
        }
    }
    /// Every 512-byte sector of the file starting at cluster `c`, `size` bytes long, in order, to `each`
    /// (sector index, bytes); false if `each` says stop. Err if the chain is shorter than the size.
    fn stream(&self, mut c: u32, size: u64, mut each: impl FnMut(u64, &[u8; 512]) -> bool) -> Result<(), &'static str> {
        let secs = size.div_ceil(512);
        let mut i = 0u64;
        let mut fat = Fat { at: u64::MAX, s: [0; 512] };
        if c < 2 || c as u64 >= self.clusters + 2 {
            return Err("the file's cluster chain on the stick is damaged");
        }
        loop {
            for k in 0..self.spc {
                if i == secs {
                    return Ok(());
                }
                let mut s = [0u8; 512];
                if !rd(self.sector(c) + k, &mut s) {
                    return Err("the stick could not be read");
                }
                if !each(i, &s) {
                    return Ok(());
                }
                i += 1;
            }
            if i == secs {
                return Ok(());
            }
            match self.next_in(c, &mut fat) {
                Some(n) => c = n,
                None => return Err("the file's cluster chain on the stick is damaged"),
            }
        }
    }
}

/// A checked update: what `updatecheck` reports and later steps act on.
pub struct Checked {
    pub version: u32,
    pub boot_size: u64,
    pub boot_sha: [u8; 32],
    pub demo: bool,
}

/// The header's checks, signature included: Ok(the checked fields, minus the boot file's hash check).
fn header(h: &[u8; 512], file_len: u64) -> Result<Checked, &'static str> {
    if file_len < BODY {
        return Err("it is shorter than an update file's header");
    }
    if &h[..8] != MAGIC {
        return Err("it is not an AfriSovera update file (wrong magic)");
    }
    if u32le(h, 8) as usize != HDR {
        return Err("its header length is not 128 bytes");
    }
    if u32le(h, 12) != FORMAT {
        return Err("its format is not one this system reads");
    }
    if u32le(h, 36) != 0 || h[88..HDR].iter().any(|&b| b != 0) {
        return Err("its reserved bytes are not zero");
    }
    let Some(key) = key_for(&h[40..48], Purpose::Update) else {
        return Err("it is signed by a key this system does not trust for updates");
    };
    let mut msg = [0u8; CONTEXT.len() + HDR];
    msg[..CONTEXT.len()].copy_from_slice(CONTEXT);
    msg[CONTEXT.len()..].copy_from_slice(&h[..HDR]);
    let mut sig = [0u8; SIG];
    sig.copy_from_slice(&h[HDR..HDR + SIG]);
    if !crate::ed25519::verify(&key.public, &msg, &sig) {
        return Err("its signature does not match its header");
    }
    // signed from here on
    let t = &h[16..32];
    let tn = t.iter().position(|&b| b == 0).unwrap_or(16);
    if &t[..tn] != TARGET || t[tn..].iter().any(|&b| b != 0) {
        return Err("it is made for another kind of machine, not pc-x86_64");
    }
    let boot_size = u64le(h, 48);
    if boot_size == 0 || boot_size > BOOT_MAX {
        return Err("its boot file size is out of bounds (1 byte to 64 MiB)");
    }
    if file_len != BODY + boot_size {
        return Err("its length does not match its header (cut short or extra bytes)");
    }
    let mut boot_sha = [0u8; 32];
    boot_sha.copy_from_slice(&h[56..88]);
    Ok(Checked { version: u32le(h, 32), boot_size, boot_sha, demo: key.demo })
}

/// The first bytes of the boot file: an x86-64 UEFI program (PE32+, machine 0x8664)?
fn is_pe64(b: &[u8; 512]) -> bool {
    if &b[..2] != b"MZ" {
        return false;
    }
    let p = u32le(b, 0x3c) as usize;
    p + 26 <= b.len() && &b[p..p + 4] == b"PE\0\0" && u16le(b, p + 4) == 0x8664 && u16le(b, p + 24) == 0x20b
}

/// The whole check. Err(reason) for a file that is refused; the outer Err for a stick that has none.
fn check() -> Result<Result<Checked, &'static str>, &'static str> {
    if crate::xhci::msc_read_only_capacity() == 0 {
        return Err("no USB stick; put update.afu on a FAT32 stick and plug it in");
    }
    let v = match volume() {
        Ok(v) => v,
        Err(why) => {
            crate::kprintln!("updatecheck: the USB stick has no FAT32 file system AfriSovera can read ({})", why);
            return Err("no update file read");
        }
    };
    let (clus, size) = match v.find() {
        Ok(Some(f)) => f,
        Ok(None) => return Err("no update.afu in the USB stick's top folder"),
        Err(why) => return Ok(Err(why)),
    };
    let size = size as u64;
    let mut first = [0u8; 512];
    if size > 0 {
        if let Err(why) = v.stream(clus, size.min(512), |_, s| {
            first = *s;
            false
        }) {
            return Ok(Err(why));
        }
    }
    let c = match header(&first, size) {
        Ok(c) => c,
        Err(why) => return Ok(Err(why)),
    };
    if c.version <= RELEASE {
        crate::kprintln!("updatecheck: it is version {}; this system is version {}", c.version, RELEASE);
        return Ok(Err("it is not newer than this system"));
    }
    // the second read: the signed header again, the boot file through SHA-256, its first 512 bytes kept
    let mut sha = Sha256::new();
    let mut head = [0u8; 512];
    let mut kept = 0usize;
    let mut same = true;
    let total = BODY + c.boot_size;
    let r = v.stream(clus, total, |i, s| {
        let at = i * 512;
        let lo = if at < BODY { (BODY - at) as usize } else { 0 };
        if i == 0 && s[..BODY as usize] != first[..BODY as usize] {
            same = false;
            return false;
        }
        let hi = ((total - at).min(512)) as usize;
        if lo < hi {
            let part = &s[lo..hi];
            sha.update(part);
            let take = (head.len() - kept).min(part.len());
            head[kept..kept + take].copy_from_slice(&part[..take]);
            kept += take;
        }
        true
    });
    if let Err(why) = r {
        return Ok(Err(why));
    }
    if !same {
        return Ok(Err("the USB stick gave different bytes on a second read"));
    }
    if sha.finish() != c.boot_sha {
        return Ok(Err("its boot file does not match the SHA-256 in its header"));
    }
    if !is_pe64(&head) {
        return Ok(Err("its boot file is not an x86-64 UEFI program"));
    }
    Ok(Ok(c))
}

/// `updatecheck`: read and check update.afu on the USB stick. Reads only.
pub fn command() {
    match check() {
        Err(why) => crate::kprintln!("updatecheck: {}; nothing written", why),
        Ok(Err(why)) => crate::kprintln!("updatecheck: REFUSED: {}; nothing written", why),
        Ok(Ok(c)) => {
            let mut hex = [0u8; 64];
            sha256::to_hex(&c.boot_sha, &mut hex);
            crate::kprintln!(
                "updatecheck: update.afu: version {} for pc-x86_64, boot file {} KiB, SHA-256 {}",
                c.version,
                c.boot_size.div_ceil(1024),
                core::str::from_utf8(&hex[..16]).unwrap_or("?")
            );
            if c.demo {
                crate::kprintln!("updatecheck: signed by the DEMO release key: a test build, never a real update");
            }
            crate::kprintln!("updatecheck: OK: version {} is newer than this system (version {}); nothing written", c.version, RELEASE);
        }
    }
}
