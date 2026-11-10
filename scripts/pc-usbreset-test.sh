#!/usr/bin/env bash
# PC track, field item F2 (9 Nov 2026): the controller reset, as Linux does it on Intel hosts.
#
# Linux's xhci_reset waits 1 ms after setting HCRST before it reads any controller register
# again (quirk XHCI_INTEL_HOST: on Intel xHCI controllers a read right after the reset can,
# rarely, hang the whole PC), halts the controller first (up to 16 ms for HCHalted) and allows
# 10 s for HCRST and Controller Not Ready to clear. xhci.rs polled the registers at once; its
# new reset() halts (up to 20 ms), resets, waits 1 ms on every host, then waits for ready, and
# says so in a boot line and in `usbaddr`.
#
# QEMU has no Intel xHCI, so the hang itself cannot be shown; this suite proves the reset path
# runs (halted first, ready after at least the 1 ms wait), the controller then works (a USB
# keyboard plugged in at power-on is taken and typed on) and `usbaddr` shows the reset line.
#   scripts/pc-usbreset-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbreset-test.sh --selftest   # grader selftest only
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
    line="$(printf '%s\n' "$S" | grep -aE '^\[xhci\] reset: .*ready [0-9]+ ms after HCRST \(first read after 1 ms, as Linux on Intel hosts(; this is one)?\)$' | head -1)"
    if [ -z "$line" ]; then
        _f "no reset line at boot"
    else
        printf '%s\n' "$line" | grep -aq '^\[xhci\] reset: halted, ready' || _f "the controller did not halt before its reset: $line"
        printf '%s\n' "$line" | grep -aq 'this is one' && _f "QEMU's controller called an Intel host: $line"
        ms="$(printf '%s\n' "$line" | sed -E 's/.* ready ([0-9]+) ms after.*/\1/')"
        { [ "$ms" -ge 1 ] && [ "$ms" -le 100 ]; } || _f "ready after $ms ms (1 to 100 expected: the 1 ms wait comes first)"
    fi
    has '^\[xhci\] controller [0-9a-f:.]+ \(xHCI [0-9.]+\), [0-9]+ ports' || _f "the controller did not start after its reset"
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} ' || _f "the keyboard was not taken at boot"
    has 'no USB keyboard' && _f "the boot said no USB keyboard"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [1-9][0-9]* keys typed$' || _f "lsusb typed on the USB keyboard did not run"
    has "^  reset: controller 1 halted, ready [0-9]+ ms after HCRST, first read after 1 ms$" ||
        _f "usbaddr does not show the reset"
    has 'did not come out of reset|did not halt' && _f "the reset went wrong"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] reset: halted, ready 1 ms after HCRST (first read after 1 ms, as Linux on Intel hosts)
[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] ports steady after 150 ms: 1 of 8 with a device
[xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] no USB mouse
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> usbaddr
usb: new devices are tried first with address first (plain); then the other way, then this way again
  reset: controller 1 halted, ready 1 ms after HCRST, first read after 1 ms
  power-on: controller 1's ports steady after 150 ms, 1 with a device"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbreset-selftest.XXXXXX")" || return 1
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
    arm slower-ready green "$(printf '%s\n' "$GREEN" | sed 's/ready 1 ms/ready 7 ms/g')"
    arm no-reset-line red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] reset:')"
    arm not-halted red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] reset: halted, /[xhci] reset: did not halt in 20 ms, reset anyway; /')"
    arm no-wait red "$(printf '%s\n' "$GREEN" | sed 's/^\(\[xhci\] reset: halted, ready\) 1 ms/\1 0 ms/')"
    arm called-intel red "$(printf '%s\n' "$GREEN" | sed 's/Intel hosts)$/Intel hosts; this is one)/')"
    arm not-started red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] controller 00')"
    arm keyboard-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v 'keys typed')"
    arm usbaddr-no-reset red "$(printf '%s\n' "$GREEN" | grep -v '^  reset: controller')"
    arm reset-timeout red "$(printf '%s\n%s\n' "$GREEN" '[xhci] controller did not come out of reset in 10 s')"
    arm cmd-failed red "$(printf '%s\n%s\n' "$GREEN" '[xhci] command type 11 failed (completion code 4)')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== PC xHCI reset (F2) =="
if ! selftest; then echo "PC USBRESET TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBRESET SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBRESET TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBRESET TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbreset.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5020,server,nowait" -qmp "tcp:127.0.0.1:5021,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5020 5021 "$W/serial.log" "lsusb" "usbaddr"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> (lsusb|usbaddr)|^usb:|^  (reset|power-on|keyboard)' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBRESET TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBRESET TEST FAILED"
exit 1
