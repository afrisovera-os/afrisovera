#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the second way of giving a USB device its address.
#
# On the HP ENVY's Intel xHCI, Address Device and GET_DESCRIPTOR ended with completion code 4
# (a USB transaction error). xhci.rs now knows both ways Linux uses: plain (Address Device
# sends SET_ADDRESS at once) and descriptor first (Address Device with BSR, the first 8 bytes
# of the device descriptor read at the default address, then a second Address Device), and a
# root-port device that does not answer is tried three times, the second try the other way.
# QEMU's devices always answer, so the retry itself cannot be shown here; this suite proves
# the descriptor-first path works end to end, which the retry depends on.
#
# Boots the PC kernel with an EMPTY xHCI. Through QMP (scripts/lib/usb-steps.py): `usbaddr bsr`
# (typed on the PS/2 keyboard), a USB keyboard plugged in and `lsusb` typed ON IT (so its
# reports run after the BSR addressing), a USB mouse plugged in (BSR too), `usbaddr plain`, a
# USB tablet plugged in (plain), and `usbaddr`. Graded: the mode lines, all three enumerated,
# lsusb on the new keyboard counting 1 descriptor-first device, the final count 1 plain and 2
# descriptor first with no retries, and no failed command, time-out or "did not answer".
#   scripts/pc-usbaddr-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbaddr-test.sh --selftest   # grader selftest only
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
    has 'usb: new devices are tried first with descriptor read before the address \(BSR\); then the other way, then this way again$' ||
        _f "usbaddr bsr did not switch to descriptor first"
    has '\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint' ||
        _f "the keyboard plugged in after usbaddr bsr was not enumerated"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' ||
        _f "lsusb typed on the BSR-addressed keyboard did not run"
    has '^  addressing: 0 plain, 1 descriptor first \(BSR\), 0 answered on a later try, 0 on no try$' ||
        _f "lsusb on the keyboard does not count it as addressed descriptor first"
    has '\[xhci\] port [0-9]+: USB mouse [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the mouse plugged in (BSR) was not enumerated"
    has 'usb: new devices are tried first with address first \(plain\); then the other way, then this way again$' ||
        _f "usbaddr plain did not switch back"
    has '\[xhci\] port [0-9]+: USB tablet [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the tablet plugged in (plain) was not enumerated"
    has '^  addressing: 1 plain, 2 descriptor first \(BSR\), 0 answered on a later try, 0 on no try$' ||
        _f "the final usbaddr does not count 1 plain and 2 descriptor first"
    has 'did not answer|answered on try|last addressing error' && _f "a QEMU device needed a retry"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] 0 device(s) connected, no USB keyboard
afrisovera> usb: new devices are tried first with descriptor read before the address (BSR); then the other way, then this way again
  addressing: 0 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hot-plug: 1 plugged in, 0 unplugged since boot
  addressing: 0 plain, 1 descriptor first (BSR), 0 answered on a later try, 0 on no try
  port 5: connected, enabled, speed 3  <- keyboard
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> [xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] port 6: device plugged in and ready
usb: new devices are tried first with address first (plain); then the other way, then this way again
  addressing: 0 plain, 2 descriptor first (BSR), 0 answered on a later try, 0 on no try
afrisovera> [xhci] port 7: USB tablet 0627:0001 (high speed), slot 3, interrupt-IN endpoint 1 every 1000 us
[xhci] port 7: device plugged in and ready
usb: new devices are tried first with address first (plain); then the other way, then this way again
  addressing: 1 plain, 2 descriptor first (BSR), 0 answered on a later try, 0 on no try"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbaddr-selftest.XXXXXX")" || return 1
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
    arm bsr-not-set red "$(printf '%s\n' "$GREEN" | grep -v 'first with descriptor read')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm counted-plain red "$(printf '%s\n' "$GREEN" | sed 's/addressing: 0 plain, 1 descriptor/addressing: 1 plain, 0 descriptor/')"
    arm mouse-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm plain-not-set red "$(printf '%s\n' "$GREEN" | grep -v 'first with address first')"
    arm tablet-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB tablet')"
    arm final-count red "$(printf '%s\n' "$GREEN" | sed 's/addressing: 1 plain, 2 descriptor/addressing: 0 plain, 3 descriptor/')"
    arm retried red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 6: answered on try 2 (address first (plain))')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC USB addressing (F2) =="
if ! selftest; then echo "PC USBADDR TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBADDR SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBADDR TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBADDR TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbaddr.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci \
    -serial "tcp:127.0.0.1:4992,server,nowait" -qmp "tcp:127.0.0.1:4993,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4992 4993 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:usbaddr bsr" "plug:usb-kbd,k1" "type:lsusb" "plug:usb-mouse,m1" "wait:2" "type:usbaddr plain" \
    "plug:usb-tablet,t1" "wait:2" "type:usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'no USB keyboard|usb: new devices|addressing|USB (keyboard|mouse|tablet)|plugged|keyboard: slot|answer|failed|timed out' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBADDR TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBADDR TEST FAILED"
exit 1
