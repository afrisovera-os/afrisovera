# AFR-0011: VFS v1, the AfriSovera file system (format 3, `AFRIVFS2`)

| | |
|---|---|
| **Status** | **Designed** (§4 to §11). Format 1 and 2 (`AFRIVFS1`, §12) **Implemented** in `kernel/src/vfs.rs` |
| **Version** | 0.1 (2026-11-14; describes format 1 at origin/master 64546242 and format 2 at `claude/nia-a2` ddbcc053) |
| **Editor** | Storage engineer, reviewed by the chief architect (ARCHITECT_REVIEWS.md, 14 Nov 2026, "AFR-0011, VFS v1") |
| **Code** | `kernel/src/vfs.rs` (formats 1 and 2); `afrifs/` (format 3, step V1, not written yet) |
| **Suites** | today: `persist`, `persist-big`, `wasm-file16`; designed: §14 |
| **Vectors** | `docs/spec/vectors/afr0011_vectors.py` (no dependencies; a fresh 8 MiB volume byte for byte) |
| **Depends on** | AFR-0001 §7.4 (rights CREATE, LIST), §7.2 (file errors), §7.9 (ring-3 file calls, step V6); AFR-0003 (`file` grant); AFR-0008 §10 (a new format takes a new magic); AFR-0010 §9 (the data format rule) |
| **Licence** | Apache-2.0 |

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119.

---

## 1. Motivation

AfriSovera keeps people's work. Today's file system (format 1 and 2, §12) holds 24 files of at most 16 KiB, has no
folders, rewrites files in place, and lets every app with `file` see every file. A save cut by a power failure can
leave a file that is neither the old one nor the new one; format 2 detects that but cannot undo it.

Format 3 is a copy-on-write file system: a power cut leaves the old state or the new one, never half. It has folders,
files up to 4 GiB, a checksum on every block, a private folder per app, and it reads the old formats and can go back
to them.

## 2. Scope and status summary

| Part | Status | Where |
|---|---|---|
| Format 1 (`AFRIVFS1`, flat, 512 bytes, extension to 16 KiB) | Implemented | `kernel/src/vfs.rs`, `docs/vfs-format.md` |
| Format 2 (format 1 plus checksums) | Implemented on `claude/nia-a2` | same |
| Format 3 on-disk layout (§4 to §6) | Designed | `afrifs/` (V1) |
| Commit and power-cut rules (§7, §8) | Designed | `afrifs/` (V1) |
| Block layer flush (§9) | Designed | `kernel/src/pc/disk.rs`, each driver, `kernel/src/virtio.rs` (V2) |
| Migration and going back (§10) | Designed | `kernel/src/vfs.rs` (V4) |
| Folders per app, handles, rights (§11) | Designed | `kernel/src/vfs.rs`, `kernel/src/nia.rs` (V5), AFR-0001 §7.9 (V6) |

**Not in scope of format 3 v1:** symbolic links (a later plan, §15), hard links, file locks, extended attributes,
owners and the Unix mode beyond one executable flag, shared writable mappings, encryption at rest (a later incompat
feature, §13), case-insensitive folders (a ro-compat feature reserved for Windows W1, §13), repair of a damaged volume
(`vfscheck fix`, later).

## 3. Terminology

- **Partition**: AfriSovera's storage area as the disk drivers give it: a GPT partition, or a whole disk. Sector 0 is
  its first 512-byte sector. All sector numbers here are relative to it.
- **Area**: the part of the partition that format 3 allocates from, in 4 KiB **blocks** numbered from 0.
- **Commit**: one atomic change of the whole tree (§7). **Generation**: the commit's number, starting at 1.
- **Pointer**: a 16-byte reference to a block, carrying that block's checksum (§5.1).
- **Inode**: a file's or folder's 256-byte record (§5.3). **Inode number**: its index, never reused (§5.4).
- **Handle**: a capability naming one folder or file with a set of rights (§11).
- All integers are little-endian. **CRC-32C** is the Castagnoli CRC (reflected polynomial 0x82F63B78, initial value
  and final XOR 0xFFFFFFFF); its check value for the ASCII bytes `123456789` is 0xE3069283. (Format 2 uses CRC-32
  IEEE; format 3 does not.)

## 4. Layout (normative, Designed)

| Sectors | Contents |
|---|---|
| 0 | the label (§4.1) |
| 1 to 871 | format 1 or 2's sectors. After a migration (§10) they MUST be left exactly as they were. On a volume formatted as format 3 they are zero. A format 3 writer MUST NOT write them, except `vfsdowngrade` (§10.3). |
| 872 to 4095 | not used. (A USB stick keeps its partition-table backup at 2048.) |
| 4096 to the end | the area: block b is sectors 4096 + 8b to 4096 + 8b + 7 |

- A partition under 16,384 sectors (8 MiB) MUST NOT be formatted as format 3. It stays format 1 or 2, and the kernel
  says so in one line.
- The area's length in blocks is `floor((partition sectors - 4096) / 8)`, fixed at format. A partition that grows
  later does not grow the area in v1.

### 4.1 The label (sector 0)

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 8 | magic | `AFRIVFS2` |
| 8 | 2 | format | 3 |
| 10 | 2 | block size | 4096 |
| 12 | 4 | reserved | 0 |
| 16 | 8 | area start | 4096 (sectors) |
| 24 | 8 | area blocks | as §4 |
| 32 | 16 | volume id | random at format |
| 48 | 4 | compat features | §13 |
| 52 | 4 | read-only-compat features | §13 |
| 56 | 4 | incompat features | §13 |
| 60 | 2 | fold table version | 0 (§13, W1) |
| 62 | 66 | reserved | 0 |
| 128 | 64 | old superblock | after a migration: bytes 0 to 63 of the format 1 or 2 sector 0 it replaced; else 0 |
| 192 | 1 | migrated | 1 after a migration, else 0 |
| 193 | 315 | reserved | 0 |
| 508 | 4 | checksum | CRC-32C of bytes 0 to 507 |

- The label is written once: at format, or as the last step of a migration (§10.1). It MUST NOT be rewritten while
  the volume is format 3, except to set a feature bit (§13), and then only as its own commit-like step: write,
  flush, read back.
- A reader MUST refuse to mount (and write nothing) when the magic, format, block size, area start or checksum is
  wrong, when an incompat bit it does not know is set, or when the area is larger than the partition.

### 4.2 Root records (area blocks 0 and 1)

Each root record is the first 128 bytes of its block. The rest of the block is zero and MUST be ignored. Keeping the
record inside the block's first 512-byte sector means a disk that tears a 4 KiB write into sectors either wrote the
record's sector or did not.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `AFRIROOT` |
| 8 | 8 | generation |
| 16 | 16 | pointer to the inode tree's top block |
| 32 | 16 | pointer to the free map's top block |
| 48 | 1 | inode tree height (0 to 3) |
| 49 | 1 | free map height (0 to 2) |
| 50 | 6 | reserved, 0 |
| 56 | 8 | blocks in use |
| 64 | 8 | inodes in use |
| 72 | 8 | next inode number |
| 80 | 8 | commit time (ns since 1970-01-01 UTC) |
| 88 | 12 | compat, read-only-compat, incompat features (u32 each), equal to the label's when written |
| 100 | 24 | reserved, 0 |
| 124 | 4 | CRC-32C of bytes 0 to 123 |

A root record is **valid** when its magic and checksum match, its two pointers name blocks inside the area other than
0 and 1, its heights are in range, and it sets no incompat bit the reader does not know. Mount uses the valid record
with the higher generation.

## 5. Structures (normative, Designed)

### 5.1 Block pointer (16 bytes)

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | block number in the area. 0 is the null pointer (block 0 is a root record and is never pointed at). |
| 8 | 4 | CRC-32C of the whole 4 KiB block |
| 12 | 4 | low 32 bits of the generation that wrote the block |

Every block reached through a pointer MUST be checked against the pointer's CRC before any byte of it is used. A
pointer naming block 1, or a block at or past the area's end, is damage (§7.2).

### 5.2 Pointer blocks

A pointer block is 256 pointers. Trees in format 3 are radix trees of pointer blocks with data blocks at the bottom.
A tree of height h over leaf index i: at each level from the top, the slot is `(i >> (8 * level)) & 255`. Height 0
means the top pointer names the single leaf.

### 5.3 Inode (256 bytes, 16 per inode block)

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | kind: 0 free, 1 file, 2 folder (3 reserved for links, §15) |
| 1 | 1 | flags: bit 0 read-only, bit 1 hidden, bit 2 executable, bit 3 case-fold (folders, §13), bit 4 damaged (§10.1); others 0 |
| 2 | 2 | reserved, 0 |
| 4 | 4 | owner: 0 the person, 1 the system, 2 and up an app's index in the app table (§11.5) |
| 8 | 8 | size in bytes |
| 16 | 8 | inode number (MUST equal the inode's own index; a mismatch is damage) |
| 24 | 8 | created, ns since 1970 UTC |
| 32 | 8 | modified, ns since 1970 UTC |
| 40 | 4 | link count: 1 |
| 44 | 4 | reserved, 0 |
| 48 | 128 | direct pointers 0 to 7 (file blocks 0 to 7) |
| 176 | 16 | single-indirect pointer: a pointer block for file blocks 8 to 263 |
| 192 | 16 | double-indirect pointer: file blocks 264 to 65,799 |
| 208 | 16 | triple-indirect pointer: the rest (the tree allows 64 GiB) |
| 224 | 32 | reserved, 0 |

- A file MUST NOT exceed 4 GiB (4,294,967,296 bytes) in v1. A write past it is refused with `TOO_BIG` (§11.6).
- A null pointer inside a file's size is a hole and reads as zeros. A writer MAY leave holes: `set_size` growing a
  file, or a write that starts past the end.
- Times come from the RTC (seconds) plus the monotonic timer (the part below a second).

### 5.4 The inode tree

Leaf i of the inode tree is the inode block holding inodes 16i to 16i + 15. Inode 0 is never used and MUST be all
zero. Inode 1 is the top folder `/`. The next inode number in the root record only grows, so an inode number is never
reused while the volume lives. A writer MUST NOT hand out an inode number of 2^28 (268,435,456) or more: that would need an
inode tree above height 3. Such a volume is full of inodes and refuses a create with `NO_SPACE`.

**Running out of inode numbers.** Numbers are never reused, so the limit counts every create over the volume's
whole life, not the files that exist now. That is generous for documents but not for build-heavy use: git, compilers
and package tools under a compatibility bridge create and delete temporary files by the hundred thousand, and such a
volume could reach 2^28 creates in months. Then a disk with free space would refuse every new file. So:
1. **It is never a surprise.** `fs_info` reports the inode numbers left, and the desk warns once the volume has used
   90% of them.
2. **The way out is reserved in the format now:** incompat feature bit 1, `inode-reuse` (§13). It allows an inode
   tree of height 4, or the reuse of numbers freed before a generation recorded in the root record. Either lets a
   format 3 volume be upgraded in place rather than made again.
3. **Until that feature exists, the recovery is `vfscompact`:** offline, it renumbers every inode into a fresh tree
   as one copy-on-write commit like any other (§8.1), so a cut leaves the old tree. It comes as its own small plan
   when the first volume's counter passes 50%.

### 5.5 Folders

A folder's data is a list of records in its data blocks:

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | inode number; 0 means the record is unused space |
| 8 | 1 | kind (as the inode's) |
| 9 | 1 | name length, 1 to 255 |
| 10 | 2 | record length: 12 + name length rounded up to 4, or more |
| 12 | n | name |

- Records MUST NOT cross a block. The last record of a block runs to the block's end. An empty block is one unused
  record of length 4096. A folder's size is its blocks times 4096.
- A name is 1 to 255 bytes, MUST NOT contain `/` or the byte 0, and MUST NOT be `.` or `..`. Names are bytes:
  compared exactly, case-sensitive (except in a case-fold folder, §13), UTF-8 by convention but not checked.
- Two records in one folder MUST NOT have the same name.
- Order is the order of insertion; a reader MUST NOT depend on it.
- A path is at most 4,096 bytes and 64 folders deep. A walk MUST stop with `NAME_TOO_LONG` at 4,097 bytes.

### 5.6 The free map

The free map's leaves are bitmap blocks; leaf i covers area blocks 32,768i to 32,768i + 32,767. Bit `b % 8` of byte
`(b % 32768) / 8` is 1 when block b is in use. Blocks 0 and 1 are always in use. Bits for blocks at or past the area's
end MUST be 1. Height 0 covers 128 MiB, height 1 covers 32 GiB, height 2 covers 8 TiB.

### 5.7 A fresh volume

Format writes, lowest free block first: block 2 the inode block (inodes 0 to 15), block 3 the bitmap, block 4 the top
folder's data. The top folder holds `system` (inode 2), `apps` (3), `home` (4) and `tmp` (5), each an empty folder;
`home` is owned by the person (0), the others by the system (1). Root record A is generation 1 with 5 blocks and 5
inodes in use and next inode 6; root record B is all zero. Vector 3 is this volume, byte for byte.

## 6. Limits (normative)

| What | Limit |
|---|---|
| file size | 4 GiB |
| name | 255 bytes |
| path | 4,096 bytes, 64 folders |
| area | 8 TiB (the free map's height 2); a bigger partition is formatted to its first 8 TiB |
| inodes | 2^28 over the volume's life (numbers are never reused) |
| one folder | no limit of its own; lookup is a scan (measured with 10,000 entries, §14) |

## 7. Mount and checks (normative, Designed)

### 7.1 Mount

1. Read and check the label (§4.1). Any failure: refuse, write nothing.
2. Read both root records; take the valid one with the higher generation (§4.2). Neither valid: refuse, write
   nothing, and say `[vfs] the file system's root records are both damaged; the disk is left untouched`. The VFS
   then runs in RAM only.
3. Read the inode tree's top block and the free map's top block through their pointers. A bad one: refuse as in 2.
4. Mount MUST NOT write. The first write happens at the first commit.

When the newer root record is invalid and the older one is valid, mount uses the older one and says so in one line.
That is the expected result of a power cut during step 3 of a commit (§8.1).

### 7.2 Reading

Every block is checked against its pointer when it is read (§5.1). A mismatch is **damage**:
- in a file's data or its pointer blocks: that file's reads fail with `DAMAGED`; the file is named once on the
  console; other files are unaffected;
- in a folder's blocks: that folder cannot be listed or walked (`DAMAGED`); the rest of the volume works;
- in an inode block or the inode tree: every inode under that block fails with `DAMAGED`;
- in the free map: the volume mounts read-only (writes `READ_ONLY`) until `vfscheck fix` (later).

A reader MUST NOT return bytes from a block that failed its check, and MUST NOT loop: every walk is bounded by the
path limits (§6) and by the tree heights.

### 7.3 `vfscheck`

Walks every reachable block, checks every pointer, checks every inode's number, kind and folder records (§5.5), and
compares the reachable set with the free map. It prints counts and each problem, and MUST NOT write. The vectors
script's checker is a reference for this walk.

## 8. Commit (normative, Designed)

### 8.1 The order

A commit takes generation N = last + 1 and:
1. writes every changed block (file data, pointer blocks, inode blocks, folder blocks, bitmap blocks, and every
   parent up to both tops) into blocks that are **free in generation N-1's free map** and not yet used by this
   commit;
2. flushes (§9). A failed flush stops the commit here: nothing visible changed;
3. writes root record N into the slot generation N-1 did not use;
4. flushes;
5. only now, in RAM, marks free the blocks that N-1 used and N does not, for commit N+1 to reuse.

A writer MUST NOT reuse in commit N a block that generation N-1 reaches. So after a cut at any point, the newest valid
root record names a tree whose blocks were never written after that root record was. Rule 5 also keeps generation N
whole while N+1 is written, so a torn root record N+1 falls back to an intact N.

### 8.2 What one commit holds

- **A whole-file write** (Nia's `file_write`, the shell's `> name`): new data blocks, the inode's new pointers, one
  commit. The file is the old one or the new one.
- **A rename** (including over an existing file): both folder records in one commit. The replaced file's blocks are
  freed by rule 5.
- **Streaming writes** (`write_at`) collect in the open transaction and commit on `sync` of any handle, on close, on
  the shell's `sync`, at shutdown, or when the dirty data reaches the writer's cap.
- **`sync` of a file or a folder** means "commit now" and returns after step 4. A caller that gets success knows the
  change is on the medium (as far as the device's flush promises, §9).

### 8.3 Past the dirty cap

When a transaction's dirty blocks exceed the writer's RAM cap (2 MiB in the kernel), the writer MAY write data blocks
early, into blocks free in N-1 (rule 1). Nothing in the committed tree reaches them and the committed free map shows
them free, so a cut leaves them as free space and nothing leaks.

### 8.4 Space

- A write that cannot fit (blocks free in N-1 minus blocks already taken by this transaction, less the system reserve
  for anything but the system's own files) MUST be refused with `NO_SPACE` before any block is taken, so the file is
  unchanged.
- The **system reserve** is 2% of the area, at least 1 MiB: only files owned by the system (§5.3 owner 1) may use it.
- Each app's folder counts its blocks. The **app limit** defaults to a quarter of the area and the person can raise
  it. A write over it is refused with `QUOTA` before any block is taken. (Where the limits are stored is a compat
  feature, defined with step V5.)

## 9. The block layer flush (normative, Designed; step V2)

- The block layer gains `blk_flush() -> bool`. It returns true only when every write it acknowledged before the call
  is on the medium: the driver's gathered writes are sent, and then the device's cache flush:

  | Device | Flush |
  |---|---|
  | NVMe | Flush command when the controller has a volatile write cache (VWC); else nothing to flush |
  | AHCI, IDE | ATA FLUSH CACHE EXT |
  | USB mass storage | SCSI SYNCHRONIZE CACHE |
  | eMMC | the cache flush through SWITCH when the card's cache is on; else no volatile cache |
  | SD | no volatile cache: complete when the card reports the write done |
  | virtio-blk (ARM, PC) | VIRTIO_BLK_T_FLUSH when VIRTIO_BLK_F_FLUSH is offered; else every request completed |

- A device that refuses or fails a flush makes `blk_flush` false. The commit then stops before its root record
  (§8.1 step 2), says `[vfs] the disk did not confirm the save; nothing changed`, and the caller gets `IO`.
- Every sector read and write result MUST be checked by format 3's writer and reader.
- Format 1 and 2's rule that a write to sector 48 ends a save (the drivers' gather and flush) stays for those formats.
  Format 3 MUST NOT depend on it: it never writes sectors 1 to 871 and calls `blk_flush` itself.
- The disk drivers recognise AfriSovera's partition by sector 0. They MUST accept `AFRIVFS1` and `AFRIVFS2` alike,
  through one function, `vfs::is_ours`. A kernel from before format 3 sees `AFRIVFS2` as "other data" and leaves the
  partition alone.

## 10. Formats 1 and 2, migration and going back (normative, Designed; step V4)

### 10.1 Migration (`vfsupgrade`)

Only on the owner's command with his code, never on a U4 trial (AFR-0010: data is read-only on trial). Automatic
migration is the owner's decision at step V6, not a default of this spec.

1. The format 1 or 2 files are in RAM (the existing loader, §12). A file format 2 marked damaged keeps the mark
   (inode flag bit 4); it is never passed off as whole.
2. Format writes the area's tree (§5.7) with each file in its place (§11.4), as generation 1. Flush.
3. Read back every file through the format 3 reader and compare with RAM. A difference stops here: sector 0 is
   untouched and the disk stays format 1 or 2.
4. Write the label with the old sector 0's first 64 bytes at offset 128 and `migrated` 1. Flush. Read it back.

A cut before step 4 leaves sector 0 as it was: the next boot reads format 1 or 2. A cut during step 4 leaves a label
that fails its checksum: the loader then reads sectors 1 to 871 as format 1 or 2, taking the superblock from the
label's offset 128 when that copy is readable, or from sector 0's own first bytes when they still say `AFRIVFS1`.

### 10.2 The data format number (AFR-0010 §9)

Format numbers: 1 (`AFRIVFS1`), 2 (`AFRIVFS1` with checksums), 3 (`AFRIVFS2`). A format 1 or 2 kernel cannot read
format 3. The safe-updates guard (`readable_by`) MUST treat a slot whose kernel writes format 1 or 2 as unable to
read a format 3 partition, and `updateback` to it MUST refuse with "run vfsdowngrade first".

### 10.3 Going back (`vfsdowngrade`)

On a migrated volume (`migrated` 1), with the owner's code: rewrite sector 0 from the label's offset 128 (the first
64 bytes; the rest of the sector zero), flush, read back. The format 1 or 2 files then read as they were at the
migration. Before asking for the code it MUST say plainly that work saved since the upgrade stays on the disk but the
old system cannot see it. A volume formatted as format 3 (not migrated) has no way back and the command says so.

## 11. Folders per app, handles and rights (normative, Designed; steps V5 and V6)

### 11.1 The top folders

| Folder | Writes allowed to | Holds |
|---|---|---|
| `/system` | the system only | `afrisovera.conf`, the integrity manifest's data, appget's records |
| `/apps/<app id>` | that app and the system | the app's own files; made at install, removed at uninstall after a question |
| `/home` | the person; apps only through a picked handle | the person's documents |
| `/tmp` | anyone, each in its own folder | emptied at boot |

### 11.2 Handles

Every file operation is relative to a handle: a folder handle (rights READ, WRITE, CREATE, LIST in any subset) or a
file handle (READ, WRITE). A name given with a folder handle is walked one part at a time from that folder.
- A part that is empty, `.` or `..`, or contains the byte 0, MUST be refused (`INVALID`); so MUST a leading `/`.
  A part over 255 bytes, a path over 4,096 bytes or one more than 64 folders deep is `NAME_TOO_LONG` (§11.6). With no links (§2), every walk stays under the handle's folder.
- Rights are AFR-0001 §7.4's: READ, WRITE, and the two added for files, CREATE (bit 10: make a name in a folder) and
  LIST (bit 11: read a folder's names). `handle_dup` MUST NOT widen them. A handle to a file found by a walk carries
  at most the folder handle's READ and WRITE.

### 11.3 What an app holds

- An app with AFR-0003's `file` gets one folder handle at launch, to `/apps/<its id>`, with all four rights. Nothing
  else.
- A file of the person's reaches an app only through the picker that the desk draws (never the app): the person
  picks a file or folder, and the app gets a handle to it alone, with READ, or READ and WRITE, as the person chose.
  The handle ends with the app.
- No app may write `/system` or another app's folder (format 2's `-4` rule, kept).
- A compatibility bridge (AFR-0001 §7.7) is an app: it gets the folder handle of the foreign program's own folder
  and maps the foreign paths onto it.

### 11.4 The flat names of formats 1 and 2

The kernel's existing calls with a bare name (`vfs::read_str`, `read_raw`, `read_full`, `write`, `write_full`) keep
working: a name without `/` means `/system/<name>` when the system owns it (`afrisovera.conf`, every bundled app's
name, every name the integrity manifest or appget owns) and `/home/<name>` otherwise. Migration (§10.1) places each
file by the same rule. Their 512-byte and 16 KiB views keep their meaning.

### 11.5 The app table

The owner field's app index (2 and up) names an app id in a table kept under `/system`. Its layout is defined with
step V5.

### 11.6 Errors

| Code | Name | Meaning |
|---|---|---|
| 1 | `NOT_FOUND` | no such name |
| 2 | `EXISTS` | the name exists (create with EXCLUSIVE) |
| 3 | `NOT_FOLDER` | a part of the path, or the handle, is not a folder |
| 4 | `IS_FOLDER` | a file operation on a folder |
| 5 | `NOT_EMPTY` | removing a folder that has names in it |
| 6 | `NO_SPACE` | the volume (or its system reserve) is full; nothing was changed |
| 7 | `NAME_TOO_LONG` | a name over 255 bytes or a path over 4,096 |
| 8 | `READ_ONLY` | the volume is mounted read-only, or the file has the read-only flag |
| 9 | `BAD_HANDLE` | no such handle |
| 10 | `IO` | a sector read or write, or a flush, failed |
| 11 | `DAMAGED` | a checksum did not match (§7.2) |
| 12 | `INVALID` | a bad name or argument (§11.2) |
| 13 | `QUOTA` | the app's limit is reached; nothing was changed |
| 14 | `TOO_BIG` | past 4 GiB |
| 15 | `DENIED` | the handle lacks the right |

On ring 3 code c is returned as `-(31 + c)` (AFR-0001 §7.2 reserves -32 to -63 for file errors); the calls
themselves are AFR-0001 §7.9 (step V6). Nia's existing `file_read` and `file_write` keep their values (-1 to -5); the
new Nia calls' values are set in AFR-0003 at step V5. A compatibility bridge maps them one to one onto the foreign
system's errors (`DAMAGED` to Linux `EIO`).

## 12. Formats 1 and 2 (`AFRIVFS1`, Implemented)

Described in `docs/vfs-format.md`, which is normative for them. In short: sector 0 superblock (magic, file count,
byte 12 = 2 when a file is over 512 bytes; format 2 adds the version at byte 13 and a CRC-32 at 16), slot i's meta at
sector 1 + 2i and first 512 bytes at 2 + 2i, the part past 512 bytes at sectors 128 + 31i onward, 24 slots, 16 KiB
per file. The loader (`vfs::load_from_disk`) stays in every kernel that writes format 3, unchanged, as the reader for
these formats.

## 13. Features and versioning

- The three feature words (label offsets 48 to 59, mirrored in each root record) work as in ext4: a reader that sees
  an **incompat** bit it does not know MUST NOT mount; a **read-only-compat** bit it does not know: mount read-only; a
  **compat** bit it does not know: ignore it. v1 sets none.
- **Reserved:** ro-compat bit 0, case-fold folders (Windows W1): a folder made empty with the case-fold flag compares
  names by a fixed simple case-folding table whose version is the label's fold table version, never the locale, and
  refuses a second name differing only in case. Incompat bit 0, encryption at rest (its own plan). Incompat bit 1, `inode-reuse`: an inode tree of
  height 4 or the reuse of inode numbers (§5.4), its own plan.
- A new incompatible layout takes a new format number and, when sector 0 changes meaning, a new magic (AFR-0008
  §10).

## 14. Test vectors and suites

**Vectors** (`docs/spec/vectors/afr0011_vectors.py`, 14 Nov 2026, all pass): CRC-32C check value; the pointer layout;
a fresh 8 MiB volume (label sector SHA-256 `70c2e9852b02c53073234b6e6cae359bd1f23d9a9bc97389739939b220d6d695`, root record A `4d0a7ce689aac679e8bb78d1cbf21c3a1e6eee99b45b07a6882ad11afe67104e`, inode block
`06c40f8d761e7136dc16053c8522bd669a6fde5a8b79d6b8041d8f9d64ec022a`, bitmap `1b5de154ee61be3af706595f3c0de96194772c9bc759c32ba3143475cbab078d`, top folder block `07a754e67dd1e767b3b349890fa3f8afd5e0a98c7f8fa4d3b4415e8951755da7`, whole image `37f46664e0af7fb908cde581f2084b6efa1659e4de8d2f9b3776ea6d0abcac3a`)
; the checker refusing a flipped byte in the label, the root record, the inode block, the
free map and the folder block; a torn root record B falling back to A.

**To write (each step's suites, run on a cloud runner):**

| Rule | Test |
|---|---|
| §8.1 old or new | the host crash harness (`afrifs`, V1): every write index of each script, every prefix of the unflushed writes, 64 seeded reorders per cut, and 4 KiB blocks torn into 1 to 7 new sectors; mount, `vfscheck` clean, and the tree equals the model before or after the commit in flight |
| §7, §5 | fuzz targets: the label and root records; inode and folder blocks; a whole image through mount; paths at the handle walk, including 4,096 and 4,097 bytes |
| §9 | each PC disk and ARM virtio: `vfs1-basic`, `vfs1-cut` (QMP quit at 20 seeded times during a 16 MiB save) |
| §10 | `vfs1-migrate` (format 1, format 2, a damaged mark, a full disk), `vfs1-migrate-cut`, `vfs1-down`, `vfs1-small` |
| §7.2 | `vfs1-damage` |
| §11 | `vfs1-apps` |
| §8.4 | `vfs1-full`: `NO_SPACE` and `QUOTA` leave the file unchanged; the system still saves its settings |
| §6 | 16 MiB and 256 MiB files by SHA-256; 10,000 names in one folder, lookup time |

## 15. Security considerations

- **No escape from a folder handle.** Walks are part by part from the handle, `..` is refused, and v1 has no links.
  The fuzz target over the walk checks that the result is always under the handle or refused.
- **Symbolic links** are where path escapes come from. v1 has none. The shape for later (a link the kernel never
  follows, resolved only by a bridge from its own handle) needs its own plan and review.
- **Hostile disks.** A stick or disk image may be made by an attacker. Every field read is range-checked, every
  block is checksum-checked before use, every walk is bounded, and mount never writes. CRC-32C detects damage, not
  forgery: someone who can write the disk can write consistent checksums. Integrity against an attacker with the disk
  is the encryption-at-rest feature's job.
- **A full disk** cannot stop the system saving its own settings (the reserve, §8.4), and one app cannot fill it
  (§8.4's app limit).
- **The flush** is a promise only as good as the device's. A device that acknowledges a flush it did not do can lose
  a committed change; the two root records still give an intact older tree as long as the device does not reorder
  across its own flush.

## 16. References

- `docs/02-drivers-and-services-asili.md` (the VFS / Storage row), ADR 0001, ADR 0004.
- AFR-0001 §7, AFR-0003 (the `file` grant), AFR-0008 §10, AFR-0010 §9.
- `docs/vfs-format.md` (formats 1 and 2).
- The plan and its verdict: VFS_V1_PLAN.md and ARCHITECT_REVIEWS.md, 14 Nov 2026.
- CRC-32C: RFC 3720 appendix B.4 (the polynomial and check value).
- Design references, for ideas, no code taken: the ZFS on-disk format (uberblock ring, checksum in the parent
  pointer), ext4's feature-flag words.
