#!/usr/bin/env bash
# PC track (12 Nov 2026): a UAS disk on a high-speed port (no streams) keeps working after a read it
# refuses, with no recovery needed.
#
# Without streams the driver sends the command IU and waits on the status pipe: data moves only
# after READ READY, and a SENSE IU with the command's tag ends the command. When the device refuses
# a read before any data, READ READY never comes and the SENSE IU (not GOOD) is the whole answer, so
# nothing is left queued and no reset or flush may follow. Until now that path was proved only by
# reading. This suite makes QEMU refuse one read: the disk's image sits behind QEMU's blkdebug
# driver, which fails the first read that touches sector 8 with an I/O error (once per QEMU run).
# That read is the driver's start-up check of multi-block reads (READ(10) of 16 blocks from block
# 0). Graded: the boot line says multi-block reads are off because the device refused one, lsusb
# shows 0 recoveries (the refusal was an ordinary answer, not a fault), and the disk still works
# one block per command: boot 1 writes a file and syncs, the image starts with the VFS's mark, boot
# 2 (the same fault again) reads the file back. The same fault over streams is suite pc-uasfault;
# the same disk with no fault is suite pc-uasdisk.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-uasfaulths-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-uasfaulths-test.sh --selftest   # grader selftest only
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
        _f "boot 1: the UAS-only disk was not driven over UAS without streams"
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} ".*", 16384 blocks of 512 B \(8 MiB\), blank or ours: used for storage$' ||
        _f "boot 1: the blank UAS disk was not identified and chosen for storage"
    hasA '^\[xhci\] USB storage: multi-block reads off \(the device refused one\): one block per command$' ||
        _f "boot 1: the injected read error did not reach the driver as a refused multi-block read"
    hasA '^  storage transport: UAS \(USB Attached SCSI, four pipes, no streams\), 0 recoveries$' ||
        _f "boot 1: lsusb does not show 0 recoveries (a refused read without streams needs none)"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the UAS disk after the refused read"
    hasB '^\[xhci\] USB storage: multi-block reads off \(the device refused one\): one block per command$' ||
        _f "boot 2: the injected read error was not refused again"
    hasB '^\[vfs\] VFS restored from disk: [0-9]+ files$' || _f "boot 2: the VFS was not restored from the UAS disk"
    hasB '^HELLO-UAS-FAULTHS-12NOV$' || _f "boot 2: notes.txt did not come back from the UAS disk"
    hasB '^  storage transport: UAS \(USB Attached SCSI, four pipes, no streams\), 0 recoveries$' ||
        _f "boot 2: lsusb does not show 0 recoveries after reading the file back"
    [ "$3" = AFRIVFS1 ] || _f "the UAS disk's image does not start with AFRIVFS1 after boot 1 (it holds '$3')"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not report its capacity|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

B1="[xhci] port 5: USB disk 46f4:0003 speaks only UAS: driven over USB Attached SCSI (high speed, four pipes, no streams)
[xhci] port 5: USB storage 46f4:0003 \"QEMU    QEMU HARDDISK\", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage
[xhci] USB storage: multi-block reads off (the device refused one): one block per command
[vfs] synced 17 files to disk
afrisovera> lsusb
  storage: slot 1, 16384 blocks of 512 B  <- storage
  storage transport: UAS (USB Attached SCSI, four pipes, no streams), 0 recoveries
afrisovera> write notes.txt HELLO-UAS-FAULTHS-12NOV
[vfs] synced 18 files to disk"
B2="[xhci] USB storage: multi-block reads off (the device refused one): one block per command
[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
HELLO-UAS-FAULTHS-12NOV
afrisovera> lsusb
  storage transport: UAS (USB Attached SCSI, four pipes, no streams), 0 recoveries"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-uasfaulths-selftest.XXXXXX")" || return 1
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
    arm no-fault red "$(printf '%s\n' "$B1" | sed 's/^\[xhci\] USB storage: multi-block reads off.*/[xhci] USB storage: multi-block reads on (READ(10) of up to 16 blocks per command; the first 16 equal one at a time)/')" "$B2" AFRIVFS1
    arm fault-differed red "$(printf '%s\n' "$B1" | sed 's/(the device refused one)/(they differed from one block at a time)/')" "$B2" AFRIVFS1
    arm streams red "$(printf '%s\n' "$B1" | sed 's/(high speed, four pipes, no streams)$/(SuperSpeed, four pipes, streams 1 and 2)/')" "$B2" AFRIVFS1
    arm recoveries-1 red "$(printf '%s\n' "$B1" | sed 's/, 0 recoveries/, 1 recoveries/')" "$B2" AFRIVFS1
    arm boot2-recoveries-2 red "$B1" "$(printf '%s\n' "$B2" | sed 's/, 0 recoveries/, 2 recoveries/')" AFRIVFS1
    arm lsusb-streams red "$(printf '%s\n' "$B1" | sed 's/four pipes, no streams), 0/four pipes, streams 1 and 2), 0/')" "$B2" AFRIVFS1
    arm not-used red "$(printf '%s\n' "$B1" | sed 's/blank or ours: used for storage/holds other data: left alone/')" "$B2" AFRIVFS1
    arm no-sync red "$(printf '%s\n' "$B1" | grep -v 'synced')" "$B2" AFRIVFS1
    arm boot2-no-fault red "$B1" "$(printf '%s\n' "$B2" | grep -v 'multi-block')" AFRIVFS1
    arm not-restored red "$B1" "$(printf '%s\n' "$B2" | grep -v 'restored')" AFRIVFS1
    arm file-missing red "$B1" "$(printf '%s\n' "$B2" | grep -v '^HELLO')" AFRIVFS1
    arm image-not-ours red "$B1" "$B2" "zeros"
    arm xhci-timed-out red "$(printf '%s\n%s\n' "$B1" '[xhci] port 5: USB storage command timed out')" "$B2" AFRIVFS1
    arm read-failed red "$B1" "$(printf '%s\n%s\n' "$B2" '[xhci] USB storage: a multi-block read at block 0 failed: one block per command from now on')" AFRIVFS1
    arm empty red "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 17 ]
}

echo "== PC UASFAULTHS refused read without streams (PC track step 3) =="
if ! selftest; then echo "PC UASFAULTHS TEST FAILED (grader selftest), no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC UASFAULTHS SELFTEST ONLY, no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC UASFAULTHS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC UASFAULTHS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-uasfaulths.XXXXXX")"
IMG="$W/uas.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB UAS-only disk
# blkdebug: the first read that touches sector 8 fails with EIO (once per QEMU run); scsi-hd
# reports it to the host (rerror=report) as CHECK CONDITION, MEDIUM ERROR
cat > "$W/fault.cfg" << 'EOF'
[inject-error]
event = "read_aio"
errno = "5"
sector = "8"
once = "on"
EOF

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    # p3=0: the controller has no USB 3 ports, so the disk sits on a USB 2 port at high speed
    qemu-system-aarch64 -M virt -m 256M -display none \
        -device qemu-xhci,id=xhci,p3=0 \
        -drive "file=blkdebug:$W/fault.cfg:$IMG,if=none,format=raw,id=ud" -device usb-uas,id=uas,bus=xhci.0 \
        -device scsi-hd,bus=uas.0,scsi-id=0,lun=0,drive=ud,rerror=report \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$((sp+1))" "$W/shot.png" "$@" > "$log" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: one read refused, then lsusb, write notes.txt, sync ---"
boot 7360 "$W/boot1.log" "lsusb" "write notes.txt HELLO-UAS-FAULTHS-12NOV" "sync"
head8="$(head -c 8 "$IMG" | tr -c 'A-Za-z0-9' '.')"
echo "  --- boot 2: the same disk and fault, cat notes.txt, lsusb ---"
boot 7362 "$W/boot2.log" "cat notes.txt" "lsusb"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' | grep -aE '^\[xhci\] .*(port|storage|UAS)|^  storage|^\[vfs\]|HELLO' | sed 's/^/  | /'
echo "  UAS disk image starts with: $head8"

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log" "$head8"; then
    rm -rf "$W"
    echo "PC UASFAULTHS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC UASFAULTHS TEST FAILED"
exit 1
