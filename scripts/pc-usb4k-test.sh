#!/usr/bin/env bash
# PC track step 3 (10 Nov 2026): gathered writes on a USB disk with 4 KiB blocks.
#
# Many newer USB SSDs and big external disks report 4096-byte logical blocks (4Kn). The VFS
# saves 512-byte sectors, so the driver used to read the 4 KiB block, change one sector and write
# the whole block back for EVERY sector: 8 writes of the same block. Now the in-order save is
# gathered by device blocks (each read once, its sectors changed in place) into runs of up to
# 8 KiB (2 blocks) sent with one WRITE(10), the same checks as on 512-byte sticks first.
# Boots the PC kernel twice with a blank 8 MiB USB disk of 4096-byte blocks (QEMU usb-storage
# with logical_block_size=4096). Boot 1: both checks pass, a file is written and synced, lsusb
# counts the commands. Boot 2: the file comes back. Graded: the 4096 B identification, the "on"
# lines (2 blocks per command), the counts (at most one write command per 8 sectors), the file
# kept across the power-off, nothing failed in the xHCI driver.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-usb4k-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-usb4k-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)|multi-block (reads|writes) off|blk self-test FAILED'
RE_W='^  storage writes: ([0-9]+) sector\(s\), ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'
RE_R='^  storage reads: ([0-9]+) sector\(s\), ([0-9]+) from the read-ahead, ([0-9]+) multi-block command\(s\), ([0-9]+) single \(on\)$'

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B n
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 2048 blocks of 4096 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "boot 1: the blank USB stick was not identified and chosen for storage"
    hasA '^\[xhci\] USB storage: multi-block reads on \(READ\(10\) of up to 2 blocks per command; the first 2 equal one at a time\)$' ||
        _f "boot 1: multi-block reads were not switched on at start-up"
    hasA '^\[xhci\] USB storage: multi-block writes on \(WRITE\(10\) of up to 2 blocks per command; the first 2 read back equal\)$' ||
        _f "boot 1: multi-block writes were not switched on at the first save"
    hasB '^\[xhci\] USB storage: multi-block reads on ' || _f "boot 2: multi-block reads were not switched on"
    n="$(printf '%s\n' "$A" | grep -aE '^  storage writes: ' | tail -1)"
    if [[ "$n" =~ $RE_W ]]; then
        local w=${BASH_REMATCH[1]} wm=${BASH_REMATCH[2]} w1=${BASH_REMATCH[3]}
        [ "$w" -ge 49 ] || _f "boot 1: only $w sectors saved (one save is 49)"
        [ "$wm" -ge 2 ] || _f "boot 1: only $wm multi-block write(s)"
        [ $((wm + w1)) -le $((w / 8)) ] || _f "boot 1: $w sectors took $((wm + w1)) write commands (want at most one per 8 sectors: one per 4 KiB block)"
    else
        _f "boot 1: lsusb has no 'storage writes: ... (on)' line after the sync"
    fi
    n="$(printf '%s\n' "$B" | grep -aE '^  storage reads: ' | tail -1)"
    if [[ "$n" =~ $RE_R ]]; then
        local r=${BASH_REMATCH[1]} ra=${BASH_REMATCH[2]} rm=${BASH_REMATCH[3]} r1=${BASH_REMATCH[4]}
        [ "$r" -ge 37 ] || _f "boot 2: only $r sectors read (18 files are 37)"
        [ $((ra + rm + r1)) -eq "$r" ] || _f "boot 2: the read counts do not add up ($ra + $rm + $r1 != $r)"
        [ $((rm + r1)) -le $((r / 4)) ] || _f "boot 2: $r sectors took $((rm + r1)) read commands (want at most a quarter)"
    else
        _f "boot 2: lsusb has no 'storage reads: ... (on)' line"
    fi
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the stick"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the stick"
    hasB '^HELLO-USB-4K-10NOV$' || _f "boot 2: notes.txt did not come back from the stick"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 1: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 2048 blocks of 4096 B (8 MiB), blank or ours: used for storage
[xhci] USB storage: multi-block reads on (READ(10) of up to 2 blocks per command; the first 2 equal one at a time)
[xhci] USB storage: multi-block writes on (WRITE(10) of up to 2 blocks per command; the first 2 read back equal)
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-USB-4K-10NOV
[vfs] synced 18 files to disk
afrisovera> lsusb
  storage: slot 1, 2048 blocks of 4096 B  <- storage
  storage reads: 15 sector(s), 8 from the read-ahead, 7 multi-block command(s), 0 single (on)
  storage writes: 98 sector(s), 6 multi-block command(s), 2 single (on)"
B2="[xhci] USB storage: multi-block reads on (READ(10) of up to 2 blocks per command; the first 2 equal one at a time)
[vfs] VFS restored from disk: 18 files
[xhci] USB storage: multi-block writes on (WRITE(10) of up to 2 blocks per command; the first 2 read back equal)
[vfs] synced 18 files to disk
afrisovera> cat notes.txt
HELLO-USB-4K-10NOV
afrisovera> lsusb
  storage: slot 1, 2048 blocks of 4096 B  <- storage
  storage reads: 50 sector(s), 44 from the read-ahead, 6 multi-block command(s), 0 single (on)
  storage writes: 49 sector(s), 3 multi-block command(s), 1 single (on)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usb4k-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-10 green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm reads-off red "$(printf '%s\n' "$B1" | sed 's/multi-block reads on .*/multi-block reads off (the device refused one): one block per command/')" "$B2"
    arm no-writes-line red "$(printf '%s\n' "$B1" | grep -v 'multi-block writes on')" "$B2"
    arm writes-off-later red "$(printf '%s\n[xhci] USB storage: a multi-block write at block 16 failed: one block per command from now on' "$B1")" "$B2"
    arm single-writes red "$(printf '%s\n' "$B1" | sed 's/6 multi-block command(s), 2 single/3 multi-block command(s), 60 single/')" "$B2"
    arm per-sector-rmw red "$(printf '%s\n' "$B1" | sed 's/6 multi-block command(s), 2 single/0 multi-block command(s), 98 single/')" "$B2"
    arm single-reads red "$B1" "$(printf '%s\n' "$B2" | sed 's/44 from the read-ahead, 6 multi-block command(s), 0 single/0 from the read-ahead, 0 multi-block command(s), 50 single/')"
    arm counts-wrong red "$B1" "$(printf '%s\n' "$B2" | sed 's/50 sector(s)/60 sector(s)/')"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm stick-not-storage red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2"
    arm blocks-512 red "$(printf '%s\n' "$B1" | sed 's/2048 blocks of 4096 B (8 MiB)/16384 blocks of 512 B (8 MiB)/')" "$B2"
    arm xhci-failed red "$(printf '%s\n[xhci] command type 11 failed (completion code 4)' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC USB4K gathered writes on a 4 KiB-block USB disk (PC track step 3) =="
if ! selftest; then echo "PC USB4K TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USB4K SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USB4K TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USB4K TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usb4k.XXXXXX")"
IMG="$W/stick.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB USB stick

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device qemu-xhci,id=xhci -device usb-storage,drive=sd,bus=xhci.0,logical_block_size=4096,physical_block_size=4096 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: write notes.txt, sync, lsusb ---"
boot 4850 "$W/boot1.log" "write notes.txt HELLO-USB-4K-10NOV" "sync" "lsusb"
echo "  --- boot 2: the same disk, cat notes.txt, lsusb ---"
boot 4852 "$W/boot2.log" "cat notes.txt" "lsusb"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE 'USB storage|^  storage|^\[vfs\]|HELLO|blk self-test' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC USB4K TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USB4K TEST FAILED"
exit 1
