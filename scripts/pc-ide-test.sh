#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): SATA disks in IDE mode (pc/ide.rs).
#
# Many PCs from about 2006 to 2014 have SATA set to "IDE" (compatibility) mode in the firmware, and
# a Windows installed that way stops starting if it is changed to AHCI. The controller is then PCI
# class 01:01, which ahci.rs never sees, so such a PC had no disk for AfriSovera. ide.rs drives it:
# ATA task-file registers at the fixed ports (or the native-mode BARs), polled PIO, LBA28/LBA48,
# FLUSH CACHE after a save; ATAPI CD/DVD drives are named and not used; a disk holding anything is
# never written; a blank one only after `claimdisk ide`.
# Two boots of the PC kernel (q35 + UEFI) with QEMU's piix3-ide (compatibility mode, ports 0x1f0 and
# 0x170): primary master a blank 8 MiB disk, primary slave an empty CD drive, secondary master an
# 8 MiB disk with an MBR (somebody's data). Boot 1: lside, claimdisk ide, write a file, sync, lside.
# Boot 2: cat the file, lside. Graded: each drive found and named as it is, the blank disk kept
# unused until claimed, the file kept across the power-off, the cache flushed, nothing failed in the
# driver, and the MBR disk byte for byte the same after both boots (checked on the image).
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-ide-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-ide-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[ide\] .*(failed|still busy|no I/O ports|could not be read|not supported)|blk self-test FAILED'
RE_S='^  storage: ([0-9]+) sector\(s\) read, ([0-9]+) written, ([0-9]+) cache flush\(es\)$'

grade() { # grade <boot1 log> <boot2 log> <word>
    local fail=0 A B n word=$3
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[ide\] controller [0-9a-f:.]+ \(IDE mode, interface 0x[0-9a-f]{2}: primary compatibility, secondary compatibility\)$' ||
        _f "boot 1: the IDE controller was not found in compatibility mode"
    hasA '^\[ide\] primary master \(0x1f0\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), blank: not used unless claimed \(claimdisk ide\)$' ||
        _f "boot 1: the blank 8 MiB disk (primary master) was not identified, or not kept unused until claimed"
    hasA '^\[ide\] primary slave \(0x1f0\): CD/DVD drive \(ATAPI\): not used$' || _f "boot 1: the CD drive (primary slave) was not named as ATAPI"
    hasA '^\[ide\] secondary master \(0x170\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), holds other data \(Windows\?\): left alone$' ||
        _f "boot 1: the MBR disk (secondary master) was not found, or not left alone"
    hasA '^\[ide\] 1 controller\(s\), 2 disk\(s\), 1 CD/DVD drive\(s\)$' || _f "boot 1: the summary line does not say 2 disks and 1 CD drive"
    hasA '^  ide0: primary master \(0x1f0\): QEMU HARDDISK, 16384 sectors \(8 MiB\)  \(blank: claimdisk ide to use it\)$' ||
        _f "boot 1: lside before the claim did not list the blank disk with its model"
    hasA '^claimdisk: the blank ide disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk ide did not take the blank disk"
    hasA '^  ide0: primary master \(0x1f0\): QEMU HARDDISK, 16384 sectors \(8 MiB\)  <- storage$' || _f "boot 1: lside after the claim does not mark the disk as storage"
    hasA '^  ide2: secondary master \(0x170\): QEMU HARDDISK, 16384 sectors \(8 MiB\)  \(other data, read-only to us\)$' ||
        _f "boot 1: lside does not list the MBR disk as read-only"
    n="$(printf '%s\n' "$A" | grep -aE '^  storage: ' | tail -1)"
    if [[ "$n" =~ $RE_S ]]; then
        [ "${BASH_REMATCH[2]}" -ge 98 ] || _f "boot 1: only ${BASH_REMATCH[2]} sectors written (claim + write + sync is at least two saves of 49)"
        [ "${BASH_REMATCH[3]}" -ge 2 ] || _f "boot 1: only ${BASH_REMATCH[3]} cache flush(es) (one per save)"
    else
        _f "boot 1: lside has no 'storage: ...' line after the sync"
    fi
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the IDE disk"
    hasB '^\[ide\] primary master \(0x1f0\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed disk was not recognised as ours"
    hasB '^\[ide\] secondary master \(0x170\): .*holds other data \(Windows\?\): left alone$' || _f "boot 2: the MBR disk was not left alone"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the IDE disk"
    hasB "^$word$" || _f "boot 2: notes.txt did not come back from the IDE disk"
    n="$(printf '%s\n' "$B" | grep -aE '^  storage: ' | tail -1)"
    if [[ "$n" =~ $RE_S ]]; then
        [ "${BASH_REMATCH[1]}" -ge 37 ] || _f "boot 2: only ${BASH_REMATCH[1]} sectors read (18 files are 37)"
    else
        _f "boot 2: lside has no 'storage: ...' line"
    fi
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[ide] controller 00:03.0 (IDE mode, interface 0x80: primary compatibility, secondary compatibility)
[ide] primary master (0x1f0): ATA disk, 16384 sectors (8 MiB), LBA48, blank: not used unless claimed (claimdisk ide)
[ide] primary slave (0x1f0): CD/DVD drive (ATAPI): not used
[ide] secondary master (0x170): ATA disk, 16384 sectors (8 MiB), LBA48, holds other data (Windows?): left alone
[ide] 1 controller(s), 2 disk(s), 1 CD/DVD drive(s)
afrisovera> lside
ide: controller 00:03.0, interface 0x80
  ide0: primary master (0x1f0): QEMU HARDDISK, 16384 sectors (8 MiB)  (blank: claimdisk ide to use it)
  primary slave (0x1f0): CD/DVD drive (ATAPI), not used
  ide2: secondary master (0x170): QEMU HARDDISK, 16384 sectors (8 MiB)  (other data, read-only to us)
afrisovera> claimdisk ide
claimdisk: the blank ide disk now holds AfriSovera's files
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-IDE-10NOV
[vfs] synced 18 files to disk
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> lside
ide: controller 00:03.0, interface 0x80
  ide0: primary master (0x1f0): QEMU HARDDISK, 16384 sectors (8 MiB)  <- storage
  primary slave (0x1f0): CD/DVD drive (ATAPI), not used
  ide2: secondary master (0x170): QEMU HARDDISK, 16384 sectors (8 MiB)  (other data, read-only to us)
  storage: 2 sector(s) read, 147 written, 3 cache flush(es)"
B2="[ide] controller 00:03.0 (IDE mode, interface 0x80: primary compatibility, secondary compatibility)
[ide] primary master (0x1f0): ATA disk, 16384 sectors (8 MiB), LBA48, ours (AFRIVFS1): used for storage
[ide] primary slave (0x1f0): CD/DVD drive (ATAPI): not used
[ide] secondary master (0x170): ATA disk, 16384 sectors (8 MiB), LBA48, holds other data (Windows?): left alone
[ide] 1 controller(s), 2 disk(s), 1 CD/DVD drive(s)
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-IDE-10NOV
afrisovera> lside
  storage: 51 sector(s) read, 0 written, 0 cache flush(es)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ide-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" HELLO-IDE-10NOV > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm no-controller red "$(printf '%s\n' "$B1" | grep -v '^\[ide\] controller')" "$B2"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/LBA48, blank: not used unless claimed (claimdisk ide)/LBA48, ours (AFRIVFS1): used for storage/')" "$B2"
    arm other-disk-used red "$(printf '%s\n' "$B1" | sed 's/LBA48, holds other data (Windows?): left alone/LBA48, blank: not used unless claimed (claimdisk ide)/')" "$B2"
    arm cd-missed red "$(printf '%s\n' "$B1" | grep -v 'ATAPI): not used')" "$B2"
    arm claim-refused red "$(printf '%s\n' "$B1" | sed 's/^claimdisk: the blank ide disk now holds.*/claimdisk: no blank ide disk to claim (disks holding data are never touched)/')" "$B2"
    arm no-flush red "$(printf '%s\n' "$B1" | sed 's/3 cache flush(es)/0 cache flush(es)/')" "$B2"
    arm few-writes red "$(printf '%s\n' "$B1" | sed 's/147 written/49 written/')" "$B2"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm not-ours-at-boot2 red "$B1" "$(printf '%s\n' "$B2" | sed 's/LBA48, ours (AFRIVFS1): used for storage/LBA48, blank: not used unless claimed (claimdisk ide)/')"
    arm write-failed red "$B1" "$(printf '%s\n[ide] primary master: write of sector 3 failed (status 0x51, error 0x4)' "$B2")"
    arm model-missing red "$(printf '%s\n' "$B1" | sed 's/QEMU HARDDISK, 16384 sectors (8 MiB)  <- storage/, 16384 sectors (8 MiB)  <- storage/')" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC IDE SATA disks in IDE mode (PC track step 3) =="
if ! selftest; then echo "PC IDE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC IDE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC IDE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC IDE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-ide.XXXXXX")"
IMG="$W/blank.img"
OTH="$W/other.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB disk
# somebody's disk: an MBR with one partition entry and the 0x55AA signature, a few bytes further in
python3 - "$OTH" <<'EOF'
import sys
d = bytearray(8 * 1024 * 1024)
d[0:16] = b"OTHER-SYSTEM-MBR"
d[446:462] = bytes([0x80, 0, 2, 0, 0x07, 0, 0, 0, 0x00, 0x08, 0, 0, 0x00, 0x38, 0, 0])
d[510:512] = b"\x55\xaa"
d[2048 * 512:2048 * 512 + 13] = b"NTFS-ISH-DATA"
open(sys.argv[1], "wb").write(d)
EOF
SUM0="$(md5sum < "$OTH")"

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none -device piix3-ide,id=pide \
        -drive "file=$IMG,if=none,format=raw,id=ib" -device ide-hd,drive=ib,bus=pide.0,unit=0 \
        -device ide-cd,bus=pide.0,unit=1 \
        -drive "file=$OTH,if=none,format=raw,id=io" -device ide-hd,drive=io,bus=pide.1,unit=0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

WORD="HELLO-IDE-10NOV"
echo "  --- boot 1: lside, claimdisk ide, write notes.txt, sync, lside ---"
boot 5474 "$W/b1.log" "lside" "claimdisk ide" "write notes.txt $WORD" "sync" "lside"
echo "  --- boot 2: the same disks, cat notes.txt, lside ---"
boot 5476 "$W/b2.log" "cat notes.txt" "lside"
cat "$W/b1.log" "$W/b2.log" | tr -d '\r' | grep -aE '^\[ide\]|^ide:|^  (ide[0-9]|primary|secondary|storage)|^\[vfs\]|^claimdisk|HELLO|blk self-test' | sed 's/^/  | /'
echo "  --- grading ---"
ok=0
grade "$W/b1.log" "$W/b2.log" "$WORD" || ok=1
if [ "$(md5sum < "$OTH")" != "$SUM0" ]; then
    echo "  FAIL: the MBR disk (secondary master) changed: somebody's data was written"
    ok=1
fi
if [ "$ok" -eq 0 ]; then
    rm -rf "$W"
    echo "PC IDE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC IDE TEST FAILED"
exit 1
