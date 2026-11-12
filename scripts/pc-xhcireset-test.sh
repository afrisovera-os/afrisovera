#!/usr/bin/env bash
# PC track step 3 (12 Nov 2026): a USB controller that stops working is reset and its devices
# are found again.
#
# After a Host System Error or a Host Controller Error (USBSTS bits 2 and 12, xHCI 4.24.1) an
# xHCI controller does nothing more until it is reset, so the USB keyboard and the USB stick on
# it were lost until a restart. The timer poll now watches USBSTS; the next idle moment resets
# the controller (`restart`, the boot's own `start`) and enumerates its ports again. `xhcireset`
# does it by hand; `xhcireset fault` marks the controller as stopped so the automatic path runs.
# A command that gets no completion event in 5 seconds marks the controller the same way
# (`xhcireset noanswer` puts a No Op command on the ring without ringing the doorbell).
#
# QEMU's controller never reports a fatal error, so this suite uses both commands: the PC kernel
# (q35 + UEFI) boots with a USB keyboard and a blank 8 MiB USB stick on one qemu-xhci. Every
# command is typed on the USB KEYBOARD (scripts/lib/usb-type.py), so each command after a reset
# proves the keyboard was found again. Boot 1: xhcireset, lsusb, a file written and synced to
# the stick after the reset, xhcireset fault, lsusb, xhcireset noanswer, a pause, lsusb.
# Boot 2: the file comes back from the stick.
#   scripts/pc-xhcireset-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-xhcireset-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1 log> <boot2 log>
    local fail=0 A B n
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasA() { printf '%s\n' "$A" | grep -aqE "$1"; }
    cntA() { printf '%s\n' "$A" | grep -acE "$1"; }
    hasA '^\[xhci\] port [0-9]+: USB storage [0-9a-f]{4}:[0-9a-f]{4} .*used for storage$' || _f "boot 1: the USB stick was not taken for storage"
    hasA '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} ' || _f "boot 1: the USB keyboard was not taken"
    hasA '^afrisovera> xhcireset$' || _f "xhcireset was not typed"
    hasA '^\[xhci\] controller 1: resetting it and finding its devices again \(2 device\(s\) were in use\)$' ||
        _f "xhcireset did not reset the controller with its 2 devices"
    hasA '^xhcireset: done$' || _f "xhcireset did not say done"
    hasA '^xhcireset: controller 1 marked as stopped \(test\); it is reset at the next idle moment$' ||
        _f "xhcireset fault did not mark the controller"
    # printed at the next idle moment, so it may follow the shell prompt on the same line
    hasA '^(afrisovera> )?\[xhci\] controller 1 stopped working \(a controller error, USBSTS 0x1000, set by xhcireset fault\)$' ||
        _f "the idle check did not find the stopped controller"
    hasA '^xhcireset: sending controller 1 a No Op command without ringing its doorbell \(test\)$' ||
        _f "xhcireset noanswer did not send its command"
    hasA '^\[xhci\] controller 1: command type 23 got no answer in 5 s \(USBSTS 0x[0-9a-f]+\)$' ||
        _f "the command without an answer was not noticed"
    hasA '^xhcireset: no answer; the controller is reset at the next idle moment$' || _f "xhcireset noanswer did not end"
    hasA '^(afrisovera> )?\[xhci\] controller 1 stopped working \(a command got no answer, USBSTS 0x[0-9a-f]+, set by xhcireset noanswer\)$' ||
        _f "the idle check did not reset the controller whose command got no answer"
    n="$(cntA '^\[xhci\] controller 1: resetting it and finding its devices again')"
    [ "$n" -eq 3 ] || _f "$n controller resets (3 expected: xhcireset, xhcireset fault, xhcireset noanswer)"
    n="$(cntA '^\[xhci\] controller 1 running again after the reset: 2 of 2 device\(s\) found again$')"
    [ "$n" -eq 3 ] || _f "the devices were found again after $n of the 3 resets"
    n="$(cntA '^\[xhci\] port [0-9]+: USB keyboard ')"
    [ "$n" -ge 4 ] || _f "the keyboard was taken $n times (4 expected: boot and three resets)"
    n="$(cntA '^\[xhci\] port [0-9]+: USB storage .*used for storage$')"
    [ "$n" -ge 4 ] || _f "the stick was taken for storage $n times (4 expected: boot and three resets)"
    hasA '^  controller resets \(it stopped working, or xhcireset\): 1$' || _f "lsusb after the first reset does not show 1 reset"
    hasA '^  controller resets \(it stopped working, or xhcireset\): 2$' || _f "lsusb after the second reset does not show 2 resets"
    hasA '^  controller resets \(it stopped working, or xhcireset\): 3$' || _f "lsusb after the third reset does not show 3 resets (the keyboard was not found again)"
    hasA '^  storage: slot [0-9]+, 16384 blocks of 512 B  <- storage$' || _f "lsusb does not list the stick as storage after the reset"
    hasA '^afrisovera> sync$' || _f "sync was not typed after the reset (the keyboard was not found again)"
    hasA '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the VFS never synced to the stick"
    printf '%s\n' "$B" | grep -aqE '^xhci-reset-12nov$' || _f "boot 2: the file written after the reset did not come back from the stick"
    if printf '%s\n%s\n' "$A" "$B" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not come back|stopped working again|did not come out of reset|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n%s\n' "$A" "$B" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|did not come back|stopped working again|did not come out of reset|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

KBD='[xhci] port 5: USB keyboard 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us'
STICK='[xhci] port 1: USB storage 46f4:0001 "QEMU    QEMU HARDDISK", 16384 blocks of 512 B (8 MiB), blank or ours: used for storage'
B1="$STICK
$KBD
afrisovera> xhcireset
[xhci] controller 1: resetting it and finding its devices again (2 device(s) were in use)
$STICK
$KBD
[xhci] controller 1 running again after the reset: 2 of 2 device(s) found again
xhcireset: done
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 2 connected
  controller resets (it stopped working, or xhcireset): 1
  storage: slot 1, 16384 blocks of 512 B  <- storage
afrisovera> write notes.txt xhci-reset-12nov
afrisovera> sync
[vfs] synced 18 files to disk
afrisovera> xhcireset fault
xhcireset: controller 1 marked as stopped (test); it is reset at the next idle moment
[xhci] controller 1 stopped working (a controller error, USBSTS 0x1000, set by xhcireset fault)
[xhci] controller 1: resetting it and finding its devices again (2 device(s) were in use)
$STICK
$KBD
[xhci] controller 1 running again after the reset: 2 of 2 device(s) found again
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 2 connected
  controller resets (it stopped working, or xhcireset): 2
afrisovera> xhcireset noanswer
xhcireset: sending controller 1 a No Op command without ringing its doorbell (test)
[xhci] controller 1: command type 23 got no answer in 5 s (USBSTS 0x8)
xhcireset: no answer; the controller is reset at the next idle moment
[xhci] controller 1 stopped working (a command got no answer, USBSTS 0x8, set by xhcireset noanswer)
[xhci] controller 1: resetting it and finding its devices again (2 device(s) were in use)
$STICK
$KBD
[xhci] controller 1 running again after the reset: 2 of 2 device(s) found again
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 2 connected
  controller resets (it stopped working, or xhcireset): 3"
B2="[vfs] VFS restored from disk: 18 files
afrisovera> cat notes.txt
xhci-reset-12nov"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-xhcireset-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot1 text> <boot2 text>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample-green green "$B1" "$B2"
    arm crlf green "$(printf '%s\n' "$B1" | sed 's/$/\r/')" "$(printf '%s\n' "$B2" | sed 's/$/\r/')"
    arm no-manual-reset red "$(printf '%s\n' "$B1" | grep -v '^xhcireset: done')" "$B2"
    arm one-reset red "$(printf '%s\n' "$B1" | awk '/resetting it/ && !s {s=1; next} {print}')" "$B2"
    arm after-prompt green "$(printf '%s\n' "$B1" | sed 's/^\[xhci\] controller 1 stopped working/afrisovera> &/')" "$B2"
    arm idle-check-missed red "$(printf '%s\n' "$B1" | grep -v 'stopped working (a controller error')" "$B2"
    arm devices-lost red "$(printf '%s\n' "$B1" | sed '0,/2 of 2 device/s/2 of 2 device/1 of 2 device/')" "$B2"
    arm keyboard-not-back red "$(printf '%s\n' "$B1" | awk '/USB keyboard/ {k++; if (k == 3) next} {print}')" "$B2"
    arm stick-not-back red "$(printf '%s\n' "$B1" | awk '/USB storage/ {k++; if (k == 2) next} {print}')" "$B2"
    arm no-count red "$(printf '%s\n' "$B1" | grep -v 'controller resets (it stopped working, or xhcireset): 2')" "$B2"
    arm no-answer-missed red "$(printf '%s\n' "$B1" | grep -v 'got no answer in 5 s')" "$B2"
    arm no-answer-idle-missed red "$(printf '%s\n' "$B1" | grep -v 'stopped working (a command got no answer')" "$B2"
    arm third-reset-lost red "$(printf '%s\n' "$B1" | grep -v 'controller resets (it stopped working, or xhcireset): 3')" "$B2"
    arm not-synced red "$(printf '%s\n' "$B1" | grep -v synced)" "$B2"
    arm not-persisted red "$B1" "$(printf '%s\n' "$B2" | grep -v '^xhci-reset')"
    arm did-not-come-back red "$(printf '%s\n%s\n' "$B1" '[xhci] controller 1 did not come back after the reset: its USB devices are not used until the next start')" "$B2"
    arm cmd-timeout red "$(printf '%s\n%s\n' "$B1" '[xhci] control request 0x06 timed out')" "$B2"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== PC xHCI controller reset (PC track step 3) =="
if ! selftest; then echo "PC XHCIRESET TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC XHCIRESET SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC XHCIRESET TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC XHCIRESET TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-xhcireset.XXXXXX")"
IMG="$W/stick.img"
head -c 8388608 /dev/zero > "$IMG" # a fresh, blank 8 MiB USB stick

boot() { # boot <serial port> <log> <commands typed on the USB keyboard...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$IMG,if=none,format=raw,id=sd" -device qemu-xhci,id=xhci \
        -device usb-storage,drive=sd,bus=xhci.0 -device usb-kbd,bus=xhci.0 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    python3 scripts/lib/usb-type.py "$sp" "$((sp+1))" "$log" "$@"
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: xhcireset, lsusb, write and sync, xhcireset fault, lsusb, xhcireset noanswer, lsusb ---"
boot 5180 "$W/boot1.log" "xhcireset" "lsusb" "write notes.txt xhci-reset-12nov" "sync" "xhcireset fault" "lsusb" \n    "xhcireset noanswer" "@wait 10" "lsusb"
echo "  --- boot 2: the same stick, cat notes.txt ---"
boot 5182 "$W/boot2.log" "cat notes.txt"
cat "$W/boot1.log" "$W/boot2.log" | tr -d '\r' |
    grep -aE '^\[xhci\]|^xhcireset|^afrisovera> |^usb:|^  (controller resets|storage:|keyboard:)|^\[vfs\]|^xhci-reset' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC XHCIRESET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC XHCIRESET TEST FAILED"
exit 1
