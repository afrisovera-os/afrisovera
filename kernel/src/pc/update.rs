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

/// This system's own release version. An update must carry a higher one (AFR-0010 §6). 1 unless the build sets
/// AFRI_RELEASE (test builds of a version-2 kernel, safe updates U4; a release build refuses it, pc/trial.rs).
pub const RELEASE: u32 = parse_u32(option_env!("AFRI_RELEASE"), 1);

const fn parse_u32(s: Option<&str>, default: u32) -> u32 {
    let Some(s) = s else { return default };
    let b = s.as_bytes();
    assert!(!b.is_empty() && b.len() <= 9, "AFRI_RELEASE must be a number");
    let (mut i, mut v) = (0, 0u32);
    while i < b.len() {
        assert!(b[i].is_ascii_digit(), "AFRI_RELEASE must be a number");
        v = v * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    v
}
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

/// One sector of the stick. A read that fails is tried again, up to three times in all: a stick the driver had to
/// reset answers the next read (seen on QEMU under load: "the data stage got no answer", 14 Nov 2026), and the
/// driver names every failure in its own line. Nothing read here is trusted anyway: the header is signed and the
/// boot file's SHA-256 checked, and staging compares the ESP's copy with it again.
fn rd(n: u64, b: &mut [u8; 512]) -> bool {
    (0..3).any(|_| crate::xhci::msc_read_only(n, b))
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
fn check(cmd: &str) -> Result<Result<(Checked, Vol, u32), &'static str>, &'static str> {
    if crate::xhci::msc_read_only_capacity() == 0 {
        return Err("no USB stick; put update.afu on a FAT32 stick and plug it in");
    }
    let v = match volume() {
        Ok(v) => v,
        Err(why) => {
            crate::kprintln!("{}: the USB stick has no FAT32 file system AfriSovera can read ({})", cmd, why);
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
        crate::kprintln!("{}: it is version {}; this system is version {}", cmd, c.version, RELEASE);
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
    Ok(Ok((c, v, clus)))
}

/// `updatecheck`: read and check update.afu on the USB stick. Reads only.
pub fn command() {
    match check("updatecheck") {
        Err(why) => crate::kprintln!("updatecheck: {}; nothing written", why),
        Ok(Err(why)) => crate::kprintln!("updatecheck: REFUSED: {}; nothing written", why),
        Ok(Ok((c, _, _))) => {
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

// --- `updatestage [yes <code>]` / `updatestage remove [yes <code>]` (safe updates U3, AFR-0010 section 8) ---
//
// The checked boot file copied from the stick into the SPARE slot of \EFI\afrisovera (the file the main entry does
// NOT name), read back from the EFI partition and its SHA-256 compared with the signed header, then a Boot####
// "AfriSovera trial" added at the END of BootOrder. The running system and the main entry are never touched; nothing
// boots the spare until `updatetry` (U4). The architect's rules (ARCHITECT_REVIEWS 14 Nov 2026) are in esp/stage.rs.

/// The boot file inside update.afu (from byte BODY), read from the stick in order for the EFI writer, hashed as it
/// goes. It never gives more than the signed size, and asks the stick for each sector once.
struct StickFile<'a> {
    v: &'a Vol,
    fat: Fat,
    c: u32,      // the cluster holding the next sector to load
    k: u64,      // that sector's place in its cluster
    loaded: u64, // file sectors loaded so far (`cur` holds sector loaded - 1)
    cur: [u8; 512],
    size: u64,
    next: u64, // the byte `fill` must be asked for next
    sha: Sha256,
}

impl StickFile<'_> {
    /// File sector `want` (of update.afu) into `cur`; only forward.
    fn load(&mut self, want: u64) -> bool {
        while self.loaded <= want {
            if self.k == self.v.spc {
                match self.v.next_in(self.c, &mut self.fat) {
                    Some(n) => self.c = n,
                    None => return false,
                }
                self.k = 0;
            }
            if !rd(self.v.sector(self.c) + self.k, &mut self.cur) {
                return false;
            }
            self.k += 1;
            self.loaded += 1;
        }
        self.loaded == want + 1
    }
}

impl crate::esp::Source for StickFile<'_> {
    fn fill(&mut self, from: usize, out: &mut [u8]) -> bool {
        if from as u64 != self.next {
            return false; // asked out of order: the writer never does this
        }
        self.next += out.len() as u64;
        let end = (from as u64 + out.len() as u64).min(self.size);
        let mut b = from as u64;
        while b < end {
            let at = BODY + b;
            if !self.load(at / 512) {
                return false;
            }
            let o = (at % 512) as usize;
            let n = ((512 - o) as u64).min(end - b) as usize;
            let d = (b - from as u64) as usize;
            out[d..d + n].copy_from_slice(&self.cur[o..o + n]);
            b += n as u64;
        }
        if end > from as u64 {
            self.sha.update(&out[..(end - from as u64) as usize]);
        }
        true
    }
}

/// The spare slot for the file the main entry names: Some((spare's long name, its full path)).
fn spare_for(main: &[u8]) -> Option<(&'static str, &'static str)> {
    if main == b"\\efi\\afrisovera\\afrisovera.efi" {
        Some((crate::esp::stage::SLOT_B, "\\EFI\\afrisovera\\afrisovera-b.efi"))
    } else if main == b"\\efi\\afrisovera\\afrisovera-b.efi" {
        Some((crate::esp::stage::SLOT_A, "\\EFI\\afrisovera\\afrisovera.efi"))
    } else {
        None
    }
}

/// The main entry, checked as the architect set (exactly one "AfriSovera", BootCurrent equal to it, naming one of
/// our two files): (it, the spare's long name, the spare's path), or the line to print.
pub(crate) fn main_and_spare(cmd: &str) -> Option<(crate::bootent::Main, &'static str, &'static str)> {
    use crate::bootent::NoMain;
    let m = match crate::bootent::main_exact() {
        Ok(m) => m,
        Err(NoMain::NotRead) => {
            crate::kprintln!("{}: the firmware's boot entries could not be read at power-on; nothing written", cmd);
            return None;
        }
        Err(NoMain::Count(0)) => {
            crate::kprintln!("{}: no boot entry is named exactly \"AfriSovera\" (bootentry adds it); nothing written", cmd);
            return None;
        }
        Err(NoMain::Count(n)) => {
            crate::kprintln!("{}: {} boot entries are named exactly \"AfriSovera\", so which one is this system's is not known; nothing written", cmd, n);
            return None;
        }
        Err(NoMain::NoPartition) => {
            crate::kprintln!("{}: the \"AfriSovera\" boot entry names no GPT partition; nothing written", cmd);
            return None;
        }
    };
    if crate::bootent::current() != Some(m.num) {
        crate::kprintln!("{}: this system was not started by AfriSovera's own boot entry (a USB stick, a trial, or another entry); nothing written", cmd);
        return None;
    }
    let Some((long, path)) = spare_for(&m.file[..m.file_n]) else {
        crate::kprintln!("{}: AfriSovera's boot entry names a file that is not one of its two boot files; nothing written", cmd);
        return None;
    };
    Some((m, long, path))
}

fn kib(b: u64) -> u64 {
    b.div_ceil(1024)
}

/// `updatestage [yes <code>]`
pub fn stage(given: Option<&str>) {
    const CMD: &str = "updatestage";
    // 1: U1's whole check
    let (c, v, clus) = match check(CMD) {
        Err(why) => {
            crate::kprintln!("{}: {}; nothing written", CMD, why);
            return;
        }
        Ok(Err(why)) => {
            crate::kprintln!("{}: REFUSED: {}; nothing written", CMD, why);
            return;
        }
        Ok(Ok(x)) => x,
    };
    if c.boot_size > crate::esp::stage::STAGE_MAX {
        crate::kprintln!("{}: REFUSED: its boot file is {} KiB; staging takes at most {} KiB; nothing written", CMD, kib(c.boot_size), kib(crate::esp::stage::STAGE_MAX));
        return;
    }
    // 2: the main entry and the spare, read, not assumed
    let Some((m, spare, path)) = main_and_spare(CMD) else { return };
    // 3: the EFI partition the main entry names
    let esp = match crate::esp::stage::open(&m.guid) {
        Ok(e) => e,
        Err(why) => {
            crate::kprintln!("{}: {}; nothing written", CMD, why);
            return;
        }
    };
    let cb = esp.cluster_bytes();
    let free = match esp.free() {
        Ok(f) => f,
        Err(why) => {
            crate::kprintln!("{}: the EFI partition: {}; nothing written", CMD, why);
            return;
        }
    };
    let old = match esp.live(spare) {
        Ok(o) => o,
        Err(why) => {
            crate::kprintln!("{}: \\EFI\\afrisovera: {}; nothing written", CMD, why);
            return;
        }
    };
    let old_clusters = old.map_or(0, |o| (o.size as u64).div_ceil(cb));
    let need = c.boot_size.div_ceil(cb).max(1);
    let keep = crate::esp::stage::KEEP_FREE.div_ceil(cb);
    if free + old_clusters < need + keep {
        crate::kprintln!(
            "{}: the EFI partition has {} KiB free; the update needs {} KiB and 2 MiB must stay free; nothing written",
            CMD,
            kib((free + old_clusters) * cb),
            kib(need * cb)
        );
        return;
    }
    let mut hex = [0u8; 64];
    sha256::to_hex(&c.boot_sha, &mut hex);
    crate::kprintln!(
        "{}: version {} ({} KiB, SHA-256 {}) -> {} in the EFI partition AfriSovera's boot entry names; the running system ({}) is not touched",
        CMD,
        c.version,
        kib(c.boot_size),
        core::str::from_utf8(&hex[..16]).unwrap_or("?"),
        path,
        core::str::from_utf8(&m.file[..m.file_n]).unwrap_or("?")
    );
    if c.demo {
        crate::kprintln!("{}: signed by the DEMO release key: a test build, never a real update", CMD);
    }
    match old {
        Some(o) => crate::kprintln!(
            "  this replaces the older system kept in the spare slot ({} KiB); after this, updateback returns to version {} once the new one is kept",
            kib(o.size as u64),
            RELEASE
        ),
        None => crate::kprintln!("  the spare slot is empty; after this, updateback returns to version {} once the new one is kept", RELEASE),
    }
    crate::kprintln!("  then a boot entry \"AfriSovera trial\" is added at the END of BootOrder; nothing starts it until updatetry");
    let Some(want) = esp.code(CMD.as_bytes(), c.version, &c.boot_sha) else {
        crate::kprintln!("{}: a read FAILED; nothing written", CMD);
        return;
    };
    if !crate::esp::agreed(CMD, given, want) {
        return;
    }
    // 4: no entry may point at the spare while it changes
    if let Err(why) = crate::bootent::trial_remove() {
        crate::kprintln!("{}: the old trial entry: {}; nothing else written", CMD, why);
        return;
    }
    // a chain a power cut left behind, freed under the three rules
    match esp.reclaim() {
        Ok((n, k, left)) => {
            if k > 0 {
                crate::kprintln!("{}: freed {} KiB a cut-off earlier stage left behind ({} chain(s))", CMD, kib(n * cb), k);
            }
            if left > 0 {
                crate::kprintln!("{}: {} old deleted entry(ies) of ours left alone (their clusters are not certainly ours)", CMD, left);
            }
        }
        Err(why) => {
            crate::kprintln!("{}: {}; the spare slot was not changed", CMD, why);
            return;
        }
    }
    // the old spare out
    if let Err(why) = esp.free_file(spare) {
        crate::kprintln!("{}: {}. The main system is untouched; the way back (updateback) is gone until a stage succeeds", CMD, why);
        return;
    }
    // 5: the new spare, streamed from the stick
    let mut src = StickFile { v: &v, fat: Fat { at: u64::MAX, s: [0; 512] }, c: clus, k: 0, loaded: 0, cur: [0; 512], size: c.boot_size, next: 0, sha: Sha256::new() };
    crate::kprintln!("{}: writing", CMD);
    let wrote = esp.write(spare, c.boot_size, &mut src);
    let stick_sha = src.sha.finish();
    if let Err(why) = wrote {
        crate::kprintln!("{}: {}. Nothing points at the spare file; the main system is untouched; the way back (updateback) is gone until a stage succeeds", CMD, why);
        return;
    }
    // 6: what is written is what was checked
    let back = esp.read_back(spare, c.boot_size);
    let bad = match back {
        Err(why) => Some(why),
        Ok((sha, _)) if sha != c.boot_sha => Some("the copy in the EFI partition does not match the SHA-256 in the signed header"),
        Ok(_) if stick_sha != c.boot_sha => Some("the USB stick gave different bytes this time"),
        Ok(_) => None,
    };
    if let Some(why) = bad {
        let freed = esp.free_file(spare).is_ok();
        crate::kprintln!(
            "{}: REFUSED: {}; the spare file was {}; no boot entry points at it; the main system is untouched",
            CMD,
            why,
            if freed { "taken out again" } else { "NOT taken out (updatestage remove takes it out)" }
        );
        return;
    }
    // the record (advice only): the spare's version and SHA, and the running file's own
    let run_long = if spare == crate::esp::stage::SLOT_B { crate::esp::stage::SLOT_A } else { crate::esp::stage::SLOT_B };
    let running = esp.live(run_long).ok().flatten().and_then(|l| esp.read_back(run_long, l.size as u64).ok()).map(|b| b.0);
    crate::trial::note_staged(if spare == crate::esp::stage::SLOT_B { 1 } else { 0 }, c.version, &c.boot_sha, running);
    // 7: the trial entry, last in BootOrder
    match crate::bootent::trial_add(&m, path) {
        Ok(num) => {
            let nm = crate::bootent::name(num);
            let free_now = back.map_or(0, |b| b.1);
            crate::kprintln!(
                "{}: version {} staged in {}, read back the same ({} KiB free now); {} \"AfriSovera trial\" added at the END of BootOrder; updatetry starts it once; nothing else changed",
                CMD,
                c.version,
                path,
                kib(free_now * cb),
                core::str::from_utf8(&nm).unwrap_or("?")
            );
        }
        Err(why) => crate::kprintln!("{}: the spare file is written and read back, but the trial entry: {}; updatestage again adds it", CMD, why),
    }
}

/// `updatestage remove [yes <code>]`: the trial entry out (BootOrder first), then the spare file, only if the main
/// entry does not name it.
pub fn unstage(given: Option<&str>) {
    const CMD: &str = "updatestage remove";
    let Some((m, spare, _)) = main_and_spare(CMD) else { return };
    let esp = match crate::esp::stage::open(&m.guid) {
        Ok(e) => e,
        Err(why) => {
            crate::kprintln!("{}: {}; nothing written", CMD, why);
            return;
        }
    };
    let old = match esp.live(spare) {
        Ok(o) => o,
        Err(why) => {
            crate::kprintln!("{}: \\EFI\\afrisovera: {}; nothing written", CMD, why);
            return;
        }
    };
    let trial = crate::bootent::trial_number();
    if old.is_none() && trial.is_none() {
        crate::kprintln!("{}: nothing is staged (no spare file, no trial entry); nothing to do", CMD);
        return;
    }
    crate::kprintln!(
        "{}: {}{}; the running system ({}) and its entry stay",
        CMD,
        if trial.is_some() { "the \"AfriSovera trial\" entry is taken out" } else { "no trial entry" },
        if old.is_some() { ", then the spare file is taken out" } else { "" },
        core::str::from_utf8(&m.file[..m.file_n]).unwrap_or("?")
    );
    let Some(want) = esp.code(CMD.as_bytes(), 0, &[0; 32]) else {
        crate::kprintln!("{}: a read FAILED; nothing written", CMD);
        return;
    };
    if !crate::esp::agreed(CMD, given, want) {
        return;
    }
    if let Err(why) = crate::bootent::trial_remove() {
        crate::kprintln!("{}: {}; the spare file was not touched", CMD, why);
        return;
    }
    match esp.free_file(spare) {
        Ok(n) => crate::kprintln!("{}: done; {} KiB freed; the firmware's boot entries and \\EFI\\afrisovera are as before updatestage", CMD, kib(n * esp.cluster_bytes())),
        Err(why) => crate::kprintln!("{}: the trial entry is out, but the spare file: {}", CMD, why),
    }
}
