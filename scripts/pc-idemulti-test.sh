#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): multi-sector PIO reads and gathered writes on SATA disks in IDE mode.
#
# ide.rs sent one READ SECTORS or WRITE SECTORS per 512-byte sector (installdisk-ide's boot 2 wrote
# 15933 sectors with 15933 commands). Now, as AHCI, NVMe and the USB disks got on 9-10 Nov, a read
# on the storage drive fetches up to 16 sectors with one command (sector count 16, one DRQ block per
# sector) into a read-ahead, and the VFS's in-order save is gathered into runs of up to 16 sectors
# sent with one WRITE SECTORS; each way is checked once against one sector at a time first (reads
# when the drive is taken for storage, writes at the first save, read back).
# Two boots of the PC kernel (q35 + UEFI) with QEMU's piix3-ide: primary master a blank 8 MiB disk,
# secondary master an 8 MiB disk with an MBR (somebody's data). Boot 1: claimdisk ide, write a
# file, sync, lside. Boot 2: cat the file, lside. Graded: the "on" lines, at most one write command
# per 8 sectors saved, at most one read command per 4 sectors read, the counts add up, the file kept
# across the power-off, the other disk never checked as storage and byte for byte the same (checked
# on the image), nothing refused or failed in the IDE driver.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-idemulti-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-idemulti-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[ide\] .*(failed|still busy|refused|could not be read)|multi-sector (reads|writes) off|blk self-test FAILED'
RE_W='^  writes: ([0-9]+) sector\(s\), ([0-9]+) multi-sector command\(s\), ([0-9]+) single \(on\)$'
RE_R='^  reads: ([0-9]+) sector\(s\), ([0-9]+) from the read-ahead, ([0-9]+) multi-sector command\(s\), ([0-9]+) single \(on\)$'

grade() { # grade <boot1 log> <boot2 log> <word>
    local fail=0 A B n word=$3
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[ide\] primary master \(0x1f0\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), blank: not used unless claimed \(claimdisk ide\)$' ||
        _f "boot 1: the blank 8 MiB disk (primary master) was not identified, or not kept unused until claimed"
    hasA '^\[ide\] secondary master \(0x170\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), holds other data \(Windows\?\): left alone$' ||
        _f "boot 1: the MBR disk (secondary master) was not found, or not left alone"
    hasA '^claimdisk: the blank ide disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk ide did not take the blank disk"
    hasA '^\[ide\] ide0: multi-sector reads on \(up to 16 sectors per command; the first 16 equal one at a time\)$' ||
        _f "boot 1: multi-sector reads were not switched on at the claim"
    hasA '^\[ide\] ide0: multi-sector writes on \(up to 16 sectors per command; the first 16 read back equal\)$' ||
        _f "boot 1: multi-sector writes were not switched on at the first save"
    printf '%s\n%s\n' "$A" "$B" | grep -aqE '^\[ide\] ide[1-9]: multi-sector' && _f "the MBR disk was checked as if it were storage"
    hasB '^\[ide\] primary master \(0x1f0\): ATA disk, 16384 sectors \(8 MiB\), LBA(28|48), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed disk was not recognised as ours"
    hasB '^\[ide\] ide0: multi-sector reads on \(up to 16 sectors per command' || _f "boot 2: multi-sector reads were not switched on at start-up"
    n="$(printf '%s\n' "$A" | grep -aE '^  writes: ' | tail -1)"
    if [[ "$n" =~ $RE_W ]]; then
        local w=${BASH_REMATCH[1]} wm=${BASH_REMATCH[2]} w1=${BASH_REMATCH[3]}
        [ "$w" -ge 98 ] || _f "boot 1: only $w sectors saved (claim + write + sync is at least two saves of 49)"
        [ "$wm" -ge 2 ] || _f "boot 1: only $wm multi-sector write(s)"
        [ $((wm + w1)) -le $((w / 8)) ] || _f "boot 1: $w sectors took $((wm + w1)) write commands (want at most one per 8 sectors)"
    else
        _f "boot 1: lside has no 'writes: ... (on)' line after the sync"
    fi
    n="$(printf '%s\n' "$B" | grep -aE '^  reads: ' | tail -1)"
    if [[ "$n" =~ $RE_R ]]; then
        local r=${BASH_REMATCH[1]} ra=${BASH_REMATCH[2]} rm=${BASH_REMATCH[3]} r1=${BASH_REMATCH[4]}
        [ "$r" -ge 37 ] || _f "boot 2: only $r sectors read (18 files are 37)"
        [ $((ra + rm + r1)) -eq "$r" ] || _f "boot 2: the read counts do not add up ($ra + $rm + $r1 != $r)"
        [ $((rm + r1)) -le $((r / 4)) ] || _f "boot 2: $r sectors took $((rm + r1)) read commands (want at most a quarter)"
    else
        _f "boot 2: lside has no 'reads: ... (on)' line"
    fi
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the IDE disk"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the IDE disk"
    hasB "^$word$" || _f "boot 2: notes.txt did not come back from the IDE disk"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[ide] controller 00:03.0 (IDE mode, interface 0x80: primary compatibility, secondary compatibility)
[ide] primary master (0x1f0): ATA disk, 16384 sectors (8 MiB), LBA48, blank: not used unless claimed (claimdisk ide)
[ide] secondary master (0x170): ATA disk, 16384 sectors (8 MiB), LBA48, holds other data (Windows?): left alone
[ide] 1 controller(s), 2 disk(s), 0 CD/DVD drive(s)
afrisovera> claimdisk ide
[ide] ide0: multi-sector reads on (up to 16 sectors per command; the first 16 equal one at a time)
claimdisk: the blank ide disk now holds AfriSovera's files
[ide] ide0: multi-sector writes on (up to 16 sectors per command; the first 16 read back equal)
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-IDE-MULTI-10NOV
[vfs] synced 18 files to disk
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> lside
  ide0: primary master (0x1f0): QEMU HARDDISK, 16384 sectors (8 MiB)  <- storage
  storage: 34 sector(s) read, 147 written, 3 cache flush(es)
  reads: 0 sector(s), 0 from the read-ahead, 0 multi-sector command(s), 0 single (on)
  writes: 147 sector(s), 9 multi-sector command(s), 3 single (on)"
B2="[ide] primary master (0x1f0): ATA disk, 16384 sectors (8 MiB), LBA48, ours (AFRIVFS1): used for storage
[ide] secondary master (0x170): ATA disk, 16384 sectors (8 MiB), LBA48, holds other data (Windows?): left alone
[ide] ide0: multi-sector reads on (up to 16 sectors per command; the first 16 equal one at a time)
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-IDE-MULTI-10NOV
afrisovera> lside
  reads: 49 sector(s), 46 from the read-ahead, 3 multi-sector command(s), 0 single (on)
  writes: 0 sector(s), 0 multi-sector command(s), 0 single (on)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-idemulti-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" HELLO-IDE-MULTI-10NOV > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm reads-off red "$(printf '%s\n' "$B1" | sed 's/^\[ide\] ide0: multi-sector reads on .*/[ide] ide0: multi-sector reads off (the drive refused one): one sector per command/')" "$B2"
    arm no-writes-line red "$(printf '%s\n' "$B1" | grep -v 'multi-sector writes on')" "$B2"
    arm write-refused-later red "$(printf '%s\n[ide] a multi-sector write at sector 16 was refused: one sector per command from now on' "$B1")" "$B2"
    arm other-disk-checked red "$(printf '%s\n[ide] ide2: multi-sector reads on (up to 16 sectors per command; the first 16 equal one at a time)' "$B1")" "$B2"
    arm per-sector-writes red "$(printf '%s\n' "$B1" | sed 's/9 multi-sector command(s), 3 single/0 multi-sector command(s), 147 single/')" "$B2"
    arm single-reads red "$B1" "$(printf '%s\n' "$B2" | sed 's/46 from the read-ahead, 3 multi-sector command(s), 0 single/0 from the read-ahead, 0 multi-sector command(s), 49 single/')"
    arm counts-wrong red "$B1" "$(printf '%s\n' "$B2" | sed 's/49 sector(s)/60 sector(s)/')"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/blank: not used unless claimed (claimdisk ide)/ours (AFRIVFS1): used for storage/')" "$B2"
    arm other-disk-taken red "$(printf '%s\n' "$B1" | sed 's/holds other data (Windows?): left alone/ours (AFRIVFS1): used for storage/')" "$B2"
    arm command-failed red "$B1" "$(printf '%s\n[ide] primary master: read of sector 3 failed (status 0x51, error 0x4)' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC IDEMULTI multi-sector PIO reads and gathered writes on IDE-mode disks (PC track step 3) =="
if ! selftest; then echo "PC IDEMULTI TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC IDEMULTI SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC IDEMULTI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC IDEMULTI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-idemulti.XXXXXX")"
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
        -drive "file=$OTH,if=none,format=raw,id=io" -device ide-hd,drive=io,bus=pide.1,unit=0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

WORD="HELLO-IDE-MULTI-10NOV"
echo "  --- boot 1: claimdisk ide, write notes.txt, sync, lside ---"
boot 5478 "$W/b1.log" "claimdisk ide" "write notes.txt $WORD" "sync" "lside"
echo "  --- boot 2: the same disks, cat notes.txt, lside ---"
boot 5480 "$W/b2.log" "cat notes.txt" "lside"
cat "$W/b1.log" "$W/b2.log" | tr -d '\r' | grep -aE '^\[ide\]|^  (ide[0-9]|reads|writes|storage)|^\[vfs\]|^claimdisk|HELLO|blk self-test' | sed 's/^/  | /'
echo "  --- grading ---"
ok=0
grade "$W/b1.log" "$W/b2.log" "$WORD" || ok=1
if [ "$(md5sum < "$OTH")" != "$SUM0" ]; then
    echo "  FAIL: the MBR disk (secondary master) changed: somebody's data was written"
    ok=1
fi
if [ "$ok" -eq 0 ]; then
    rm -rf "$W"
    echo "PC IDEMULTI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC IDEMULTI TEST FAILED"
exit 1
