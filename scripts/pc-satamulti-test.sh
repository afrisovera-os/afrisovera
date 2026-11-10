#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): multi-block reads and gathered writes on SATA (AHCI) disks.
#
# ahci.rs sent one READ DMA EXT or WRITE DMA EXT per sector (and on a 4Kn drive read the sector,
# changed 512 bytes and wrote it back for every 512-byte piece). Now, as NVMe and the USB disks
# got on 9-10 Nov, a read on the storage disk fetches up to 8 KiB with one command into a
# read-ahead, and the VFS's in-order save is gathered by device block into runs of up to 8 KiB
# sent with one WRITE DMA EXT; each way is checked once against one sector at a time first (reads
# when the disk is taken for storage, writes at the first save, read back).
# Two boots of the PC kernel (q35 + UEFI) with a blank 8 MiB SATA disk on AHCI port 2 next to the
# UEFI boot disk (port 0). QEMU's IDE/AHCI disks only take 512-byte logical sectors, so this is the
# 16-sectors-per-command case (the 4Kn path shares its code with the NVMe one, suite pc-nvmemulti).
# Boot 1: claimdisk sata, write a file, sync, lsahci. Boot 2: cat the file, lsahci. Graded: the
# "on" lines, at most one write command per 8 sectors saved, at most one read command per 4
# sectors read, the counts add up, the file kept across the power-off, the boot disk untouched,
# nothing refused or failed in the AHCI driver, and lsahci naming the claimed disk's model (it was
# empty after claimdisk sata until 10 Nov 2026).
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-satamulti-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-satamulti-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[ahci\] .*(failed|timed out|busy|refused)|multi-block (reads|writes) off|blk self-test FAILED'
RE_W='^  writes: ([0-9]+) sector\(s\), ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'
RE_R='^  reads: ([0-9]+) sector\(s\), ([0-9]+) from the read-ahead, ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'

grade() { # grade <boot1 log> <boot2 log> <word>
    local fail=0 A B n word=$3
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[ahci\] port 0: SATA disk, [0-9]+ sectors .*left alone$' || _f "boot 1: the boot disk on port 0 was not found, or not left alone"
    hasA '^\[ahci\] port 2: SATA disk, 16384 sectors \(8 MiB\), blank: not used unless claimed \(claimdisk sata\)$' ||
        _f "boot 1: the blank 8 MiB disk on port 2 was not identified (and kept unused until claimed)"
    hasA '^claimdisk: the blank sata disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk sata did not take the blank disk"
    hasA '^  storage disk model: QEMU HARDDISK$' || _f "boot 1: lsahci did not name the claimed disk's model (QEMU HARDDISK)"
    hasA '^\[ahci\] port 2: multi-block reads on \(up to 16 sectors per command; the first 16 equal one at a time\)$' ||
        _f "boot 1: multi-block reads were not switched on at the claim"
    hasA '^\[ahci\] port 2: multi-block writes on \(up to 16 sectors per command; the first 16 read back equal\)$' ||
        _f "boot 1: multi-block writes were not switched on at the first save"
    hasA '^\[ahci\] port 0: multi-block' && _f "boot 1: the boot disk on port 0 was checked as if it were storage"
    hasB '^\[ahci\] port 2: SATA disk, 16384 sectors \(8 MiB\), ours \(AFRIVFS1\): used for storage$' ||
        _f "boot 2: the claimed disk was not recognised as ours"
    hasB '^\[ahci\] port 2: multi-block reads on \(up to 16 sectors per command' || _f "boot 2: multi-block reads were not switched on at start-up"
    n="$(printf '%s\n' "$A" | grep -aE '^  writes: ' | tail -1)"
    if [[ "$n" =~ $RE_W ]]; then
        local w=${BASH_REMATCH[1]} wm=${BASH_REMATCH[2]} w1=${BASH_REMATCH[3]}
        [ "$w" -ge 98 ] || _f "boot 1: only $w sectors saved (claim + write + sync is at least two saves of 49)"
        [ "$wm" -ge 2 ] || _f "boot 1: only $wm multi-block write(s)"
        [ $((wm + w1)) -le $((w / 8)) ] || _f "boot 1: $w sectors took $((wm + w1)) write commands (want at most one per 8 sectors)"
    else
        _f "boot 1: lsahci has no 'writes: ... (on)' line after the sync"
    fi
    n="$(printf '%s\n' "$B" | grep -aE '^  reads: ' | tail -1)"
    if [[ "$n" =~ $RE_R ]]; then
        local r=${BASH_REMATCH[1]} ra=${BASH_REMATCH[2]} rm=${BASH_REMATCH[3]} r1=${BASH_REMATCH[4]}
        [ "$r" -ge 37 ] || _f "boot 2: only $r sectors read (18 files are 37)"
        [ $((ra + rm + r1)) -eq "$r" ] || _f "boot 2: the read counts do not add up ($ra + $rm + $r1 != $r)"
        [ $((rm + r1)) -le $((r / 4)) ] || _f "boot 2: $r sectors took $((rm + r1)) read commands (want at most a quarter)"
    else
        _f "boot 2: lsahci has no 'reads: ... (on)' line"
    fi
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the SATA disk"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the SATA disk"
    hasB "^$word$" || _f "boot 2: notes.txt did not come back from the SATA disk"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[ahci] port 2: SATA disk, 16384 sectors (8 MiB), blank: not used unless claimed (claimdisk sata)
[ahci] controller 00:1f.2 (AHCI 1.0), 6 port(s), 2 disk(s)
afrisovera> claimdisk sata
[ahci] port 2: multi-block reads on (up to 16 sectors per command; the first 16 equal one at a time)
claimdisk: the blank sata disk now holds AfriSovera's files
[ahci] port 2: multi-block writes on (up to 16 sectors per command; the first 16 read back equal)
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-SATA-MULTI-10NOV
[vfs] synced 18 files to disk
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> lsahci
ahci: controller 00:1f.2 ABAR 0xfebd5000, AHCI 1.0, ports implemented 0x3f
  port 0: 1032192 sectors (504 MiB)  (other data, read-only to us)
  port 2: 16384 sectors (8 MiB)  <- storage
  storage disk model: QEMU HARDDISK
  reads: 0 sector(s), 0 from the read-ahead, 0 multi-block command(s), 0 single (on)
  writes: 147 sector(s), 9 multi-block command(s), 3 single (on)"
B2="[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[ahci] port 2: SATA disk, 16384 sectors (8 MiB), ours (AFRIVFS1): used for storage
[ahci] controller 00:1f.2 (AHCI 1.0), 6 port(s), 2 disk(s)
[ahci] port 2: multi-block reads on (up to 16 sectors per command; the first 16 equal one at a time)
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-SATA-MULTI-10NOV
afrisovera> lsahci
  reads: 49 sector(s), 46 from the read-ahead, 3 multi-block command(s), 0 single (on)
  writes: 0 sector(s), 0 multi-block command(s), 0 single (on)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-satamulti-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" HELLO-SATA-MULTI-10NOV > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm reads-off red "$(printf '%s\n' "$B1" | sed 's/^\[ahci\] port 2: multi-block reads on .*/[ahci] port 2: multi-block reads off (the disk refused one): one sector per command/')" "$B2"
    arm no-writes-line red "$(printf '%s\n' "$B1" | grep -v 'multi-block writes on')" "$B2"
    arm write-refused-later red "$(printf '%s\n[ahci] a multi-block write at sector 16 was refused: one sector per command from now on' "$B1")" "$B2"
    arm boot-disk-checked red "$(printf '%s\n[ahci] port 0: multi-block reads on (up to 16 sectors per command; the first 16 equal one at a time)' "$B1")" "$B2"
    arm per-sector-writes red "$(printf '%s\n' "$B1" | sed 's/9 multi-block command(s), 3 single/0 multi-block command(s), 147 single/')" "$B2"
    arm single-reads red "$B1" "$(printf '%s\n' "$B2" | sed 's/46 from the read-ahead, 3 multi-block command(s), 0 single/0 from the read-ahead, 0 multi-block command(s), 49 single/')"
    arm counts-wrong red "$B1" "$(printf '%s\n' "$B2" | sed 's/49 sector(s)/60 sector(s)/')"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm blank-taken-unasked red "$(printf '%s\n' "$B1" | sed 's/blank: not used unless claimed (claimdisk sata)/ours (AFRIVFS1): used for storage/')" "$B2"
    arm model-empty red "$(printf '%s\n' "$B1" | sed 's/^  storage disk model: .*/  storage disk model: /')" "$B2"
    arm command-failed red "$B1" "$(printf '%s\n[ahci] port 2 command 0x25 lba 3 failed (IS 0x40000000, TFD 0x451)' "$B2")"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC SATAMULTI multi-block reads and gathered writes on SATA (PC track step 3) =="
if ! selftest; then echo "PC SATAMULTI TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC SATAMULTI SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC SATAMULTI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC SATAMULTI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-satamulti.XXXXXX")"
IMG="$W/sata.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB SATA disk

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device ide-hd,drive=sd,bus=ide.2 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}

WORD="HELLO-SATA-MULTI-10NOV"
echo "  --- boot 1: claimdisk sata, write notes.txt, sync, lsahci ---"
boot 5470 "$W/b1.log" "claimdisk sata" "write notes.txt $WORD" "sync" "lsahci"
echo "  --- boot 2: the same disk, cat notes.txt, lsahci ---"
boot 5472 "$W/b2.log" "cat notes.txt" "lsahci"
cat "$W/b1.log" "$W/b2.log" | tr -d '\r' | grep -aE '^\[ahci\]|^  (port|reads|writes|storage)|^\[vfs\]|HELLO|blk self-test' | sed 's/^/  | /'
echo "  --- grading ---"
if grade "$W/b1.log" "$W/b2.log" "$WORD"; then
    rm -rf "$W"
    echo "PC SATAMULTI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC SATAMULTI TEST FAILED"
exit 1
