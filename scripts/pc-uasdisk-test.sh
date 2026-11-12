#!/usr/bin/env bash
# PC track (12 Nov 2026): a USB disk that speaks only UAS (USB Attached SCSI) is driven as the disk.
#
# Many USB 3 SSD enclosures and fast sticks speak UAS, and some offer nothing else. On a high-speed
# (USB 2) port UAS runs without streams: the command IU on the command pipe, READ READY or WRITE
# READY on the status pipe before the data, a SENSE IU at the end. Boots the PC kernel (q35 + UEFI)
# twice with a blank 8 MiB UAS-only disk (QEMU usb-uas with a scsi-hd behind it) on a qemu-xhci with
# USB 2 ports only (p3=0), so the disk comes up at high speed. Boot 1 writes a file and syncs; boot 2
# must read it back. Also graded: the boot log says the disk is driven over UAS, it is chosen for
# storage, `lsusb` lists it with the UAS transport and no recoveries, and after boot 1 the image
# starts with the VFS's mark. The UAS disk on a USB 3 port is driven with streams (suite
# pc-uasdisk3), and stays named, not driven, on a controller without streams (suite pc-usbuas).
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-uasdisk-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-uasdisk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log> <the image's first 8 bytes after boot 1>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    hasB() { printf '%s\n' "$B" | grep -aqE "$1"; }
    hasA '^\[xhci\] port [0-9]+: USB disk [0-9a-f]{4}:[0-9a-f]{4} speaks only UAS: driven over USB Attached SCSI \(high speed, four pipes, no streams\)$' ||
        _f "boot 1: the UAS-only disk was not driven over UAS"
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "boot 1: the blank UAS disk was not identified (INQUIRY, READ CAPACITY) and chosen for storage"
    hasA '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "lsusb does not list the UAS disk as storage"
    hasA '^  storage transport: UAS \(USB Attached SCSI, four pipes, no streams\), 0 recoveries$' ||
        _f "lsusb does not show the UAS transport with no recoveries"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the UAS disk"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the UAS disk"
    hasB '^HELLO-UAS-DISK-12NOV$' || _f "boot 2: notes.txt did not come back from the UAS disk"
    [ "$3" = AFRIVFS1 ] || _f "the UAS disk's image does not start with AFRIVFS1 after boot 1 (it holds '$3')"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|UAS only: needs streams|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|UAS only: needs streams|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 1: USB disk 46f4:0003 speaks only UAS: driven over USB Attached SCSI (high speed, four pipes, no streams)
[xhci] port 1: USB storage 46f4:0003 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[vfs] synced 17 files to disk
afrisovera> lsusb
  storage: slot 1, 16384 blocks of 512 B  <- storage
  storage transport: UAS (USB Attached SCSI, four pipes, no streams), 0 recoveries
afrisovera> write notes.txt HELLO-UAS-DISK-12NOV
[vfs] synced 18 files to disk"
B2="[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-UAS-DISK-12NOV"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-uasdisk-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text> <image head>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" "$5" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm expected-green green "$B1" "$B2" AFRIVFS1
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')" AFRIVFS1
    arm not-driven red "$(printf '%s\n' "$B1" | sed '1s/.*/[xhci] port 1: device 46f4:0003 (mass storage, UAS only: needs streams, which this port or controller lacks) is not one this driver uses/')" "$B2" AFRIVFS1
    arm no-uas-line red "$(printf '%s\n' "$B1" | grep -v 'speaks only UAS')" "$B2" AFRIVFS1
    arm not-used red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2" AFRIVFS1
    arm no-transport red "$(printf '%s\n' "$B1" | grep -v 'storage transport')" "$B2" AFRIVFS1
    arm recoveries red "$(printf '%s\n' "$B1" | sed 's/, 0 recoveries/, 2 recoveries/')" "$B2" AFRIVFS1
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v 'synced')" "$B2" AFRIVFS1
    arm not-restored red "$B1" "$(printf '%s\n' "$B2" | grep -v 'restored')" AFRIVFS1
    arm file-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')" AFRIVFS1
    arm image-not-ours red "$B1" "$B2" "zeros"
    arm xhci-failed red "$(printf '%s\n%s\n' "$B1" '[xhci] port 1: configure endpoint failed')" "$B2" AFRIVFS1
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

echo "== PC UASDISK driven (PC track step 3) =="
if ! selftest; then echo "PC UASDISK TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC UASDISK SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC UASDISK TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC UASDISK TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-uasdisk.XXXXXX")"
IMG="$W/uas.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB UAS-only disk

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    # p3=0: USB 2 ports only, so usb-uas comes up at high speed (UAS without streams)
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=xhci,p3=0 \
        -drive "file=$IMG,if=none,format=raw,id=ud" -device usb-uas,id=uas,bus=xhci.0 -device scsi-hd,bus=uas.0,scsi-id=0,lun=0,drive=ud \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: lsusb, write notes.txt, sync ---"
boot 7342 "$W/boot1.log" "lsusb" "write notes.txt HELLO-UAS-DISK-12NOV" "sync"
head8="$(head -c 8 "$IMG" | tr -c 'A-Za-z0-9' '.')"
echo "  --- boot 2: the same disk, cat notes.txt ---"
boot 7344 "$W/boot2.log" "cat notes.txt"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^\[xhci\] .*(port|storage|UAS)|^  storage|^\[vfs\]|HELLO' | sed 's/^/  | /'
echo "  UAS disk image starts with: $head8"

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$head8"; then
    rm -rf "$W"
    echo "PC UASDISK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC UASDISK TEST FAILED"
exit 1
