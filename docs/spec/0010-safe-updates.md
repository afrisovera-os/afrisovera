# AFR-0010: Safe updates (two boot files, firmware rollback)

| | |
|---|---|
| **Status** | **Partly implemented**: §4 to §7 and `updatecheck` (§8.1) Implemented; staging, trial and keep (§8.2 to §8.5) **Designed** |
| **Version** | 0.1 (2026-11-13) |
| **Editor** | Reliability engineer, reviewed by the chief architect (ARCHITECT_REVIEWS.md, 13 Nov 2026 night) |
| **Code** | `kernel/src/pc/update.rs`, `kernel/src/pc/compat/shell_pc.rs` (`updatecheck`), `scripts/make-update.py` |
| **Suites** | `pc-updatecheck` |
| **Vectors** | `docs/spec/vectors/afr0010_vectors.py` (demo release key only) |
| **Depends on** | AFR-0008 (boot file, `bootcopy`, `bootentry`, `bootnext`); G7 step 1 (strict Ed25519 verify) |
| **Replaces** | AFR-0008 §9 "A/B system partitions" |
| **Licence** | Apache-2.0 |

RFC 2119 key words apply.

---

## 1. Motivation

One bad update must never stop a machine from starting, as a single faulty update stopped millions of PCs in
July 2024. An AfriSovera update is checked whole before anything is written, it is started once on trial, and if
it does not prove itself healthy the machine goes back to the system it had, with no action from anyone.

## 2. The key fact

On the PC the boot file (`mansa-pc.efi`, AFR-0008 §3) is the whole system, apps included. The owner's data is
in AfriSovera's own partition (`AFRIVFS1`). So two copies of the system are two boot files in the EFI system
partition (ESP), not two partitions: no partition table changes, and every write reuses AFR-0008's tested code.

## 3. Terms

- **Slot file**: `\EFI\afrisovera\afrisovera.efi` or `\EFI\afrisovera\afrisovera-b.efi`.
- **Main entry**: AfriSovera's Boot#### (AFR-0008 §6); its File() node names one slot file.
- **Spare slot**: the slot file the main entry does NOT name, read from the variable's device path, never assumed.
- **Trial entry**: a second Boot#### "AfriSovera trial", at the END of BootOrder, naming the spare slot.
- **On trial**: BootCurrent is the trial entry, whatever brought the machine there (BootNext, or the firmware
  falling through BootOrder to it).

## 4. The update file (normative, Implemented)

Little-endian. Format 1:

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 8 | magic | `AFRIUPD1` |
| 8 | 4 | header length | 128 for format 1; a reader MUST refuse anything over 4096 |
| 12 | 4 | format | 1 |
| 16 | 16 | target | ASCII, zero padded: `pc-x86_64` (ARM targets later) |
| 32 | 4 | release version | MUST be greater than the running system's (§6) |
| 36 | 4 | reserved | 0 |
| 40 | 8 | key id | first 8 bytes of SHA-256(the signing public key) |
| 48 | 8 | boot file size | 1 byte to 64 MiB |
| 56 | 32 | boot file SHA-256 | |
| 88 | 40 | reserved | all 0 |
| 128 | 64 | signature | Ed25519 over `AfriSovera update v1`, one 0x00 byte, then bytes 0 to 127 |
| 192 | size | boot file | an x86-64 UEFI program (MZ, PE32+, machine 0x8664); nothing after it |

Every field the device acts on is inside the signed header. The context string means a signature made with the
same key for anything else can never pass as an update.

## 5. Checks (normative, Implemented in `updatecheck`)

A reader MUST, in this order, refuse a file whose: length is under 192; magic is wrong; header length is not
128; format is not 1; reserved bytes are not zero; key id names no key trusted for updates (§7); signature does
not verify (strict Ed25519, G7 step 1); target is not this machine's; version is not newer (§6); boot file size is
out of bounds; total length is not 192 + size; boot file SHA-256 differs; boot file is not an x86-64 UEFI program.

The stick is untrusted:
- Every length is bounded before it is used, and every FAT cluster chain is walked at most once per cluster.
- The boot file is streamed through SHA-256 a sector at a time, never held in memory whole.
- The header is read twice, and the second read MUST give the bytes the signature was checked over.
- `updatecheck` writes nothing to any disk, stick or firmware variable.

## 6. Versions

A plain u32 compared with `update::RELEASE` built into the running kernel (today 1). A lower or equal version
is refused. `updateback` (§8.5) is the owner's deliberate return to the previous slot and stays allowed until a
TPM NV floor exists (later step, with the TPM work).

## 7. Keys

- The kernel holds a table of (public key, purpose). A key has ONE purpose: an update key never verifies an
  app, and an app publisher key (`integrity.rs`) never verifies an update.
- The demo release key (seed `21 22 .. 40`, `scripts/make-update.py`) is in the repo, so it is for test builds
  only. A build that trusts it says so at boot (`[pc] updates: release 1; trusts the DEMO release key: a test
  build, not for release`) and in every `updatecheck` that it accepts.
- The real release key is made and kept offline by the owner. It is never in git, the cloud, CI or a chat.
- **Built (14 Nov 2026):** a build marked for release (`--features pc,release`) does not compile while it trusts a
  demo key, update or app: compile-time checks in `pc/update.rs`. The update table is checked by each row's `demo`
  flag and by its bytes, so the demo key copied into a row marked `demo: false` is refused too. The suite
  `pc-releaseguard` proves the checks are armed: the plain build passes, the release build fails naming both keys,
  and a test-only planted row (`--features pc,releaseguard-plant`) is caught by the byte check. When the owner's
  keys land, its expectation flips in the same change. The ARM phone track gets the same check with its own
  release build.
- **What may be called a release.** `cargo build --release` is a build profile (speed), not a release: it does not
  turn the guard on, and an image built with it, such as the fast USB stick, is still a test image and says so.
  Only a build marked `--features pc,release` may be called a release, and only once the owner's keys exist.

## 8. The update steps

### 8.1 `updatecheck` (Implemented)
Reads `UPDATE.AFU` in the top folder of a FAT32 USB stick (the whole stick, the first FAT partition of an MBR,
or GPT entry 1; 512-byte sectors), runs §5, prints the version, size, the first 16 hex digits of the boot file's
SHA-256 and the verdict. Nothing is written.

### 8.2 `updatestage [remove]` (Designed; architect merge review before code)
1. Run §5 again. Find the main entry and the spare slot (§3). Refuse if the running system is not the one the
   main entry names (on trial, started from a stick, or a hand-made entry). Never write the main entry's file.
2. The ESP is shared with Windows: check free space first and leave at least 2 MiB free after the copy; never
   touch a file outside `\EFI\afrisovera\`.
3. FAT write order: the file's clusters, then both FAT copies, then the directory entry last.
4. Compute SHA-256 over the bytes READ BACK from the ESP and compare with the signed header. A match with the
   stick's bytes is not enough.
5. Add the trial entry at the END of BootOrder. The running system and the main entry are not touched.

### 8.3 `updatetry` (Designed)
Writes BootNext = the trial entry, on the owner's code. The firmware starts the new system once and deletes
BootNext as it does so.

### 8.4 On trial (Designed)
- The owner's data partition is mounted READ ONLY, and the person is told plainly that changes are not saved
  until the new version is kept.
- Self-tests run (Ed25519, SHA, heap, the desk, the data partition's check).
- Not healthy: the system calls ResetSystem; BootNext is already gone, so the firmware starts the old system.
  A crash before the self-tests (a triple fault in early boot) ends the same way. A hang needs the power button
  until a hardware watchdog driver exists.
- Kept automatically when the self-tests pass, the desk has run 2 minutes AND the person has used it (a key or
  pointer event). A machine nobody touches stays on trial. A setting can require `updatekeep` instead.

### 8.5 Keep and back (Designed)
- Keep: first the main entry is rewritten to name the new slot, then the trial entry is removed. A power cut
  between the two leaves a harmless extra entry, which the next boot removes.
- One notice after keeping: "updated to version N; `updateback` returns to version M".
- The old slot file stays as the way back (`updateback`).

## 9. The data format rule (normative)

An update MUST NOT change the data partition's format in a way the previous version cannot read, until a
format-version plan exists. Otherwise `updateback` and the automatic rollback would bring back a system that
cannot read the owner's data. A new format takes a new magic (AFR-0008 §10).

## 10. Limits

- Without Secure Boot, a person holding the PC can replace a slot file (AFR-0008 §8). The signature guards the
  update path, not physical access.
- A hang after the new system has taken the screen needs the power button until the TCO watchdog step.

## 11. Test vectors and suites

| Rule | Where |
|---|---|
| Header layout, context string, key id, demo key, deterministic signature | `afr0010_vectors.py` |
| §5 every refusal, both table kinds, no writes, demo boot line | `pc-updatecheck` (13 boots) |
| §8.2 to §8.5 | `pc-updatestage`, `pc-updatetry` (SATA, NVMe 4K, recovery-first, dual-boot), Designed |
