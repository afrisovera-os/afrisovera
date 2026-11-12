#!/usr/bin/env bash
# PC track (12 Nov 2026): a USB disk that speaks only UAS (USB Attached SCSI) is named, not used,
# when its USB 3 port's controller has no streams (UAS on USB 3 needs them).
#
# Many USB 3 SSD enclosures and fast sticks speak UAS. Most also offer Bulk-Only Transport, which
# the driver uses; one that offers only UAS used to be reported as plain "mass storage ... not one
# this driver uses", which says nothing about why. Boots the PC kernel (q35 + UEFI) once with a
# UAS-only disk (QEMU usb-uas with a scsi-hd behind it) and a blank Bulk-Only stick (usb-storage)
# on the same qemu-xhci with streams turned off (streams=off; with them the disk is driven, suite
# pc-uasdisk3). Graded: the UAS disk's boot line says it speaks only UAS and is not
# driven, the stick beside it is still chosen for storage and listed by `lsusb`, and the UAS
# disk's image is byte for byte the same after the boot.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-usbuas-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbuas-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot log> <"same" or "changed": the UAS disk's image after the boot>
    local fail=0 A
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasA '^\[xhci\] port [0-9]+: device [0-9a-f]{4}:[0-9a-f]{4} \(mass storage, UAS only: needs streams, which this port or controller lacks\) is not one this driver uses$' ||
        _f "the UAS-only disk was not named as UAS only"
    [ "$(printf '%s\n' "$A" | grep -acE '^\[xhci\] .*\(mass storage(, | of )')" -eq 1 ] ||
        _f "not exactly one mass storage device was left alone (the stick must be used)"
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "the Bulk-Only stick beside it was not chosen for storage"
    hasA '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "lsusb does not list the stick as storage"
    [ "$2" = same ] || _f "the UAS disk's image changed"
    if printf '%s\n' "$A" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$A" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 1: device 46f4:0003 (mass storage, UAS only: needs streams, which this port or controller lacks) is not one this driver uses
[xhci] port 2: USB storage 46f4:0001 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[vfs] synced 17 files to disk
afrisovera> lsusb
  storage: slot 2, 16384 blocks of 512 B  <- storage"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbuas-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <log text> <same|changed>
        local got
        printf '%s\n' "$3" > "$D/a"
        if grade "$D/a" "$4" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm expected-green green "$B1" same
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" same
    arm old-words red "$(printf '%s\n' "$B1" | sed 's/(mass storage, UAS only: needs streams, which this port or controller lacks)/(mass storage)/')" same
    arm cbi-words red "$(printf '%s\n' "$B1" | sed 's/UAS only: needs streams, which this port or controller lacks/CBI only (an old USB floppy drive): not driven/')" same
    arm stick-left-too red "$(printf '%s\n%s\n' "$B1" '[xhci] port 2: device 46f4:0001 (mass storage of a kind not driven) is not one this driver uses')" same
    arm stick-not-used red "$(printf '%s\n' "$B1" | grep -v 'USB storage')" same
    arm no-lsusb red "$(printf '%s\n' "$B1" | grep -v '  storage:')" same
    arm uas-written red "$B1" changed
    arm xhci-failed red "$(printf '%s\n%s\n' "$B1" '[xhci] port 1: configure endpoint failed')" same
    arm empty red "" same
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC USBUAS disk named (PC track step 3) =="
if ! selftest; then echo "PC USBUAS TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBUAS SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBUAS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBUAS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbuas.XXXXXX")"
UAS="$W/uas.img"
STICK="$W/stick.img"
head -c 8388608 /dev/zero > "$STICK" # a fresh, blank 8 MiB Bulk-Only stick
# the UAS disk holds other data, so a write to it would show
{ printf 'UAS-DISK-OTHER-DATA-12NOV'; head -c 16777191 /dev/zero; } > "$UAS"
before="$(md5sum "$UAS" | cut -d' ' -f1)"

echo "  --- boot: lsusb ---"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci,streams=off \
    -drive "file=$UAS,if=none,format=raw,id=ud" -device usb-uas,id=uas,bus=xhci.0 -device scsi-hd,bus=uas.0,scsi-id=0,lun=0,drive=ud \
    -drive "file=$STICK,if=none,format=raw,id=sd" -device usb-storage,drive=sd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:7340,server,nowait" -qmp "tcp:127.0.0.1:7341,server,nowait" &
q=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 7340 7341 "$W/shot.png" "lsusb" > "$W/boot.log" 2>&1
kill "$q" 2> /dev/null
wait "$q" 2> /dev/null
after="$(md5sum "$UAS" | cut -d' ' -f1)"
same=changed
[ "$before" = "$after" ] && same=same
tr -d '\r' < "$W/boot.log" | grep -aE '^\[xhci\] .*(port|storage)|^  storage:' | sed 's/^/  | /'
echo "  UAS disk image after the boot: $same"

echo "  --- grading ---"
if grade "$W/boot.log" "$same"; then
    rm -rf "$W"
    echo "PC USBUAS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBUAS TEST FAILED"
exit 1
