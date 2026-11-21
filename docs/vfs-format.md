# The VFS on disk: the AFRIVFS1 format

This is the disk format of AfriSovera's small file system (`kernel/src/vfs.rs`), the same on the phone (ARM) and
PC kernels. The code comment above `DISK_MAGIC` in `vfs.rs` is the short version; this page is the long one.
Written 11 Nov 2026, after files up to 16 KiB were merged (`490de519`).

## In memory

24 files (slots). A name is up to 24 bytes. A file holds up to 16,384 bytes. Every classic caller (shell
commands, config, the editor) sees at most the first 512 bytes through `read_raw`, `read_str` and `write`. Only
`read_full`/`write_full` see the whole file, and today only `nia::run_file` uses `read_full`, so `nia NAME.wasm`
runs apps up to 16 KiB.

## On disk (512-byte sectors)

| Sectors | What |
|---|---|
| 0 | superblock: `AFRIVFS1` (bytes 0-7), file count (u32, bytes 8-11), byte 12 = 2 when a file over 512 bytes is saved, else 0 |
| 1 + 2i | slot i's meta: used (byte 0), name (1-24), name length (u32 at 25), length up to 512 (u32 at 29), mtime (u32 at 33), true length (u32 at 37, only for a file over 512 bytes) |
| 2 + 2i | slot i's bytes 0-511 |
| 49-127 | free; sector 100 is the disk self-test (`blk_selftest`) |
| 128 + 31i to 128 + 31i + 30 | slot i's bytes 512 onward (only the sectors a file uses are written) |
| 872 and up | not used by the VFS (a USB stick keeps its partition-table backup at 2048) |

- The magic stays `AFRIVFS1`: the PC disk drivers use it to recognise a disk as AfriSovera's.
- A disk with only small files is exactly the old format: no flag, no true length, nothing at 128-871.
- A disk or partition under 872 sectors saves big files as their first 512 bytes and prints a line.
- `sync` writes the extension sectors first, then sectors 0-48, so the PC drivers' cache flush at the end of the
  VFS area still ends the save.
- Load clamps every length it reads (name 24, length 512, true length 16,384) and reads the extension only with
  the flag and within the disk. If the extension cannot be read, the file keeps its first 512 bytes and a line
  says so.

## Version 2: checksums (A2, 14 Nov 2026)

A save cut part way (power, a crash) could leave a big file's old metadata over part-new data, a file that was
neither the old one nor the new one, with nothing to say so. Since A2 every save writes checksums:

| Where | What |
|---|---|
| sector 0, byte 13 | the format version: 2 |
| sector 0, bytes 16-19 | CRC-32 (IEEE) of bytes 0-15 |
| meta sector, bytes 41-44 | CRC-32 of the file's name, its length (u32) and every byte of it |
| meta sector, byte 45 | 0xC1: the checksum is there |

- A save writes a file's extension sectors, then its first 512 bytes, then its meta sector (the checksum) last. An
  app's `file_write` saves only its file this way, then the superblock, then reads and writes back sector 48 so
  the PC drivers end the save and flush the disk's cache as after a full `sync`.
- At load, a file whose checksum does not match is kept, named on the console (`[vfs] NAME was damaged by a cut
  during a save; ...`), marked in `ls`, and `file_read` answers -5 for it. A fresh write clears the mark.
- An image without the marker (version 1, or made by `mkvfs-big.py` or nia-apps' `mkvfs.py`) loads as before,
  unchecked. An older kernel ignores the new bytes; its next save writes none, and this kernel then loads that
  image unchecked. A disk with only small files is no longer byte-identical to version 1 (the checksums).
- True "old or new, never in between" is the future file system's job (AFR-0011, copy-on-write). Version 2 makes
  a cut save visible; it does not undo it.

## Downgrade

An older kernel (before `490de519`) on a disk with big files shows each big file's first 512 bytes. Its next
`sync` writes the superblock without the flag, so the big files are then cut to 512 bytes on disk. Nothing past a
slot is ever read.

## Known limits (architect's notes, 11 Nov 2026; not fixed yet)

- **A cut-off save can mix old and new.** If a big file is rewritten and the save is cut off part way (power,
  crash), the old metadata can be paired with part-new extension data. The VFS already has the same kind of torn
  write inside one slot, so this is not new. The real fix (two copies of the metadata, or a checksum) belongs
  with the future file system, not this format.
- **Failed sector writes are not reported.** `sync` does not check the result of each sector write or read,
  neither for the extension nor for sectors 0-48, so a failed write goes unnoticed. To do when the VFS gets error
  reporting.

## Tools

- `scripts/lib/mkvfs-big.py` makes and checks images in this format (used by the `persist-big` suite).
- nia-apps `tools/mkvfs.py` writes the same format for the apps' disk image.
