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

// A2 (the architect's change 3): a file whose checksum did not match at load (a save cut part way). Its bytes are
// kept and it is named on the console, in `ls`, and to an app that reads it; a fresh write clears the mark.
static mut FILE_DAMAGED: [bool; MAX_FILES] = [false; MAX_FILES];

/// Whether `name` was found damaged at load (a cut during a save) and has not been written since.
pub fn is_damaged(name: &str) -> bool {
    // SAFETY: a scalar read; find gives a valid index.
    find(name).map_or(false, |i| unsafe { addr_of_mut!(FILE_DAMAGED).cast::<bool>().add(i).read() })
}

fn set_damaged(i: usize, v: bool) {
    // SAFETY: i < MAX_FILES (callers index the table).
    unsafe { addr_of_mut!(FILE_DAMAGED).cast::<bool>().add(i).write(v) }
}

/// CRC-32 (IEEE, reflected, 0xEDB88320), continued from `crc` (start with 0xFFFF_FFFF, finish with `!`).
fn crc32_more(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    crc
}

/// A file's checksum on disk: its name, its length and every byte of it.
fn file_crc(name: &[u8], data: &[u8]) -> u32 {
    let c = crc32_more(0xFFFF_FFFF, name);
    let c = crc32_more(c, &(data.len() as u32).to_le_bytes());
    !crc32_more(c, data)
}

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
        set_damaged(i, false); // a fresh write: whatever a cut save left is replaced
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
            set_damaged(i, false);
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
// A2 (the architect's change 3, 14 Nov 2026): format version 2 adds checksums. Each file's meta sector holds the
// CRC-32 of its name, length and bytes at META_CRC, and META_SUM_MARK there says the CRC is present; the
// superblock holds the version at SB_VER and a CRC-32 of its first 16 bytes at SB_CRC. An older image has zeros
// there: it loads as before, unchecked. An older kernel ignores these bytes, and its next save writes none.
const META_CRC: usize = META_TRUE + 4; // 41
const META_SUM: usize = META_CRC + 4; // 45
const META_SUM_MARK: u8 = 0xC1;
/// A file found damaged keeps this mark through later saves (every boot's full sync included), so the damage is
/// never written over with a fresh checksum of the mixed bytes; a fresh write of the file clears it.
const META_DAMAGED_MARK: u8 = 0xDA;
const SB_VER: usize = 13;
const SB_VERSION: u8 = 2;
const SB_CRC: usize = 16;
/// The VFS's last sector: the PC disk drivers end a save's gathered writes and flush the disk's cache on it.
const VFS_LAST: u64 = 48;
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

/// Whether any file needs the extension (a file over 512 bytes) and the disk has room for it.
unsafe fn big_in_use(room: bool) -> bool {
    room && (0..MAX_FILES).any(|i| (*file(i)).used && (*file(i)).dlen.min(DATA_LEN) > 512)
}

/// Write slot `i`'s extension sectors (a file over 512 bytes, when there is room), then its first 512 bytes, then
/// its meta sector with the checksum last, so a cut before the meta leaves the old checksum and the file is found
/// damaged at load, never silently mixed. Returns false when a sector write failed.
unsafe fn write_slot(i: usize, big: bool) -> bool {
    let fp = file(i);
    let len = (*fp).dlen.min(DATA_LEN);
    let mut ok = true;
    if (*fp).used && big && len > 512 {
        for k in 0..ext_sectors(len) {
            let mut sec = [0u8; 512];
            let from = 512 + k * 512;
            let to = (from + 512).min(len);
            sec[..to - from].copy_from_slice(&(&(*fp).data)[from..to]);
            ok &= crate::virtio::write_sector(ext_sector(i, k), &sec);
        }
    }
    ok &= write_first(i);
    ok &= write_meta(i, big);
    ok
}

/// Slot i's first 512 bytes (sector 2 + 2i).
unsafe fn write_first(i: usize) -> bool {
    let fp = file(i);
    let mut data = [0u8; 512];
    data.copy_from_slice(&(&(*fp).data)[..512]);
    crate::virtio::write_sector(1 + (i as u64) * 2 + 1, &data)
}

/// The full sync's order for slot i, sector by sector as before: meta (1 + 2i), then the first 512 bytes (2 + 2i).
unsafe fn write_meta_and_first(i: usize, big: bool) -> bool {
    write_meta(i, big) & write_first(i)
}

/// Slot i's meta sector, with the file's checksum (format version 2).
unsafe fn write_meta(i: usize, big: bool) -> bool {
    let fp = file(i);
    let len = (*fp).dlen.min(DATA_LEN);
    let mut meta = [0u8; 512];
    if (*fp).used {
        meta[0] = 1;
        meta[1..1 + NAME_LEN].copy_from_slice(&(*fp).name);
        let o = 1 + NAME_LEN;
        let kept = if big { len } else { len.min(512) };
        meta[o..o + 4].copy_from_slice(&((*fp).nlen as u32).to_le_bytes());
        meta[o + 4..o + 8].copy_from_slice(&(len.min(512) as u32).to_le_bytes());
        if big && len > 512 {
            meta[META_TRUE..META_TRUE + 4].copy_from_slice(&(len as u32).to_le_bytes());
        }
        // mtime lives in the free space of this slot's meta sector, past the existing fields, so no offsets shift
        // and an old image (zero there) loads back as mtime 0 = "unknown".
        let mt = addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).read();
        meta[o + 8..o + 12].copy_from_slice(&mt.to_le_bytes());
        let nm = &(&(*fp).name)[..(*fp).nlen.min(NAME_LEN)];
        meta[META_CRC..META_CRC + 4].copy_from_slice(&file_crc(nm, &(&(*fp).data)[..kept]).to_le_bytes());
        let damaged = addr_of_mut!(FILE_DAMAGED).cast::<bool>().add(i).read();
        meta[META_SUM] = if damaged { META_DAMAGED_MARK } else { META_SUM_MARK };
    }
    crate::virtio::write_sector(1 + (i as u64) * 2, &meta)
}

/// The superblock: magic, file count, the big flag, the format version and its own checksum.
unsafe fn write_superblock(big: bool) -> bool {
    let mut sb = [0u8; 512];
    sb[..8].copy_from_slice(DISK_MAGIC);
    let count = (0..MAX_FILES).filter(|&i| (*file(i)).used).count() as u32;
    sb[8..12].copy_from_slice(&count.to_le_bytes());
    if big {
        sb[SB_FLAGS] = SB_BIG;
    }
    sb[SB_VER] = SB_VERSION;
    let c = !crc32_more(0xFFFF_FFFF, &sb[..SB_CRC]);
    sb[SB_CRC..SB_CRC + 4].copy_from_slice(&c.to_le_bytes());
    crate::virtio::write_sector(0, &sb)
}

/// Save one file (A2: an app's file_write), not the whole VFS: its extension, its first sector, its meta (with the
/// checksum), then the superblock, then the VFS's last sector read and written back unchanged, so the PC drivers
/// end the save and flush the disk's cache as after a full sync. Returns false with no disk, no such file, or a
/// failed sector write. A disk too small for big files gets the full sync's rule (the first 512 bytes, a line).
pub fn sync_file(name: &str) -> bool {
    if !crate::virtio::blk_present() {
        return false;
    }
    let Some(i) = find(name) else { return false };
    // SAFETY: single-threaded shell or interpreter context.
    unsafe {
        let room = crate::virtio::blk_capacity() >= EXT_END;
        if !room && (*file(i)).dlen > 512 {
            crate::kprintln!("[vfs] disk too small for files over 512 bytes: {} saved as 512 bytes", name);
        }
        let big = big_in_use(room);
        let mut ok = write_slot(i, big) && write_superblock(big);
        let mut last = [0u8; 512];
        ok &= crate::virtio::read_sector(VFS_LAST, &mut last) && crate::virtio::write_sector(VFS_LAST, &last);
        ok
    }
}

/// Serialize the current VFS to the disk. Returns true if written.
pub fn sync_to_disk() -> bool {
    if !crate::virtio::blk_present() {
        crate::kprintln!("[vfs] no disk attached — nothing to sync");
        return false;
    }
    // SAFETY: single-threaded shell/boot context.
    unsafe {
        // Big files first: their extension sectors go out BEFORE sectors 0-48, so the PC drivers' flush at the end
        // of the VFS area still ends the save, and a cut mid-save leaves the old metadata (and, since A2, the old
        // checksum, so the file is found damaged rather than silently mixed).
        let room = crate::virtio::blk_capacity() >= EXT_END;
        for i in 0..MAX_FILES {
            if (*file(i)).used && (*file(i)).dlen.min(DATA_LEN) > 512 && !room {
                crate::kprintln!(
                    "[vfs] disk too small for files over 512 bytes ({} sectors, {} needed): {} saved as 512 bytes",
                    crate::virtio::blk_capacity(),
                    EXT_END,
                    name_of(i)
                );
            }
        }
        let big = big_in_use(room);
        // the extensions first (write_slot writes a slot's extension, then its sectors 1 + 2i and 2 + 2i: here the
        // extensions of every slot go out before sector 0, as before)
        for i in 0..MAX_FILES {
            let fp = file(i);
            let len = (*fp).dlen.min(DATA_LEN);
            if (*fp).used && big && len > 512 {
                for k in 0..ext_sectors(len) {
                    let mut sec = [0u8; 512];
                    let from = 512 + k * 512;
                    let to = (from + 512).min(len);
                    sec[..to - from].copy_from_slice(&(&(*fp).data)[from..to]);
                    crate::virtio::write_sector(ext_sector(i, k), &sec);
                }
            }
        }
        write_superblock(big);
        let count = (0..MAX_FILES).filter(|&i| (*file(i)).used).count();
        for i in 0..MAX_FILES {
            write_meta_and_first(i, big);
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
        if sb[SB_VER] == SB_VERSION {
            let c = !crc32_more(0xFFFF_FFFF, &sb[..SB_CRC]);
            if sb[SB_CRC..SB_CRC + 4] != c.to_le_bytes() {
                crate::kprintln!("[vfs] the disk's superblock checksum does not match; each file is still checked by its own");
            }
        }
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
                // A2: a file saved with a checksum must match it, else a save was cut part way. Keep its bytes, name
                // it, and mark it damaged (ls, file_read); never pass a mix of old and new off as the file.
                let marked = meta[META_SUM] == META_DAMAGED_MARK;
                let damaged = marked || meta[META_SUM] == META_SUM_MARK && {
                    let want = u32::from_le_bytes([meta[META_CRC], meta[META_CRC + 1], meta[META_CRC + 2], meta[META_CRC + 3]]);
                    let nm = &(&(*fp).name)[..(*fp).nlen];
                    file_crc(nm, &(&(*fp).data)[..(*fp).dlen]) != want
                };
                set_damaged(i, damaged);
                if marked {
                    crate::kprintln!("[vfs] {} is still marked damaged (a cut during an earlier save)", name_of(i));
                } else if damaged {
                    crate::kprintln!(
                        "[vfs] {} was damaged by a cut during a save; its bytes are kept as {} and marked damaged",
                        name_of(i),
                        name_of(i)
                    );
                }
                restored += 1;
            } else {
                set_damaged(i, false);
                (*fp).used = false;
                addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(0);
            }
        }
        crate::kprintln!("[vfs] VFS restored from disk: {} files", restored);
        true
    }
}
