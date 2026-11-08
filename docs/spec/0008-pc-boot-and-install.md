# AFR-0008 — PC boot and install chain (UEFI, GPT, ESP, boot entries)

| | |
|---|---|
| **Status** | **Implemented** (live boot, whole-disk install, side-by-side partition, boot file copy, boot entry, BootNext); A/B updates and Secure Boot signing **Designed** |
| **Version** | 0.1 (describes code at `485b863`, 2026-11-08) |
| **Editor** | Chief architect |
| **Code** | `kernel/src/pc/main.rs` (`efi_main`), `pc/uefi.rs`, `pc/gpt.rs`, `pc/esp.rs`, `pc/bootent.rs`, `pc/fwinfo.rs`, `scripts/make-pc-usb.sh` |
| **Suites** | `pc-boot`, `pc-gpt`, `pc-installplan`, `pc-tablebackup`, `pc-installpart`, `pc-tablerestore`, `pc-formatpart`, `pc-formatsata`, `pc-installdisk`, `pc-installdisk-sata`, `pc-installdisk-emmc`, `pc-bootentries`, `pc-bootplan`, `pc-bootcopy`, `pc-bootentry`, `pc-bootnext`, `pc-fwinfo` |
| **Vectors** | `docs/spec/vectors/afr0008_vectors.py` |
| **Depends on** | ADR 0006 (PC track); AFR-0002 §4.4 rule 5 (storage safety) |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply.

---

## 1. Motivation

Installing an operating system is the one moment it can destroy the owner's data. The PC track (ADR 0006)
promises that AfriSovera can boot from a USB stick without touching the PC, and be installed beside or
instead of the existing system with a way back at every step. This spec fixes the on-disk formats
AfriSovera reads and writes, the order of writes, the refusals, and the owner-confirmation scheme, so that
any change to the install path is checked against a written contract.

## 2. Scope and status summary

| Part | Status | Command / code |
|---|---|---|
| Boot as a UEFI application from removable media | Implemented | `efi_main`, `make-pc-usb.sh` |
| Read every internal disk's partition table (GPT/MBR) | Implemented, read-only | `lsparts` (`gpt::show`) |
| Plan a partition in free space | Implemented, read-only | `installplan` |
| Back up partition tables to a USB stick; compare | Implemented | `tablebackup`, `tablecheck` |
| Add AfriSovera's partition to a healthy GPT | Implemented | `installpart <disk> <MiB>` |
| Restore a table from the stick | Implemented | `tablerestore <disk>` |
| Format AfriSovera's partition | Implemented | `formatpart <disk>` |
| Whole-disk install (fresh GPT + ESP + partition) | Implemented | `installdisk <nvme\|sataN\|emmc>` |
| Read firmware boot entries | Implemented, read-only | `bootentries` |
| Plan / copy the boot file into an existing ESP | Implemented | `bootplan`, `bootcopy [remove]` |
| Add / remove AfriSovera's Boot#### | Implemented | `bootentry [remove]` |
| One-shot boot via BootNext | Implemented | `bootnext` |
| Firmware and Secure Boot report | Implemented, read-only | `fwinfo` |
| MBR disks: change | **Not built** (planned only, refused) | — |
| Secure Boot (signed shim / own keys) | **Designed** (§9) | — |
| A/B system partitions with rollback | **Designed** (§9) | — |

## 3. Terminology

- **ESP** — EFI system partition (type `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`), FAT32.
- **AfriSovera partition** — GPT type **`A5F1C0DE-0B5E-4A50-9AF1-50FE2A000001`**, name `AfriSovera`, holding
  the AfriSovera VFS (`AFRIVFS1` at its first sector).
- **Boot file** — the kernel itself, a PE32+ UEFI application (`mansa-pc.efi`, target `x86_64-unknown-uefi`).
- **Owner code** — four hex digits printed by a write command, derived from the current on-disk state (§7).
- **bs** — the disk's logical block size.

## 4. Boot (normative)

### 4.1 Media

- Removable: a FAT32 stick with the boot file at `\EFI\BOOT\BOOTX64.EFI` (the UEFI default removable path).
  `scripts/make-pc-usb.sh` produces `build/afrisovera-pc-usb.zip` containing that tree; no disk image.
- Installed (whole disk): the same path in the ESP that `installdisk` writes.
- Installed (beside another OS): `\EFI\afrisovera\afrisovera.efi` in the existing ESP, started by a
  Boot#### entry (§6) or from the firmware's boot menu.

### 4.2 What the kernel does before `ExitBootServices`

In this order (`pc/main.rs`):

1. **Keep its own image** (`uefi::keep_own_image`): read its boot file through `EFI_LOADED_IMAGE_PROTOCOL`
   (`5B1B31A1-9562-11D2-8E3F-00A0C969723B`) and `EFI_SIMPLE_FILE_SYSTEM_PROTOCOL`
   (`964E5B22-6459-11D2-8E39-00A0C969723B`); path from the image's device-path file nodes, falling back to
   `\EFI\BOOT\BOOTX64.EFI`; stored in pages below 4 GiB of memory type `0x8000_0A50`, at most 16 MiB. This
   copy is what `installdisk` and `bootcopy` write, so the installed file is byte-identical to the running one.
2. **Keep boot entries** (`uefi::keep_boot_entries`): `BootOrder`, `BootCurrent`, `BootNext` and every
   `Boot####` of `EFI_GLOBAL_VARIABLE` (`8BE4DF61-93CA-11D2-AA0D-00E098032B8C`); limits 32 entries, 768
   bytes per option, 64 BootOrder slots.
3. **Keep firmware facts** (`fwinfo::keep`): vendor, revision, UEFI version, `SecureBoot`, `SetupMode`.

After `ExitBootServices`, runtime services are called at their **physical** addresses with interrupts
masked; `SetVirtualAddressMap` is never called. A firmware that requires virtual mapping of runtime services
is not supported (none found in tests).

### 4.3 Secure Boot

AfriSovera's boot file is not signed by a key in the default `db`. With Secure Boot enabled the firmware
refuses it; `fwinfo` tells the owner in plain words to switch Secure Boot off. The kernel MUST NOT attempt
to change `PK`, `KEK`, `db` or `SetupMode`.

## 5. Formats written (normative)

### 5.1 CRC32

IEEE 802.3, reflected, polynomial `0xEDB88320`, initial `0xFFFFFFFF`, final complement (`gpt::crc_feed`
then `!`). Vector: `"123456789"` → `0xCBF43926`.

### 5.2 GPT as read

A header is accepted only if: signature `EFI PART`; header size 92…512; `MyLBA` equals the LBA it was read
from; CRC32 over `HeaderSize` bytes with bytes 16–19 zeroed equals bytes 16–19. A table is **healthy** (and
only then may be changed) when additionally: the protective MBR has `55 AA` and a type `0xEE` entry; the
backup header (at `AlternateLBA < disk blocks`) is also valid; entry size is 128…512 and divides 512;
1…1024 entries; the entry-array CRC equals header offset 88 **in both copies and both are equal**; no two
partitions overlap. A damaged table is reported "not safe to change" and never written.

### 5.3 `installdisk` layout

For a disk of `blocks` blocks of size `bs` (`bs ≤ 4096`):

| Item | Value |
|---|---|
| Entry array | 128 entries × 128 bytes = 16 384 bytes = `E = 16384 / bs` blocks |
| First / last usable LBA | `2 + E` / `blocks − 2 − E` |
| Alignment | 1 MiB (`1 MiB / bs` blocks) |
| ESP | starts at the first 1 MiB boundary, 300 MiB, entry 1, name `EFI system` |
| AfriSovera partition | from the end of the ESP to the last usable LBA, aligned down to 1 MiB, entry 2, name `AfriSovera`; refused if under 256 MiB |
| Header | revision `0x00010000`, size 92, random v4 disk GUID, entries start LBA 2 (primary) / `blocks − 1 − E` (backup) |
| Partition GUIDs | random v4; attributes 0 |
| Protective MBR | one entry: status 0, CHS `00 02 00`, type `0xEE`, CHS end `FF FF FF`, start LBA 1, size `min(blocks − 1, 0xFFFFFFFF)`; `55 AA` |

**Write order** (each step leaves a disk no other OS mistakes for a valid foreign table):
1. zero LBA 0, LBA 1 and the last block;
2. write the ESP file system (§5.4);
3. zero the AfriSovera partition's first block;
4. backup entry array, then backup header;
5. primary entry array, then primary header;
6. **protective MBR last**;
7. read back: the table must pass `ours()` (healthy, exactly one AfriSovera entry inside the usable range,
   no overlap) and the boot file must compare byte-for-byte;
8. format the AfriSovera partition (VFS sync) and check `AFRIVFS1` at its start.

`installdisk` does **not** create a Boot#### entry: firmware starts `\EFI\BOOT\BOOTX64.EFI` on a disk with
no entry of its own.

### 5.4 ESP file system written by `installdisk`

FAT32, one sector per cluster (= one disk block), 32 reserved sectors, 2 FATs, media `0xF8`, hidden
sectors = partition start, root cluster 2, FSInfo at sector 1 (signatures `0x41615252`, `0x61417272`,
`0xAA550000`), backup boot sector at 6, drive `0x80`, extended signature `0x29`, random serial, OEM
`AFRISOVR`, label `AFRISOVERA `, type `FAT32   `; at least 65 525 clusters. Clusters: 2 = root (volume
label + `EFI`), 3 = `EFI` (`.`, `..`, `BOOT`), 4 = `BOOT` (`.`, `..`, `BOOTX64 EFI`), boot file contiguous
from cluster 5. Directory timestamps fixed at 2026-11-06 12:00 (reproducible output).

### 5.5 `installpart` (beside another OS)

Adds exactly one entry (type AfriSovera, name `AfriSovera`, random unique GUID) in a free slot, in the
largest 1 MiB-aligned free gap (`installplan`). Never resizes, moves or changes another entry. Preconditions:
the table is healthy; a USB-stick backup of this disk exists and compares equal (`tablebackup`); the owner
code matches; both array CRCs are equal; the chosen slot is still empty. Write order: backup array sector,
backup header, primary array sector, primary header; then read back both headers and both arrays.

### 5.6 Table backup on the stick

On a stick already treated as AfriSovera's (blank or `AFRIVFS1`, never the stick the PC booted from):
sector 2048 is a directory (`AFRIPTBK`, count, time); then one 260-sector slot per disk (max 8): a record
sector (label, size, copied sector runs, CRC32 of the data, CRC32 of the record) and up to 259 copied
sectors (GPT: both headers and arrays; MBR: sector 0). `tablebackup` keeps a pre-change copy unless the owner
types `tablebackup new`. `tablerestore` writes back only the sectors that differ, backup GPT copy first.

### 5.7 `bootcopy` into an existing ESP

Target file `\EFI\afrisovera\afrisovera.efi`. Preconditions: healthy GPT; the ESP is FAT32 **closed
cleanly** (FAT[1] bits 27 and 26 set — not after Windows Fast Startup or a crash); both FAT copies equal;
enough free clusters; `\EFI` exists with two adjacent free directory slots and a free short name
`AFRISO~1`…`AFRISO~9` (long file name written too). Written: only free clusters, their FAT entries in both
FATs, the two `\EFI` slots (written **last**, so an interrupted copy leaves only unreferenced clusters), and
FSInfo's free count. Read back byte-for-byte. `bootcopy remove` restores both FATs and FSInfo exactly.

### 5.8 Boot entry (`EFI_LOAD_OPTION`, 140 bytes)

| Offset | Size | Field | Value |
|---|---|---|---|
| 0 | 4 | Attributes | `0x00000001` (`LOAD_OPTION_ACTIVE`) |
| 4 | 2 | FilePathListLength | 112 |
| 6 | 22 | Description | `AfriSovera` UTF-16LE + NUL |
| 28 | 42 | Hard-drive media node | `04 01 2A 00`; partition number u32 (**1-based GPT entry index**); start LBA u64; size in LBAs u64; partition unique GUID (16, as stored in the entry); MBRType `02` (GPT); SignatureType `02` (GUID) |
| 70 | 66 | File-path media node | `04 04 42 00`; `\EFI\afrisovera\afrisovera.efi` UTF-16LE + NUL |
| 136 | 4 | End of device path | `7F FF 04 00` |

Variables are written with attributes `NV | BS | RT` (`7`); a delete is attributes 0 with empty data.

**`bootentry` sequence:** refuse if AfriSovera already has an entry (description starts `afrisovera`,
case-insensitive, or the path contains `\afrisovera\`); pick the lowest `####` below `0x100` that is neither
present nor in BootOrder and re-probe it is still `EFI_NOT_FOUND`; refuse if BootOrder is full (64) or already
lists it; check the owner code; write `Boot####`, read back; write BootOrder with the number **appended at the
end** (the PC keeps starting its usual OS first), read back. On any failure: delete `Boot####` and restore
the old BootOrder. `bootentry remove` reverses it. `bootnext` writes `BootNext` = the number (one boot only;
firmware deletes it) on its own code, read back.

## 6. State machine of a side-by-side install

```
 live (USB) ──tablebackup──► Backed-up ──installpart + code──► Partitioned ──formatpart + code──► Formatted
                │                              │ tablerestore + code               │
                │                              ▼                                   │
                │                          Live (table as before)                  │
 Formatted ──bootcopy + code──► File-in-ESP ──bootentry + code──► Entry-at-end-of-BootOrder
                ▲ bootcopy remove                ▲ bootentry remove            │ bootnext + code
                └────────────────────────────────┘                             ▼
                                                                   boots AfriSovera once, then as before
```

Every forward arrow has a way back, and every arrow is refused unless its preconditions hold *now*.

## 7. Owner confirmation codes (normative)

Every command that writes to an internal disk or a firmware variable first prints what it will do and a
four-hex-digit code, and writes nothing unless run again with `yes <code>`. The code is the low 16 bits of
the complemented CRC32 (§5.1) of the state the owner was shown, so a code shown for one state cannot
authorise a write to a different state:

| Command | CRC input (concatenated, integers little-endian) |
|---|---|
| `installdisk` | `"installdisk"` ‖ capacity u64 (blocks) ‖ bs u64 ‖ LBA 0 (512 B) ‖ LBA 1 (512 B) |
| `installpart` | primary header CRC u32 ‖ planned start LBA u64 ‖ requested size u64 (MiB) |
| `formatpart` | partition start u64 ‖ length u64 ‖ the partition's first sector (512 B) |
| `tablerestore` | record bytes 64–67 ‖ CRC of the disk's current copy of those sectors (u32) |
| `bootentry`, `bootnext` | tag (`"bootentry"`, …) ‖ Boot number u16 ‖ load option (140 B) ‖ current BootOrder bytes |

The code is **not** a security mechanism (anyone at the keyboard can read it); it is a guard against
writing a state the owner did not see — a stale screen, a swapped disk, a table changed by another OS.

## 8. Security considerations

- **Data safety is the primary property.** Rules: never write a damaged table; never write an unknown
  sector 0; never resize or move another system's partition; never write the medium the PC booted from;
  back up before change; read back after write; every write has an undo command.
- Physical presence is assumed: the shell that runs the install commands takes input only from the local
  keyboards, COM1 and a USB serial cable on the machine itself; there is no network shell. A future remote
  shell MUST NOT expose these commands.
- BootOrder is appended to, never reordered: a mistake leaves the owner's usual OS starting first.
- Firmware variable writes are bounded (one Boot####, BootOrder, BootNext). Repeated writes wear NVRAM; the
  commands refuse no-op writes (entry already present).
- Without Secure Boot, anyone with physical access can replace the boot file. A/B verified boot (§9) is the
  designed answer; until then this is a stated limit.

## 9. Designed, not built

- **Secure Boot:** a signed shim (Microsoft UEFI CA) that verifies an AfriSovera-signed kernel, or enrolment
  of the owner's own key by the owner in firmware setup. AfriSovera never enrols keys itself.
- **A/B system partitions:** two AfriSovera system partitions (type GUID suffix `…000002` A, `…000003` B) and
  one data partition (`…000001`, today's type); boot file reads a `boot-state` record (active slot, tries
  left, successful flag) and falls back to the other slot after 3 failed boots (docs/02 "Update & integrity").
- **MBR disks:** read and planned today; writing requires converting to GPT, which is out of scope until a
  backup-and-convert path with its own spec exists.

## 10. Versioning

The partition type GUIDs and the boot path are fixed for the life of the PC track; a format change takes a
new type GUID, never a reinterpretation of an existing one. A new VFS format takes a new magic (`AFRIVFS2`),
never a changed meaning of `AFRIVFS1`. The load option layout follows UEFI 2.10 §3.1.3.

## 11. Test vectors

### 11.1 Existing suites

| Rule | Suite |
|---|---|
| Boots as UEFI app on q35 + OVMF | `pc-boot` |
| Damaged GPT reported, never changed; both copies + array CRC checked | `pc-gpt` |
| Plan never resizes another partition | `pc-installplan` |
| Backup on the stick, read back and CRC-checked | `pc-tablebackup` |
| `installpart` requires backup + code; both copies written; read back | `pc-installpart` |
| Restore writes only differing sectors, backup copy first | `pc-tablerestore` |
| Format only inside AfriSovera's partition | `pc-formatpart`, `pc-formatsata` |
| Whole-disk install boots with no stick | `pc-installdisk`, `pc-installdisk-sata`, `pc-installdisk-emmc` |
| Boot entries read-only listing | `pc-bootentries` |
| ESP checked clean + FATs equal; copy and remove exact | `pc-bootplan`, `pc-bootcopy` |
| Entry appended, survives power-off, removed exactly | `pc-bootentry` |
| BootNext starts AfriSovera once | `pc-bootnext` |
| Secure Boot state reported | `pc-fwinfo` |

### 11.2 Byte vectors (`vectors/afr0008_vectors.py`)

Load option for partition 1, start LBA 2048, size 614 400 LBAs, GUID bytes `00 01 … 0F`:

```
  0  01 00 00 00 70 00 41 00 66 00 72 00 69 00 53 00
 16  6F 00 76 00 65 00 72 00 61 00 00 00 04 01 2A 00
 32  01 00 00 00 00 08 00 00 00 00 00 00 00 60 09 00
 48  00 00 00 00 00 01 02 03 04 05 06 07 08 09 0A 0B
 64  0C 0D 0E 0F 02 02 04 04 42 00 5C 00 45 00 46 00
 80  49 00 5C 00 61 00 66 00 72 00 69 00 73 00 6F 00
 96  76 00 65 00 72 00 61 00 5C 00 61 00 66 00 72 00
112  69 00 73 00 6F 00 76 00 65 00 72 00 61 00 2E 00
128  65 00 66 00 69 00 00 00 7F FF 04 00
```

Owner codes:

| Input | Code |
|---|---|
| `bootentry`, Boot0002, the option above, BootOrder `0000,0001` | `8201` |
| `installpart`, header CRC `0x12345678`, start 2048, 512 MiB | `2D0C` |
| `installdisk`, 16 777 216 blocks of 512 B, LBA 0 and 1 all zero | `C259` |

### 11.3 Conformance tests still to write

- A unit test (host-side) that runs the generator's inputs through `bootent::option` and `gpt::code` and
  compares with the table above.
- A fault-injection suite: power cut (QEMU `quit`) after each numbered write step of §5.3 and §5.5, then
  boot the other OS image and check it still sees a valid table.

## 12. References

- UEFI Specification 2.10: §3.1.3 (load options), §3.3 (globally defined variables), §5.3 (GPT), §10.3
  (device path nodes), §13.3 (FAT, ESP).
- Microsoft FAT32 File System Specification (FSInfo, clean-shutdown bits).
- docs/05-roadmap.md PC track steps 4–5; ADR 0006.
