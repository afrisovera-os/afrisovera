#!/usr/bin/env bash
# PC track step 3, USB (9 Nov 2026): a controller's own speed ID table (xHCI 7.2.1, PSIC).
#
# A port's speed ID (PORTSC bits 13:10) means full, low, high or SuperSpeed only when the
# controller keeps the default IDs. A controller may publish its own table instead (PSIC > 0 in
# its Supported Protocol capability, one PSI dword per ID: bit rate and, on USB 3, SuperSpeed or
# SuperSpeedPlus); newer Intel and AMD hosts do. xhci.rs counted those IDs but took the port's ID
# as a default one. Now it reads up to 8 PSI dwords per protocol, turns the port's ID into a
# speed through the table (USB 2 by bit rate, USB 3 by link protocol or a rate over 5 Gb/s, as
# Linux), names the table in the port kinds line (boot and lsusb), and `usbaddr` counts the
# devices whose speed came from a table.
#
# QEMU's qemu-xhci has no table of its own, so the test switch `usbaddr psi` puts one on
# controller 1 as Intel hosts publish it (USB 2: 1 full 12 Mb/s, 2 low 1500 Kb/s, 3 high
# 480 Mb/s; USB 3: 4 SuperSpeed 5 Gb/s, 5 SuperSpeedPlus 10 Gb/s) and checks each entry. Boots
# with an EMPTY xHCI; through QMP (scripts/lib/usb-steps.py): `usbaddr`, `usbaddr psi`, a USB
# keyboard plugged in and `lsusb` typed on it, a USB mouse plugged in, `usbaddr`. Graded: 5 of 5
# entries, the table in lsusb's kinds line, both devices taken at their own speed (QEMU: high) through the table
# (0 before, 2 after), nothing failed.
#   scripts/pc-usbpsi-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbpsi-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S k
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port kinds: .*USB 2\.0 on ports [0-9]+-[0-9]+' || _f "no port kinds line at boot"
    has '^\[xhci\] port kinds: .*speed IDs of its own' && _f "qemu-xhci said it has its own speed IDs before the test table"
    has '^  speed IDs: 0 device\(s\) by a controller.s own table$' || _f "the first usbaddr does not show 0 devices by a table"
    has '^usb: test: controller 1 now has its own speed ID table; 5 of 5 entries read as expected$' ||
        _f "usbaddr psi did not read all 5 test entries as expected"
    k="$(printf '%s\n' "$S" | grep -aE '^  port kinds:' | tail -1)"
    printf '%s\n' "$k" | grep -aqF '(3 speed IDs of its own: 1 full 2 low 3 high)' ||
        _f "the last kinds line does not show the USB 2 table: $k"
    printf '%s\n' "$k" | grep -aqF '(2 speed IDs of its own: 4 super 5 super plus)' ||
        _f "the last kinds line does not show the USB 3 table: $k"
    has '(^|> )\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|high) speed\)' || _f "the keyboard was not taken at its own (full or high) speed"
    has '(^|> )\[xhci\] port [0-9]+: USB mouse [0-9a-f]{4}:[0-9a-f]{4} \((full|high) speed\)' || _f "the mouse was not taken at its own (full or high) speed"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' || _f "lsusb typed on the USB keyboard did not run"
    has '^  speed IDs: 2 device\(s\) by a controller.s own table \(test table on controller 1\)$' ||
        _f "the last usbaddr does not count 2 devices by the test table"
    has '\(\? speed\)' && _f "a device with a speed the driver cannot name"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not answer)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not answer)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port kinds: USB 2.0 on ports 5-8, USB 3.0 on ports 1-4
[xhci] no USB keyboard
afrisovera> usbaddr
usb: new devices are tried first with plain; then the other way, then this way again
  speed IDs: 0 device(s) by a controller's own table
afrisovera> usbaddr psi
usb: test: controller 1 now has its own speed ID table; 5 of 5 entries read as expected
  port kinds: USB 2.0 on ports 5-8 (3 speed IDs of its own: 1 full 2 low 3 high), USB 3.0 on ports 1-4 (2 speed IDs of its own: 4 super 5 super plus)
[xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  port kinds: USB 2.0 on ports 5-8 (3 speed IDs of its own: 1 full 2 low 3 high), USB 3.0 on ports 1-4 (2 speed IDs of its own: 4 super 5 super plus)
  keyboard: slot 1, 12 reports, 6 keys typed
[xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
afrisovera> usbaddr
usb: new devices are tried first with plain; then the other way, then this way again
  speed IDs: 2 device(s) by a controller's own table (test table on controller 1)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbpsi-selftest.XXXXXX")" || return 1
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
    arm table-at-boot red "$(printf '%s\n' "$GREEN" | sed '2s/ports 1-4$/ports 1-4 (2 speed IDs of its own: 4 super 5 super plus)/')"
    arm no-first-count red "$(printf '%s\n' "$GREEN" | grep -v 'speed IDs: 0 device')"
    arm entry-wrong red "$(printf '%s\n' "$GREEN" | sed 's/5 of 5 entries/4 of 5 entries/')"
    arm usb2-table-wrong red "$(printf '%s\n' "$GREEN" | sed 's/1 full 2 low 3 high/1 low 2 full 3 high/')"
    arm usb3-table-wrong red "$(printf '%s\n' "$GREEN" | sed 's/4 super 5 super plus/4 super 5 super/')"
    arm kbd-low red "$(printf '%s\n' "$GREEN" | sed 's/USB keyboard 0627:0001 (high speed)/USB keyboard 0627:0001 (low speed)/')"
    arm mouse-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm count-1 red "$(printf '%s\n' "$GREEN" | sed 's/speed IDs: 2 device/speed IDs: 1 device/')"
    arm unknown-speed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 7: USB tablet 0627:0001 (? speed), slot 3')"
    arm not-answered red "$(printf '%s\n%s\n' "$GREEN" '[xhci] port 5: device did not answer (completion code 4 at Address Device); port reset, try 2 of 3')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC USB controller speed ID tables (PSIC) =="
if ! selftest; then echo "PC USBPSI TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBPSI SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBPSI TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBPSI TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbpsi.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci \
    -serial "tcp:127.0.0.1:5022,server,nowait" -qmp "tcp:127.0.0.1:5023,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5022 5023 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:usbaddr" "type:usbaddr psi" "plug:usb-kbd,k1" "type:lsusb" "plug:usb-mouse,m1" "wait:2" \
    "type:usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'port kinds|speed ID|USB (keyboard|mouse)|keyboard: slot|answer|failed|timed out' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBPSI TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBPSI TEST FAILED"
exit 1
