#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the ports' power-on wait.
#
# On the HP ENVY the boot said "0 devices connected", and the webcam, Bluetooth and fingerprint
# reader then turned up as hot-plugs a moment later (and failed to be addressed): the driver
# looked at the ports the instant the controller was started, before a real xHCI had seen its
# devices again. xhci.rs's settle() now powers every port and watches them until the set with a
# device has not changed for 100 ms (at least 150 ms, at most 1 s), as Linux's hub driver waits
# for power-good and a 100 ms steady connection before it uses a port.
#
# QEMU shows its devices at once, so the late arrival itself cannot be shown here; this suite
# proves the wait runs, ends steady, counts the devices plugged in at power-on, and that those
# devices are then taken at boot (not as hot-plugs) and work. Boots the PC kernel with a USB
# keyboard and a USB mouse already plugged in; `lsusb` and `usbaddr` are typed on the keyboard.
#   scripts/pc-usbsettle-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbsettle-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    local line ms
    line="$(printf '%s\n' "$S" | grep -aE '^\[xhci\] ports (steady|still changing) after [0-9]+ ms: [0-9]+ of [0-9]+ with a device$' | head -1)"
    if [ -z "$line" ]; then
        _f "no power-on wait line at boot"
    else
        printf '%s\n' "$line" | grep -aq '^\[xhci\] ports steady after' || _f "the ports did not end steady: $line"
        printf '%s\n' "$line" | grep -aqE ': 2 of [0-9]+ with a device$' || _f "the wait did not count the 2 devices: $line"
        ms="$(printf '%s\n' "$line" | sed -E 's/.* after ([0-9]+) ms.*/\1/')"
        { [ "$ms" -ge 150 ] && [ "$ms" -le 1000 ]; } || _f "the wait took $ms ms (150 to 1000 expected)"
    fi
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the keyboard was not taken at boot"
    has '^\[xhci\] port [0-9]+: USB mouse [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the mouse was not taken at boot"
    has '^\[xhci\] port [0-9]+: device plugged in' && _f "a device present at power-on was seen as a hot-plug"
    has 'no USB keyboard' && _f "the boot said no USB keyboard"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' || _f "lsusb typed on the USB keyboard did not run"
    has "^  power-on: controller 1's ports steady after [0-9]+ ms, 2 with a device$" ||
        _f "usbaddr does not show the power-on wait"
    has '^  addressing: 2 plain, 0 descriptor first \(BSR\), 0 answered on a later try, 0 on no try$' ||
        _f "usbaddr does not count the 2 devices addressed plain"
    has 'did not answer|answered on try' && _f "a QEMU device needed a retry"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] ports steady after 150 ms: 2 of 8 with a device
[xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 2 connected
  addressing: 2 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> usbaddr
usb: new devices are tried first with address first (plain); then the other way, then this way again
  power-on: controller 1's ports steady after 150 ms, 2 with a device
  addressing: 2 plain, 0 descriptor first (BSR), 0 answered on a later try, 0 on no try"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbsettle-selftest.XXXXXX")" || return 1
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
    arm longer-wait green "$(printf '%s\n' "$GREEN" | sed 's/after 150 ms/after 420 ms/g')"
    arm no-wait-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] ports steady')"
    arm still-changing red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] ports steady/[xhci] ports still changing/')"
    arm wrong-count red "$(printf '%s\n' "$GREEN" | sed 's/: 2 of 8 with/: 0 of 8 with/')"
    arm too-short red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] ports steady after 150/[xhci] ports steady after 40/')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm mouse-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse 0627')"
    arm hot-plugged red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 5: device plugged in and ready')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm usbaddr-no-wait red "$(printf '%s\n' "$GREEN" | grep -v 'power-on: controller')"
    arm retried red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 6: answered on try 2 (address first (plain))')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC USB power-on wait (F2) =="
if ! selftest; then echo "PC USBSETTLE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBSETTLE SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBSETTLE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBSETTLE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbsettle.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5020,server,nowait" -qmp "tcp:127.0.0.1:5021,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5020 5021 "$W/serial.log" "lsusb" "usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> (lsusb|usbaddr)|^usb:|^  (power-on|addressing|keyboard)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBSETTLE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBSETTLE TEST FAILED"
exit 1
