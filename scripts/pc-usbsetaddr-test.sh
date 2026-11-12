#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the device gets time to take its new address.
#
# On the HP ENVY's Intel xHCI, the descriptor read after Address Device ended with completion
# code 4 (a USB transaction error). Address Device sends SET_ADDRESS, and USB 2.0 (9.2.6.3)
# gives a device 2 ms to take the new address; the driver asked for the descriptor at once,
# which QEMU's devices answer but a real device may not. xhci.rs now waits 10 ms after every
# SET_ADDRESS (as Linux does), and the first read before the address (BSR) asks for 64 bytes,
# as Windows and Linux's new scheme do. This suite proves both paths still work end to end and
# that the wait is made once per addressed device.
#
# Boots the PC kernel with an EMPTY xHCI. Through QMP (scripts/lib/usb-steps.py): `usbaddr bsr`,
# a USB keyboard plugged in and `lsusb` typed ON IT, `usbaddr plain`, a USB mouse plugged in,
# and `usbaddr`. Graded: both enumerated, lsusb typed on the keyboard, the final set-address
# line (10 ms, 2 waits, 64 bytes first, an EP0 packet size of 8 to 512), 1 plain and 1
# descriptor first, and no failed command, time-out or "did not answer".
#   scripts/pc-usbsetaddr-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbsetaddr-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] 0 device\(s\) connected, no USB keyboard$' || _f "the boot did not start with an empty xHCI"
    has '^  set address: 10 ms for the device to take it, 0 wait\(s\) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 0$' ||
        _f "usbaddr bsr did not show the set-address line with nothing addressed yet"
    has '\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint' ||
        _f "the keyboard plugged in after usbaddr bsr (64-byte first read) was not enumerated"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' ||
        _f "lsusb typed on the BSR-addressed keyboard did not run"
    has '\[xhci\] port [0-9]+: USB mouse [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the mouse plugged in (plain, with the wait) was not enumerated"
    has '^  set address: 10 ms for the device to take it, 2 wait\(s\) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size (8|16|32|64|9|512)$' ||
        _f "the final usbaddr does not show 2 waits and a real EP0 packet size"
    has '^  addressing: 1 plain, 1 descriptor first \(BSR\), 0 answered on a later try, 0 on no try$' ||
        _f "the final usbaddr does not count 1 plain and 1 descriptor first"
    has 'did not answer|answered on try|last addressing error' && _f "a QEMU device needed a retry"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] 0 device(s) connected, no USB keyboard
afrisovera> usb: new devices are tried first with descriptor read before the address (BSR); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 0 with a device
  set address: 10 ms for the device to take it, 0 wait(s) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 0
  addressing: 0 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hot-plug: 1 plugged in, 0 unplugged since boot
  addressing: 0 plain, 1 descriptor first (BSR), 0 answered on a later try, 0 on no try
  port 5: connected, enabled, speed 3  <- keyboard
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> usb: new devices are tried first with address first (plain); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 0 with a device
  set address: 10 ms for the device to take it, 1 wait(s) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 64
  addressing: 0 plain, 1 descriptor first (BSR), 0 answered on a later try, 0 on no try
afrisovera> [xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] port 6: device plugged in and ready
afrisovera> usb: new devices are tried first with address first (plain); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 0 with a device
  set address: 10 ms for the device to take it, 2 wait(s) made; first read 64 bytes descriptor first, 8 plain; last EP0 packet size 64
  addressing: 1 plain, 1 descriptor first (BSR), 0 answered on a later try, 0 on no try"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbsetaddr-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm sample-green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm keyboard-at-boot red "$(printf '%s\n' "$GREEN" | grep -v 'no USB keyboard')"
    arm no-first-line red "$(printf '%s\n' "$GREEN" | grep -v '0 wait(s) made')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm mouse-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm one-wait red "$(printf '%s\n' "$GREEN" | sed 's/2 wait(s) made/1 wait(s) made/')"
    arm no-wait red "$(printf '%s\n' "$GREEN" | sed 's/ 10 ms for/ 0 ms for/')"
    arm read-8 red "$(printf '%s\n' "$GREEN" | sed 's/first read 64 bytes/first read 8 bytes/')"
    arm mps-zero red "$(printf '%s\n' "$GREEN" | sed '/2 wait(s) made/s/packet size 64$/packet size 0/')"
    arm final-count red "$(printf '%s\n' "$GREEN" | sed 's/addressing: 1 plain, 1 descriptor/addressing: 0 plain, 2 descriptor/')"
    arm retried red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 6: answered on try 2 (address first (plain))')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] control request 0x06 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC USB set-address wait (F2) =="
if ! selftest; then echo "PC USBSETADDR TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBSETADDR SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBSETADDR TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBSETADDR TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbsetaddr.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci \
    -serial "tcp:127.0.0.1:4992,server,nowait" -qmp "tcp:127.0.0.1:4993,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4992 4993 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:usbaddr bsr" "plug:usb-kbd,k1" "type:lsusb" "type:usbaddr plain" "plug:usb-mouse,m1" "wait:2" \
    "type:usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'no USB keyboard|usb: new devices|set address|addressing|USB (keyboard|mouse)|plugged|keyboard: slot|answer|failed|timed out' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBSETADDR TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBSETADDR TEST FAILED"
exit 1
