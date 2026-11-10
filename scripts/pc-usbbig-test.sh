#!/usr/bin/env bash
# PC track step 3 (9 Nov 2026): USB disks over 2 TiB (READ CAPACITY(16), READ(16), WRITE(16)).
#
# READ(10)/WRITE(10) carry a 32-bit block address, so a disk of more than 2^32 blocks (over
# 2 TiB with 512-byte blocks) answers READ CAPACITY(10) with 0xFFFFFFFF. The driver then asks
# READ CAPACITY(16) for the real size and sends every read and write as READ(16)/WRITE(16), as
# Linux does. At start-up it checks the 16-byte form at the disk's last block (written, read back,
# put back; the block a 32-bit address would wrap to must stay untouched).
# Boots the PC kernel twice with a blank 3 TiB thin USB disk (qcow2, or a sparse file) on a
# qemu-xhci. Boot 1: the size, the 16-byte line, the last-block check, multi-block READ(16) and
# WRITE(16) on, a file written and synced. Boot 2: the file comes back. Graded: those lines, the
# lsusb commands line, nothing failed in the xHCI driver.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-usbbig-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-usbbig-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

BLOCKS=6442450944 # 3 TiB of 512-byte blocks
LAST=$((BLOCKS - 1)); WRAP=$((LAST & 0xFFFFFFFF))
BAD='KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)|multi-block (reads|writes) off|16-byte command check did not pass|blk self-test FAILED'

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA "^\[xhci\] USB storage: $BLOCKS blocks, past what READ\(10\) can address: READ\(16\)/WRITE\(16\) for every read and write$" ||
        _f "boot 1: the disk's real size was not read with READ CAPACITY(16)"
    hasA "^\[xhci\] USB storage: 16-byte commands checked at block $LAST \(written, read back, put back; block $WRAP untouched\)$" ||
        _f "boot 1: the 16-byte check at the last block did not pass"
    hasA "^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} \".*\", $BLOCKS blocks of 512 B \(3145728 MiB\), blank or ours: used for storage$" ||
        _f "boot 1: the blank 3 TiB disk was not identified and chosen for storage"
    hasA '^\[xhci\] USB storage: multi-block reads on \(READ\(16\) of up to 16 blocks per command; the first 16 equal one at a time\)$' ||
        _f "boot 1: multi-block READ(16) was not switched on"
    hasA '^\[xhci\] USB storage: multi-block writes on \(WRITE\(16\) of up to 16 blocks per command; the first 16 read back equal\)$' ||
        _f "boot 1: multi-block WRITE(16) was not switched on at the first save"
    hasA '^  storage commands: READ\(16\)/WRITE\(16\), over 2\^32 blocks \(checked at the last block\)$' ||
        _f "boot 1: lsusb does not show the 16-byte commands"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the disk"
    hasB '^\[xhci\] USB storage: 16-byte commands checked at block ' || _f "boot 2: the 16-byte check did not pass"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the disk"
    hasB '^HELLO-USB-BIG-9NOV$' || _f "boot 2: notes.txt did not come back from the disk"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE "$BAD"; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE "$BAD" | head -1)"
    fi
    return "$fail"
}

B1="[xhci] USB storage: $BLOCKS blocks, past what READ(10) can address: READ(16)/WRITE(16) for every read and write
[xhci] USB storage: 16-byte commands checked at block $LAST (written, read back, put back; block $WRAP untouched)
[xhci] port 1: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", $BLOCKS blocks of 512 B (3145728 MiB), blank or ours: used for storage
[xhci] USB storage: multi-block reads on (READ(16) of up to 16 blocks per command; the first 16 equal one at a time)
[xhci] USB storage: multi-block writes on (WRITE(16) of up to 16 blocks per command; the first 16 read back equal)
[vfs] synced 17 files to disk
afrisovera> write notes.txt HELLO-USB-BIG-9NOV
[vfs] synced 18 files to disk
afrisovera> lsusb
  storage: slot 1, $BLOCKS blocks of 512 B  <- storage
  storage commands: READ(16)/WRITE(16), over 2^32 blocks (checked at the last block)
  storage reads: 1 sector(s), 1 from the read-ahead, 0 multi-block command(s), 0 single (on)
  storage writes: 98 sector(s), 6 multi-block command(s), 2 single (on)"
B2="[xhci] USB storage: $BLOCKS blocks, past what READ(10) can address: READ(16)/WRITE(16) for every read and write
[xhci] USB storage: 16-byte commands checked at block $LAST (written, read back, put back; block $WRAP untouched)
[xhci] USB storage: multi-block reads on (READ(16) of up to 16 blocks per command; the first 16 equal one at a time)
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-USB-BIG-9NOV"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbbig-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm expected-green green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm size-32bit red "$(printf '%s\n' "$B1" | grep -v 'past what READ(10)' | sed "s/$BLOCKS blocks of/4294967296 blocks of/")" "$B2"
    arm check-failed red "$(printf '%s\n' "$B1" | sed 's/^\[xhci\] USB storage: 16-byte commands checked.*/[xhci] USB storage: 16-byte command check did not pass (the write landed on a 32-bit wrapped block too): disk left alone/')" "$B2"
    arm reads-10 red "$(printf '%s\n' "$B1" | sed 's/reads on (READ(16)/reads on (READ(10)/')" "$B2"
    arm writes-10 red "$(printf '%s\n' "$B1" | sed 's/writes on (WRITE(16)/writes on (WRITE(10)/')" "$B2"
    arm no-lsusb-line red "$(printf '%s\n' "$B1" | grep -v 'storage commands:')" "$B2"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm left-alone red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2"
    arm xhci-failed red "$(printf '%s\n[xhci] command type 11 failed (completion code 4)' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== PC USBBIG USB disks over 2 TiB, 16-byte SCSI commands (PC track step 3) =="
if ! selftest; then echo "PC USBBIG TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBBIG SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBBIG TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBBIG TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbbig.XXXXXX")"
# A fresh, blank 3 TiB disk that takes almost no room: qcow2 when qemu-img is there, else sparse.
QI="$(command -v qemu-img 2> /dev/null || true)"
[ -n "$QI" ] || for c in "$QDIR/qemu-img" "$QDIR/qemu-img.exe"; do [ -x "$c" ] && { QI="$c"; break; }; done
if [ -n "$QI" ]; then
    IMG="$W/disk.qcow2"; FMT="qcow2,cache=writethrough" # every write reaches the file (QEMU is killed, not shut down)
    "$QI" create -q -f qcow2 "$IMG" 3T || { echo "PC USBBIG TEST FAILED (harness: qemu-img create)"; exit 1; }
else
    IMG="$W/disk.img"; FMT=raw
    truncate -s 3T "$IMG" || { echo "PC USBBIG TEST FAILED (harness: no qemu-img and no sparse file)"; exit 1; }
fi

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=$FMT,id=sd" -device qemu-xhci,id=xhci -device usb-storage,drive=sd,bus=xhci.0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: write notes.txt, sync, lsusb ---"
boot 4828 "$W/boot1.log" "write notes.txt HELLO-USB-BIG-9NOV" "sync" "lsusb"
echo "  --- boot 2: the same disk, cat notes.txt, lsusb ---"
boot 4830 "$W/boot2.log" "cat notes.txt" "lsusb"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE 'USB storage|^  storage|^\[vfs\]|HELLO|blk self-test' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC USBBIG TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBBIG TEST FAILED"
exit 1
