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
const DATA_LEN: usize = 512;
/// Maximum bytes of content per file (the data cap). Exposed for `df`.
pub const MAX_FILE_BYTES: usize = DATA_LEN;

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

/// Write bytes to `name` (creating it if needed), truncating to the file cap.
pub fn write(name: &str, data: &[u8]) -> bool {
    let i = match create(name) {
        Some(i) => i,
        None => return false,
    };
    // SAFETY: valid index from create().
    unsafe {
        let f = file(i);
        let n = data.len().min(DATA_LEN);
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

/// Read a file's raw bytes (files live in static storage → 'static). Used to
/// load binary content such as a `.wasm` module.
pub fn read_raw(name: &str) -> Option<&'static [u8]> {
    let i = find(name)?;
    // SAFETY: valid index; slice points into the 'static FILES table.
    unsafe {
        let f = file(i);
        Some(core::slice::from_raw_parts((*f).data.as_ptr(), (*f).dlen))
    }
}

/// Read a file's contents as a &str (files live in static storage → 'static).
pub fn read_str(name: &str) -> Option<&'static str> {
    let i = find(name)?;
    // SAFETY: valid index; slice points into the 'static FILES table.
    unsafe {
        let f = file(i);
        let s = core::slice::from_raw_parts((*f).data.as_ptr(), (*f).dlen);
        core::str::from_utf8(s).ok()
    }
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

pub fn size_of(i: usize) -> usize {
    // SAFETY: bounds-checked read.
    unsafe {
        if i < MAX_FILES {
            (*file(i)).dlen
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
const DISK_MAGIC: &[u8; 8] = b"AFRIVFS1";

/// Serialize the current VFS to the disk. Returns true if written.
pub fn sync_to_disk() -> bool {
    if !crate::virtio::blk_present() {
        crate::kprintln!("[vfs] no disk attached — nothing to sync");
        return false;
    }
    // SAFETY: single-threaded shell/boot context.
    unsafe {
        let mut sb = [0u8; 512];
        sb[..8].copy_from_slice(DISK_MAGIC);
        let mut count = 0u32;
        for i in 0..MAX_FILES {
            if (*file(i)).used {
                count += 1;
            }
        }
        sb[8..12].copy_from_slice(&count.to_le_bytes());
        crate::virtio::write_sector(0, &sb);

        for i in 0..MAX_FILES {
            let fp = file(i);
            let mut meta = [0u8; 512];
            if (*fp).used {
                meta[0] = 1;
                meta[1..1 + NAME_LEN].copy_from_slice(&(*fp).name);
                let o = 1 + NAME_LEN;
                meta[o..o + 4].copy_from_slice(&((*fp).nlen as u32).to_le_bytes());
                meta[o + 4..o + 8].copy_from_slice(&((*fp).dlen as u32).to_le_bytes());
                // mtime lives in the free space of this slot's meta sector, past
                // the existing fields, so no offsets shift and an old image (zero
                // there) loads back as mtime 0 = "unknown".
                let mt = addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).read();
                meta[o + 8..o + 12].copy_from_slice(&mt.to_le_bytes());
            }
            crate::virtio::write_sector(1 + (i as u64) * 2, &meta);
            let mut data = [0u8; 512];
            data.copy_from_slice(&(*fp).data);
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
        let mut restored = 0u32;
        for i in 0..MAX_FILES {
            let mut meta = [0u8; 512];
            crate::virtio::read_sector(1 + (i as u64) * 2, &mut meta);
            let fp = file(i);
            if meta[0] == 1 {
                (*fp).used = true;
                (*fp).name.copy_from_slice(&meta[1..1 + NAME_LEN]);
                let o = 1 + NAME_LEN;
                (*fp).nlen = u32::from_le_bytes([meta[o], meta[o + 1], meta[o + 2], meta[o + 3]]) as usize;
                (*fp).dlen =
                    u32::from_le_bytes([meta[o + 4], meta[o + 5], meta[o + 6], meta[o + 7]]) as usize;
                let mt = u32::from_le_bytes([meta[o + 8], meta[o + 9], meta[o + 10], meta[o + 11]]);
                addr_of_mut!(FILE_MTIME).cast::<u32>().add(i).write(mt);
                let mut data = [0u8; 512];
                crate::virtio::read_sector(1 + (i as u64) * 2 + 1, &mut data);
                (*fp).data.copy_from_slice(&data);
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
