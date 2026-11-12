//! A minimal in-RAM filesystem (VFS v0).
//!
//! A flat namespace of small files held in a fixed static table — no heap churn,
//! no allocator pressure, bounded and leak-free. It backs the shell's file
//! commands (`ls/cat/write/rm/touch`) and stands up the "VFS core service"
//! concept from [`docs/02`]. A real copy-on-write on-disk filesystem (with
//! directories, snapshots, atomic updates) is a later Phase 1/2 build; this is
//! the interface and behavior, in memory.

use core::ptr::addr_of_mut;

pub const MAX_FILES: usize = 24;
const NAME_LEN: usize = 24;
/// Bytes a slot holds in RAM (big files, claude/vfs-big): a Nia app up to 16 KiB.
const DATA_LEN: usize = 16384;
/// What every classic caller sees of a file: `read_raw`, `read_str` and `write`
/// keep today's 512-byte view, so the shell commands, config and the editor are
/// unchanged. Only `read_full` / `write_full` (used by `nia::run_file`) and the
/// disk save and load see the whole file.
pub const MAX_FILE_BYTES: usize = 512;
/// The most a file can hold (whole-file API and disk).
pub const MAX_BIG_FILE_BYTES: usize = DATA_LEN;

#[derive(Clone, Copy)]
struct File {
    name: [u8; NAME_LEN],
    nlen: usize,
    data: [u8; DATA_LEN],
    dlen: usize,
    used: bool,
}

const EMPTY: File = File {
    name: [0; NAME_LEN],
    nlen: 0,
    data: [0; DATA_LEN],
    dlen: 0,
    used: false,
};

static mut FILES: [File; MAX_FILES] = [EMPTY; MAX_FILES];

// Last-modified time (Unix epoch seconds) per slot, parallel to FILES. Kept in a
// separate array so the on-disk File format (sync_to_disk) is unchanged. 0 = not
// written at runtime (e.g. a boot-seeded or disk-loaded file). In-RAM only.
static mut FILE_MTIME: [u32; MAX_FILES] = [0; MAX_FILES];

/// Current wall-clock epoch seconds from the PL031 RTC (for stamping writes).
fn rtc_now() -> u32 {
    // The *resolved* base: identical to the compile-time constant unless a live
    // device tree corrected it (milestone 2.726).
    let base = crate::board::resolved().rtc;
    // SAFETY: valid RTC MMIO on the active board.
    unsafe { core::ptr::read_volatile(base as *const u32) }
}

#[inline]
unsafe fn file(i: usize) -> *mut File {
    (addr_of_mut!(FILES) as *mut File).add(i)
}

fn name_matches(f: *const File, name: &str) -> bool {
    // SAFETY: f is a valid table entry pointer.
    unsafe {
        let nb = name.as_bytes();
        if (*f).nlen != nb.len() {
            return false;
        }
        for i in 0..(*f).nlen {
            if (*f).name[i] != nb[i] {
                return false;
            }
        }
        true
    }
}

fn find(name: &str) -> Option<usize> {
    // SAFETY: single-threaded shell context.
    unsafe {
        for i in 0..MAX_FILES {
            if (*file(i)).used && name_matches(file(i), name) {
                return Some(i);
            }
        }
        None
    }
}

/// Create an empty file (or return the existing one). None if the table is full
/// or the name is too long.
pub fn create(name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > NAME_LEN {
        return None;
    }
    if let Some(i) = find(name) {
        return Some(i);
    }
    // SAFETY: single-threaded.
    unsafe {
        for i in 0..MAX_FILES {
            if !(*file(i)).used {
                let f = file(i);
                (*f).used = true;
                (*f).nlen = name.len();
                (*f).dlen = 0;
                (&mut (*f).name)[..name.len()].copy_from_slice(name.as_bytes());
                return Some(i);
            }
        }
        None
    }
}

/// Write bytes to `name` (creating it if needed), truncating to MAX_FILE_BYTES
/// (512), as every classic caller expects.
pub fn write(name: &str, data: &[u8]) -> bool {
    write_capped(name, data, MAX_FILE_BYTES)
}

/// Write up to MAX_BIG_FILE_BYTES (16 KiB). Only for whole-file users.
#[allow(dead_code)]
pub fn write_full(name: &str, data: &[u8]) -> bool {
    write_capped(name, data, MAX_BIG_FILE_BYTES)
}

fn write_capped(name: &str, data: &[u8], cap: usize) -> bool {
    let i = match create(name) {
        Some(i) => i,
        None => return false,
    };
    // SAFETY: valid index from create().
    unsafe {
        let f = file(i);
        let n = data.len().min(cap).min(DATA_LEN);
        (&mut (*f).data)[..n].copy_from_slice(&data[..n]);
        (*f).dlen = n;
        // Stamp the last-modified time from the RTC (parallel array).
        addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(rtc_now());
    }
    true
}

/// Create `name` as an empty file if it does not exist, or (re)stamp its
/// last-modified time from the RTC if it does -- without touching its content.
/// Returns false only if the name is too long or the table is full.
pub fn touch(name: &str) -> bool {
    let i = match create(name) {
        Some(i) => i,
        None => return false,
    };
    // SAFETY: valid slot index from create(); parallel mtime array.
    unsafe { addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(rtc_now()) };
    true
}

/// Last-modified time (Unix epoch seconds) of `name`, or None if it doesn't
/// exist. 0 means the file has not been written at runtime (seeded/loaded).
pub fn mtime_of(name: &str) -> Option<u32> {
    let i = find(name)?;
    // SAFETY: valid index into the parallel mtime array.
    Some(unsafe { addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).read() })
}

/// Up to `cap` bytes of slot `i` (never past the slot: dlen is clamped).
fn slice_of(i: usize, cap: usize) -> &'static [u8] {
    // SAFETY: valid index; the length is clamped to DATA_LEN, so the slice stays
    // inside the 'static FILES table.
    unsafe {
        let f = file(i);
        let n = (*f).dlen.min(cap).min(DATA_LEN);
        core::slice::from_raw_parts((*f).data.as_ptr(), n)
    }
}

/// Read a file's raw bytes (files live in static storage → 'static): at most
/// the first MAX_FILE_BYTES (512), the classic view.
pub fn read_raw(name: &str) -> Option<&'static [u8]> {
    Some(slice_of(find(name)?, MAX_FILE_BYTES))
}

/// The whole file, up to MAX_BIG_FILE_BYTES. Only for whole-file users
/// (`nia::run_file`); every other caller keeps `read_raw`'s 512-byte view.
pub fn read_full(name: &str) -> Option<&'static [u8]> {
    Some(slice_of(find(name)?, MAX_BIG_FILE_BYTES))
}

/// Read a file's contents as a &str (at most the first 512 bytes).
pub fn read_str(name: &str) -> Option<&'static str> {
    core::str::from_utf8(slice_of(find(name)?, MAX_FILE_BYTES)).ok()
}

/// Delete a file. Returns false if it didn't exist.
pub fn remove(name: &str) -> bool {
    match find(name) {
        Some(i) => {
            // SAFETY: valid index.
            unsafe {
                (*file(i)).used = false;
            }
            true
        }
        None => false,
    }
}

pub fn is_used(i: usize) -> bool {
    // SAFETY: bounds-checked read.
    unsafe { i < MAX_FILES && (*file(i)).used }
}

/// The true size of slot `i` (can be over 512 for a big file).
pub fn size_of(i: usize) -> usize {
    // SAFETY: bounds-checked read.
    unsafe {
        if i < MAX_FILES {
            (*file(i)).dlen.min(DATA_LEN)
        } else {
            0
        }
    }
}

pub fn name_of(i: usize) -> &'static str {
    // SAFETY: name points into the 'static FILES table.
    unsafe {
        let f = file(i);
        core::str::from_utf8(core::slice::from_raw_parts((*f).name.as_ptr(), (*f).nlen)).unwrap_or("?")
    }
}

/// Seed a couple of files so a fresh boot has something to `ls`/`cat`.
pub fn seed() {
    write("motd", b"Welcome to AfriSovera OS -- a sovereign, capability-secured OS.");
    write(
        "about.txt",
        b"AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\nAI as an OS service. You own the device and the software on it.",
    );
}

// --- Persistence: back the VFS with a virtio-blk disk (VFS-on-disk v0). ---
// On-disk layout (512-byte sectors): sector 0 = superblock (magic + file count);
// slot i uses two sectors: metadata (used flag + name + nlen + dlen) then data.
// Minimal flat format, not a journaling FS.
//
// Big files (claude/vfs-big). The magic stays AFRIVFS1 (the PC drivers claim a
// disk by it) and sectors 0-48 keep their layout. A file over 512 bytes keeps
// dlen = 512 in its meta sector (an old kernel reads a safe 512-byte prefix) and
// its true length at meta offset 37 (META_TRUE); superblock byte 12 = 2 says the
// extension is in use. Slot i's bytes 512.. live in sectors 128 + 31*i to
// 128 + 31*i + 30 (EXT_FIRST, EXT_PER): past the disk self-test at sector 100,
// ending at 871, below the USB table backup at 2048. A disk or partition under
// 872 sectors saves big files as their first 512 bytes, with a line. With no big
// file the image is byte-identical to the old format (no flag, no length, no
// extension). Downgrade: an old kernel shows the 512-byte prefix and its next
// sync clears the flag, so big files are then cut to 512 bytes on disk.
const DISK_MAGIC: &[u8; 8] = b"AFRIVFS1";
const SB_FLAGS: usize = 12;
const SB_BIG: u8 = 2;
const META_TRUE: usize = 1 + NAME_LEN + 12;
const EXT_FIRST: u64 = 128;
const EXT_PER: u64 = 31;
const EXT_END: u64 = EXT_FIRST + EXT_PER * MAX_FILES as u64; // 872: sectors needed for big files

fn ext_sector(i: usize, k: usize) -> u64 {
    EXT_FIRST + EXT_PER * i as u64 + k as u64
}

/// Sectors of extension a file of `len` bytes uses (0 for 512 bytes or less).
fn ext_sectors(len: usize) -> usize {
    len.saturating_sub(512).div_ceil(512)
}

/// Serialize the current VFS to the disk. Returns true if written.
pub fn sync_to_disk() -> bool {
    if !crate::virtio::blk_present() {
        crate::kprintln!("[vfs] no disk attached — nothing to sync");
        return false;
    }
    // SAFETY: single-threaded shell/boot context.
    unsafe {
        // Big files first: their extension sectors go out BEFORE sectors 0-48, so
        // the PC drivers' flush at the end of the VFS area still ends the save,
        // and a cut mid-save leaves the old metadata.
        let room = crate::virtio::blk_capacity() >= EXT_END;
        let mut big = false;
        for i in 0..MAX_FILES {
            let fp = file(i);
            let len = (*fp).dlen.min(DATA_LEN);
            if !(*fp).used || len <= 512 {
                continue;
            }
            if !room {
                crate::kprintln!(
                    "[vfs] disk too small for files over 512 bytes ({} sectors, {} needed): {} saved as 512 bytes",
                    crate::virtio::blk_capacity(),
                    EXT_END,
                    name_of(i)
                );
                continue;
            }
            big = true;
            for k in 0..ext_sectors(len) {
                let mut sec = [0u8; 512];
                let from = 512 + k * 512;
                let to = (from + 512).min(len);
                sec[..to - from].copy_from_slice(&(&(*fp).data)[from..to]);
                crate::virtio::write_sector(ext_sector(i, k), &sec);
            }
        }
        let mut sb = [0u8; 512];
        sb[..8].copy_from_slice(DISK_MAGIC);
        let mut count = 0u32;
        for i in 0..MAX_FILES {
            if (*file(i)).used {
                count += 1;
            }
        }
        sb[8..12].copy_from_slice(&count.to_le_bytes());
        if big {
            sb[SB_FLAGS] = SB_BIG;
        }
        crate::virtio::write_sector(0, &sb);

        for i in 0..MAX_FILES {
            let fp = file(i);
            let mut meta = [0u8; 512];
            if (*fp).used {
                meta[0] = 1;
                meta[1..1 + NAME_LEN].copy_from_slice(&(*fp).name);
                let o = 1 + NAME_LEN;
                let len = (*fp).dlen.min(DATA_LEN);
                meta[o..o + 4].copy_from_slice(&((*fp).nlen as u32).to_le_bytes());
                meta[o + 4..o + 8].copy_from_slice(&(len.min(512) as u32).to_le_bytes());
                if big && len > 512 {
                    meta[META_TRUE..META_TRUE + 4].copy_from_slice(&(len as u32).to_le_bytes());
                }
                // mtime lives in the free space of this slot's meta sector, past
                // the existing fields, so no offsets shift and an old image (zero
                // there) loads back as mtime 0 = "unknown".
                let mt = addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).read();
                meta[o + 8..o + 12].copy_from_slice(&mt.to_le_bytes());
            }
            crate::virtio::write_sector(1 + (i as u64) * 2, &meta);
            let mut data = [0u8; 512];
            data.copy_from_slice(&(&(*fp).data)[..512]);
            crate::virtio::write_sector(1 + (i as u64) * 2 + 1, &data);
        }
        crate::kprintln!("[vfs] synced {} files to disk", count);
        true
    }
}

/// Restore the VFS from disk if a valid filesystem is present. Returns true if
/// restored (replacing the in-RAM contents).
pub fn load_from_disk() -> bool {
    if !crate::virtio::blk_present() {
        return false;
    }
    // SAFETY: single-threaded boot context.
    unsafe {
        let mut sb = [0u8; 512];
        if !crate::virtio::read_sector(0, &mut sb) || &sb[..8] != DISK_MAGIC {
            return false; // blank/foreign disk — keep the in-RAM VFS
        }
        let flagged = sb[SB_FLAGS] == SB_BIG;
        let room = crate::virtio::blk_capacity() >= EXT_END;
        let mut restored = 0u32;
        for i in 0..MAX_FILES {
            let mut meta = [0u8; 512];
            crate::virtio::read_sector(1 + (i as u64) * 2, &mut meta);
            let fp = file(i);
            if meta[0] == 1 {
                (*fp).used = true;
                (*fp).name.copy_from_slice(&meta[1..1 + NAME_LEN]);
                let o = 1 + NAME_LEN;
                // Clamp everything read from the disk: a damaged image must
                // never give a slice past the slot.
                (*fp).nlen = (u32::from_le_bytes([meta[o], meta[o + 1], meta[o + 2], meta[o + 3]]) as usize)
                    .min(NAME_LEN);
                (*fp).dlen = (u32::from_le_bytes([meta[o + 4], meta[o + 5], meta[o + 6], meta[o + 7]]) as usize)
                    .min(512);
                let t = &meta[META_TRUE..META_TRUE + 4];
                let truelen = (u32::from_le_bytes([t[0], t[1], t[2], t[3]]) as usize).min(DATA_LEN);
                let mt = u32::from_le_bytes([meta[o + 8], meta[o + 9], meta[o + 10], meta[o + 11]]);
                addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(mt);
                let mut data = [0u8; 512];
                crate::virtio::read_sector(1 + (i as u64) * 2 + 1, &mut data);
                (&mut (*fp).data)[..512].copy_from_slice(&data);
                if flagged && truelen > 512 {
                    // The extension: only with the flag, only within the disk.
                    let mut whole = room;
                    for k in 0..ext_sectors(truelen) {
                        if !whole {
                            break;
                        }
                        let mut sec = [0u8; 512];
                        whole = crate::virtio::read_sector(ext_sector(i, k), &mut sec);
                        let from = 512 + k * 512;
                        let to = (from + 512).min(truelen);
                        (&mut (*fp).data)[from..to].copy_from_slice(&sec[..to - from]);
                    }
                    if whole {
                        (*fp).dlen = truelen;
                    } else {
                        crate::kprintln!(
                            "[vfs] {}: the part past 512 bytes could not be read; keeping the first 512 bytes",
                            name_of(i)
                        );
                    }
                }
                restored += 1;
            } else {
                (*fp).used = false;
                addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(0);
            }
        }
        crate::kprintln!("[vfs] VFS restored from disk: {} files", restored);
        true
    }
}
