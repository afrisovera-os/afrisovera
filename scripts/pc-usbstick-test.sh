#!/usr/bin/env bash
# PC track (5 Nov 2026): a USB stick (mass storage, Bulk-Only Transport, SCSI) as the disk.
#
# Boots the PC kernel (q35 + UEFI) twice with a blank 8 MiB USB stick (QEMU usb-storage on a
# qemu-xhci) next to the UEFI boot disk (AHCI port 0). Boot 1 writes a file and syncs; boot 2
# must read it back from the stick. Also graded: the stick is identified (INQUIRY, READ
# CAPACITY) and chosen for storage, the boot disk is left alone, and `lsusb` lists it.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-usbstick-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-usbstick-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "boot 1: the blank USB stick was not identified and chosen for storage"
    hasA '^\[ahci\] port 0: SATA disk, [0-9]+ sectors .*left alone$' || _f "boot 1: the boot disk was not found, or not left alone"
    hasA '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "lsusb does not list the stick as storage"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the stick"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the stick"
    hasB '^HELLO-USB-STICK-5NOV$' || _f "boot 2: notes.txt did not come back from the stick"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 1: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[ahci] port 0: SATA disk, 1032192 sectors (504 MiB), holds other data (boot disk?): left alone
[vfs] synced 17 files to disk
afrisovera> lsusb
  storage: slot 1, 16384 blocks of 512 B  <- storage
afrisovera> write notes.txt HELLO-USB-STICK-5NOV
[vfs] synced 18 files to disk"
B2="[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-USB-STICK-5NOV"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbstick-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm stick-not-storage red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2"
    arm boot-disk-used red "$(printf '%s\n' "$B1" | sed 's/holds other data (boot disk?): left alone/blank or ours: used for storage/')" "$B2"
    arm no-lsusb red "$(printf '%s\n' "$B1" | grep -v '  storage:')" "$B2"
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v synced)" "$B2"
    arm not-restored red "$B1" "$(printf '%s\n' "$B2" | grep -v restored)"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')"
    arm no-capacity red "$(printf '%s\n[xhci] port 1: USB storage 46f4:0001 did not report its capacity' "$B1")" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC USBSTICK disk (PC track step 3) =="
if ! selftest; then echo "PC USBSTICK TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBSTICK SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBSTICK TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBSTICK TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbstick.XXXXXX")"
IMG="$W/stick.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB USB stick

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device qemu-xhci,id=xhci -device usb-storage,drive=sd,bus=xhci.0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: lsusb, write notes.txt, sync ---"
boot 4824 "$W/boot1.log" "lsusb" "write notes.txt HELLO-USB-STICK-5NOV" "sync"
echo "  --- boot 2: the same disk, cat notes.txt ---"
boot 4826 "$W/boot2.log" "cat notes.txt"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE 'USB storage|^  storage:|^\[vfs\]|HELLO' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC USBSTICK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBSTICK TEST FAILED"
exit 1
