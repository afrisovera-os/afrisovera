#!/usr/bin/env bash
# PC track step 3, USB (9 Nov 2026): which root ports are USB 2 and which USB 3, and the
# SuperSpeedPlus speed IDs.
#
# An xHCI controller names its root ports in Supported Protocol capabilities (xHCI 7.2): a
# range of ports per USB revision. xhci.rs never read them, and it took the port speed only as
# 1 to 4: on the USB-C ports of newer PCs a USB 3.1/3.2 device reports speed ID 5 to 7
# (SuperSpeedPlus, 10 or 20 Gb/s, xHCI 7.2.2.1.1 defaults), which the driver treated as full
# speed, so its control endpoint got an 8-byte packet size instead of 512 and addressing it
# would fail. Now every speed ID from 4 up is SuperSpeed (512 bytes, the SuperSpeed hub path),
# 5 to 7 are named "super plus", and a boot line (also in `lsusb`) names the port ranges, so a
# picture of a real PC's screen says which ports its built-in devices can be on (field item F2).
#
# QEMU has no SuperSpeedPlus device, so the speed IDs 5 to 7 cannot be shown here; this suite
# proves the port kinds line (USB 2.0 and USB 3.0 ranges covering all 8 ports of qemu-xhci, no
# overlap), that it is repeated in `lsusb`, and that a USB keyboard plugged in at power-on still
# works (taken at boot, typed on).
#   scripts/pc-usbports-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbports-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# check_kinds <line> <ports>: both ranges present, inside 1..ports, no overlap, every port named
check_kinds() {
    local l="$1" n="$2" a b c d
    printf '%s\n' "$l" | grep -aqE 'port kinds: USB 2\.0 on ports [0-9]+-[0-9]+, USB 3\.0 on ports [0-9]+-[0-9]+$|port kinds: USB 3\.0 on ports [0-9]+-[0-9]+, USB 2\.0 on ports [0-9]+-[0-9]+$' || return 1
    read -r a b < <(printf '%s\n' "$l" | sed -E 's/.*USB 2\.0 on ports ([0-9]+)-([0-9]+).*/\1 \2/')
    read -r c d < <(printf '%s\n' "$l" | sed -E 's/.*USB 3\.0 on ports ([0-9]+)-([0-9]+).*/\1 \2/')
    [ "$a" -ge 1 ] && [ "$c" -ge 1 ] && [ "$b" -le "$n" ] && [ "$d" -le "$n" ] && [ "$a" -le "$b" ] && [ "$c" -le "$d" ] || return 1
    { [ "$b" -lt "$c" ] || [ "$d" -lt "$a" ]; } || return 1
    [ $((b - a + 1 + d - c + 1)) -eq "$n" ]
}

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    local n line
    n="$(printf '%s\n' "$S" | grep -aE '^\[xhci\] controller [0-9a-f:.]+ \(xHCI [0-9.]+\), [0-9]+ ports' | head -1 | sed -E 's/.*\), ([0-9]+) ports.*/\1/')"
    [ -n "$n" ] || { _f "the controller did not start"; n=0; }
    line="$(printf '%s\n' "$S" | grep -aE '^\[xhci\] port kinds:' | head -1)"
    if [ -z "$line" ]; then
        _f "no port kinds line at boot"
    elif ! check_kinds "$line" "$n"; then
        _f "the boot's port kinds do not cover the $n ports as one USB 2.0 and one USB 3.0 range: $line"
    fi
    line="$(printf '%s\n' "$S" | grep -aE '^  port kinds:' | head -1)"
    if [ -z "$line" ]; then
        _f "lsusb does not show the port kinds"
    elif ! check_kinds "$line" "$n"; then
        _f "lsusb's port kinds do not cover the $n ports: $line"
    fi
    has 'port kinds: not named|not named$' && _f "ports left unnamed"
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high|super) speed\)' || _f "the keyboard was not taken at boot"
    has '\(\? speed\)' && _f "a device with a speed the driver cannot name"
    has 'no USB keyboard' && _f "the boot said no USB keyboard"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' || _f "lsusb typed on the USB keyboard did not run"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not answer)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not answer)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port kinds: USB 2.0 on ports 1-4, USB 3.0 on ports 5-8
[xhci] ports steady after 150 ms: 1 of 8 with a device
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] no USB mouse
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  port kinds: USB 2.0 on ports 1-4, USB 3.0 on ports 5-8
  keyboard: slot 1, 12 reports, 6 keys typed"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbports-selftest.XXXXXX")" || return 1
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
    arm usb3-first green "$(printf '%s\n' "$GREEN" | sed 's/USB 2.0 on ports 1-4, USB 3.0 on ports 5-8/USB 3.0 on ports 1-4, USB 2.0 on ports 5-8/')"
    arm no-kinds-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] port kinds')"
    arm lsusb-no-kinds red "$(printf '%s\n' "$GREEN" | grep -v '^  port kinds')"
    arm overlap red "$(printf '%s\n' "$GREEN" | sed 's/USB 3.0 on ports 5-8/USB 3.0 on ports 4-7/')"
    arm short red "$(printf '%s\n' "$GREEN" | sed 's/USB 3.0 on ports 5-8/USB 3.0 on ports 5-7, 1 of 8 not named/')"
    arm unnamed red "$(printf '%s\n' "$GREEN" | sed 's/port kinds: .*/port kinds: not named by the controller/')"
    arm unknown-speed red "$(printf '%s\n' "$GREEN" | sed 's/(full speed)/(? speed)/')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm not-answered red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 5: device did not answer (completion code 4 at Address Device); port reset, try 2 of 3')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC USB port kinds and SuperSpeedPlus speed IDs =="
if ! selftest; then echo "PC USBPORTS TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBPORTS SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBPORTS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBPORTS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbports.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5020,server,nowait" -qmp "tcp:127.0.0.1:5021,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5020 5021 "$W/serial.log" "lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> lsusb|^usb:|^  (port kinds|keyboard)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBPORTS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBPORTS TEST FAILED"
exit 1
