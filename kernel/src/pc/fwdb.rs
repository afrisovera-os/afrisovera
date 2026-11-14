//! Device firmware files (13 Nov 2026; FIRMWARE_PLAN.md, the architect's verdict and firmware rules of 13 Nov, the
//! owner's yes of 13 Nov).
//!
//! Some chips (most laptop Bluetooth, many Wi-Fi chips) need a file from their vendor sent to them before they
//! work. That file runs on the chip, never on our CPU: it is never mapped executable, and AfriSovera only reads
//! it and hands it to the one device it is for. The file lives in `\EFI\afrisovera\firmware\` on an EFI system
//! partition (esp.rs `read_file`), which anyone who boots the PC can write. So the kernel trusts only this table:
//! a file goes to a device only if its size and its SHA-256 equal the row's, and only if the device is one the row
//! names. There is no override.
//!
//! Every row of `FIRMWARE` is a row of firmware/MANIFEST, with its licence text in firmware/licences/
//! (scripts/fw/fwtool.py check, run by pc-firmware). The binaries are not in git: the image build fetches them
//! from their pinned source and checks the same hashes.
//!
//! `firmware` lists every row and whether its file is there and right; `firmware test` runs the loader's cases on
//! the test files of pc-firmware's disk image.

use core::fmt;
use core::ptr::addr_of_mut;
use core::sync::atomic::{compiler_fence, AtomicBool, Ordering};

use crate::esp::FwRead;

/// The largest firmware file (Intel Bluetooth files are 0.5 to 1 MiB).
pub const MAX: usize = 4 << 20;

/// A device a firmware file may be sent to. Plain data, as hwdb's `Bind`.
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)] // the PCI form comes with the first PCI driver that loads a file
pub enum Dev {
    /// A USB vendor and product id.
    UsbId { vendor: u16, product: u16 },
    /// A PCI vendor and device id.
    PciId { vendor: u16, device: u16 },
    /// No device: `firmware test` only.
    SelfTest,
}

/// One firmware file: its plain name in \EFI\afrisovera\firmware\, its size and SHA-256, and the only devices it
/// may go to.
pub struct Fw {
    pub name: &'static str,
    pub size: usize,
    pub sha256: [u8; 32],
    pub devs: &'static [Dev],
}

/// 64 hex digits as 32 bytes, at compile time (a wrong digit stops the build).
const fn hex(s: &str) -> [u8; 32] {
    let b = s.as_bytes();
    assert!(b.len() == 64);
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 64 {
        let d = match b[i] {
            c @ b'0'..=b'9' => c - b'0',
            c @ b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("fwdb: a sha256 holds a character that is not a lower-case hex digit"),
        };
        out[i / 2] |= if i % 2 == 0 { d << 4 } else { d };
        i += 1;
    }
    out
}

/// A plain file name: 1 to 48 of A-Z a-z 0-9 . _ -, not starting with a dot (so never a folder, never "..").
const fn plain(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 48 || b[0] == b'.' {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if !matches!(b[i], b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-') {
            return false;
        }
        i += 1;
    }
    true
}

/// The test file's size and hash (firmware/MANIFEST, `gen:fwtest`).
const FWTEST_SIZE: usize = 70000;
const FWTEST_SHA: [u8; 32] = hex("738d796683b53552914356087ff339feb1be5d8b796cff59925bbaf9e7320fa8");

/// The AfriSovera test Bluetooth controller (pc-bt); no real device has this id.
const BT_TEST: Dev = Dev::UsbId { vendor: 0x46F4, product: 0xAF02 };

/// Every firmware file AfriSovera may send, one row per firmware/MANIFEST row (fwtool.py check compares them).
const ROWS: [Fw; 4] = [
    Fw { name: "fwtest.bin", size: 70000, sha256: hex("738d796683b53552914356087ff339feb1be5d8b796cff59925bbaf9e7320fa8"), devs: &[Dev::SelfTest] },
    // pc-bt's test patches, for the test Bluetooth controller only (scripts/ci/qemu-hci-test.patch, 46f4:af02)
    Fw { name: "ibt-hw-37.8.10-fw-1.1.1.2c.13.bseq", size: 120, sha256: hex("3e0d0db38c76db7491137442176a40d53e66184bdf4a73430a6f025fa0a3bc3d"), devs: &[BT_TEST] },
    Fw { name: "ibt-hw-37.8.10-fw-1.1.2.2c.13.bseq", size: 120, sha256: hex("14f39d19030c5be4630030e300997db261a6ddd56dfc0a73f39e2f9ef1929fab"), devs: &[BT_TEST] },
    Fw { name: "ibt-hw-37.8.10-fw-1.1.4.2c.13.bseq", size: 120, sha256: hex("d6b5018633ca5a188f5db1e54d13db103900c2a4bd870632dc53cb024d22f143"), devs: &[BT_TEST] },
];
pub static FIRMWARE: [Fw; ROWS.len()] = ROWS;

const _: () = {
    let mut i = 0;
    while i < ROWS.len() {
        assert!(plain(ROWS[i].name) && ROWS[i].size > 0 && ROWS[i].size <= MAX && !ROWS[i].devs.is_empty());
        i += 1;
    }
};

/// Why a file was not sent.
#[derive(Clone, Copy, PartialEq)]
pub enum Why {
    /// the device is not one the row names (nothing was read)
    NotForDevice,
    /// another file is being loaded
    Busy,
    Missing,
    /// the directory entry's size differs from the table's (nothing was read)
    WrongSize(u32),
    Damaged(&'static str),
    /// the file's SHA-256 differs from the table's (nothing was sent)
    WrongHash,
    /// the driver's upload to the device failed
    DeviceRefused,
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Why::NotForDevice => f.write_str("refused: it is not for this device"),
            Why::Busy => f.write_str("refused: another firmware file is being loaded"),
            Why::Missing => f.write_str("missing (it belongs in \\EFI\\afrisovera\\firmware\\)"),
            Why::WrongSize(n) => write!(f, "refused: it is {} bytes, not the size listed", n),
            Why::Damaged(why) => write!(f, "refused: {}", why),
            Why::WrongHash => f.write_str("refused: its sha256 is not the one listed"),
            Why::DeviceRefused => f.write_str("the device did not take it"),
        }
    }
}

/// The one buffer a firmware file is read into, hashed in, and sent from (the architect's Q6 conditions).
#[repr(C, align(4096))]
struct Buf([u8; MAX]);
static mut BUF: Buf = Buf([0; MAX]);
static BUSY: AtomicBool = AtomicBool::new(false);

/// The first `n` bytes of BUF (n <= MAX).
/// SAFETY: the caller holds BUSY (or knows no one does), so nothing else uses BUF meanwhile.
unsafe fn buf(n: usize) -> &'static mut [u8] {
    core::slice::from_raw_parts_mut(addr_of_mut!(BUF) as *mut u8, n.min(MAX))
}

/// Held while BUF is in use: on every way out it zeroes what was read and frees the buffer.
struct Held(usize);

impl Drop for Held {
    fn drop(&mut self) {
        // SAFETY: BUSY is held, so no one else touches BUF; self.0 <= MAX.
        unsafe { buf(self.0).fill(0) };
        compiler_fence(Ordering::SeqCst);
        BUSY.store(false, Ordering::Release);
    }
}

/// The row for `name`, for a driver.
#[allow(dead_code)] // the first caller is the Bluetooth driver
pub fn by_name(name: &str) -> Option<&'static Fw> {
    FIRMWARE.iter().find(|f| f.name == name)
}

/// Send `fw` to `dev`: refused at once unless the row names `dev`; otherwise the file is read once into the
/// buffer, its size and SHA-256 checked, and `upload` called with that same buffer (it returns true when the
/// device took it). The buffer is zeroed afterwards whatever happened. Shell context only (it reads disks).
pub fn send(fw: &Fw, dev: Dev, upload: impl FnOnce(&[u8]) -> bool, from: &mut [u8; 32]) -> Result<usize, Why> {
    if !fw.devs.contains(&dev) {
        return Err(Why::NotForDevice);
    }
    load(fw, upload, from)
}

fn load(fw: &Fw, upload: impl FnOnce(&[u8]) -> bool, from: &mut [u8; 32]) -> Result<usize, Why> {
    if fw.size == 0 || fw.size > MAX {
        return Err(Why::Damaged("its listed size is outside what a firmware file may be"));
    }
    if BUSY.swap(true, Ordering::Acquire) {
        return Err(Why::Busy);
    }
    let _held = Held(fw.size);
    // SAFETY: BUSY is ours until _held drops; fw.size <= MAX.
    let buf = unsafe { buf(fw.size) };
    match crate::esp::read_file(fw.name, fw.size, buf, from) {
        FwRead::Read => {}
        FwRead::Missing => return Err(Why::Missing),
        FwRead::WrongSize(n) => return Err(Why::WrongSize(n)),
        FwRead::Bad(why) => return Err(Why::Damaged(why)),
    }
    if crate::sha256::digest(&*buf) != fw.sha256 {
        return Err(Why::WrongHash);
    }
    // the same bytes that were hashed, never read again from the disk
    if upload(&*buf) {
        Ok(fw.size)
    } else {
        Err(Why::DeviceRefused)
    }
}

/// The test files on pc-firmware's disk image, each with what the loader must say. They share the test file's
/// size and hash; each image file differs in one way.
const TESTS: [(&str, &str, Option<Why>); 7] = [
    ("fwtest.bin", "the right file, in a cluster chain with a gap", None),
    ("fwtest-byte.bin", "one byte changed", Some(Why::WrongHash)),
    ("fwtest-size.bin", "one byte longer", Some(Why::WrongSize(70001))),
    ("fwtest-loop.bin", "a cluster chain that loops", Some(Why::Damaged("its cluster chain loops back on itself"))),
    ("fwtest-short.bin", "a cluster chain that ends early", Some(Why::Damaged("its cluster chain ends before the file's size"))),
    ("fwtest-far.bin", "a cluster number past the partition", Some(Why::Damaged("its cluster chain points outside the partition"))),
    ("fwtest-gone.bin", "not on the disk", Some(Why::Missing)),
];

/// `firmware [test]`
pub fn command(arg: &str) {
    let mut from = [0u8; 32];
    if arg.trim() == "test" {
        let (mut ok, mut n) = (0, 0);
        let mut case = |name: &str, got: Result<usize, Why>, want: Option<Why>| {
            let good = match (got, want) {
                (Ok(k), None) => k == FWTEST_SIZE,
                (Err(g), Some(w)) => g == w,
                _ => false,
            };
            n += 1;
            if good {
                ok += 1;
            }
            let tail = if good { "as expected" } else { "NOT as expected" };
            match got {
                Ok(k) => crate::kprintln!("firmware test: {}: sent, {} bytes ({})", name, k, tail),
                Err(w) => crate::kprintln!("firmware test: {}: {} ({})", name, w, tail),
            }
        };
        for (name, what, want) in TESTS {
            crate::kprintln!("firmware test: {} ({})", name, what);
            let fw = Fw { name, size: FWTEST_SIZE, sha256: FWTEST_SHA, devs: &[Dev::SelfTest] };
            let mut seen = 0usize;
            let mut sum = [0u8; 32];
            let got = send(&fw, Dev::SelfTest, |b| {
                seen = b.len();
                sum = crate::sha256::digest(b);
                true
            }, &mut from);
            // the upload must have seen exactly the hashed bytes, or nothing at all
            let got = match got {
                Ok(k) if seen != k || sum != FWTEST_SHA => Err(Why::DeviceRefused),
                g => g,
            };
            case(name, got, want);
        }
        // rule 2: a file goes only to a device its row names; refused before a sector is read
        let fw = &FIRMWARE[0];
        let got = send(fw, Dev::UsbId { vendor: 0x8087, product: 0x0026 }, |_| true, &mut from);
        case("fwtest.bin to a device its row does not name", got, Some(Why::NotForDevice));
        // the buffer holds nothing after the cases
        // SAFETY: shell context; nothing holds BUF now (BUSY is free after every case).
        let clean = unsafe { buf(MAX).iter().all(|&b| b == 0) };
        n += 1;
        if clean {
            ok += 1;
        }
        crate::kprintln!("firmware test: the buffer is zero after every case ({})", if clean { "as expected" } else { "NOT as expected" });
        crate::kprintln!("firmware test: {} of {} cases as expected", ok, n);
        return;
    }
    crate::kprintln!(
        "firmware: {} file(s) listed; each must match its size and sha256 before it may go to its device",
        FIRMWARE.len()
    );
    for fw in FIRMWARE.iter() {
        match load(fw, |_| true, &mut from) {
            Ok(k) => crate::kprintln!(
                "firmware: {} ({} bytes): present on {}, sha256 as listed",
                fw.name,
                k,
                crate::gpt::label_str(&from)
            ),
            Err(w) => crate::kprintln!("firmware: {} ({} bytes): {}", fw.name, fw.size, w),
        }
    }
}
