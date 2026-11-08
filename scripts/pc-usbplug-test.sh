#!/usr/bin/env bash
# PC track (5 Nov 2026): USB hot-plug -- a keyboard plugged in after boot, then pulled out.
#
# Boots the PC kernel with an xHCI and NOTHING on it. Through QMP, scripts/lib/usb-steps.py
# plugs in a USB keyboard (device_add), types `lsusb` on it, unplugs it (device_del) and
# types `lsusb` again (QEMU then sends the keys to the PS/2 keyboard). xhci.rs notes the
# port-status-change events in the timer interrupt and enumerates / detaches from the
# shell's idle loop (xhci::service). Graded: the keyboard is enumerated after boot, the
# command typed on it runs, and the unplug is seen and counted.
#   scripts/pc-usbplug-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbplug-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[xhci\] 0 device\(s\) connected, no USB keyboard$' || _f "the boot did not start with no USB keyboard"
    has '\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint' ||
        _f "the keyboard plugged in after boot was not enumerated"
    has '\[xhci\] port [0-9]+: device plugged in and ready$' || _f "the plug-in was not reported"
    has '^  hot-plug: 1 plugged in, 0 unplugged since boot$' || _f "lsusb typed on the new keyboard did not run (or did not count the plug)"
    has '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' || _f "lsusb did not list the hot-plugged keyboard"
    has '\[xhci\] port [0-9]+: device unplugged$' || _f "the unplug was not seen"
    has '^  hot-plug: 1 plugged in, 1 unplugged since boot$' || _f "lsusb after the unplug does not count it"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] 0 device(s) connected, no USB keyboard
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hot-plug: 1 plugged in, 0 unplugged since boot
  port 5: connected, enabled, speed 3  <- keyboard
  keyboard: slot 1, 12 reports, 6 keys typed
afrisovera> [xhci] port 5: device unplugged
usb: xHCI controller 00:03.0, 8 ports, 0 connected
  hot-plug: 1 plugged in, 1 unplugged since boot"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbplug-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm keyboard-at-boot red "$(printf '%s\n' "$GREEN" | grep -v 'no USB keyboard')"
    arm not-enumerated red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard 0627')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v '0 unplugged since boot')"
    arm unplug-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'device unplugged')"
    arm unplug-uncounted red "$(printf '%s\n' "$GREEN" | sed 's/1 plugged in, 1 unplugged/1 plugged in, 0 unplugged/')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 8 ]
}

echo "== PC USB hot-plug =="
if ! selftest; then echo "PC USBPLUG TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBPLUG SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBPLUG TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBPLUG TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbplug.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none -device qemu-xhci,id=xhci \
    -serial "tcp:127.0.0.1:4844,server,nowait" -qmp "tcp:127.0.0.1:4845,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4844 4845 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "plug:usb-kbd,k1" "type:lsusb" "unplug:k1" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'no USB keyboard|USB keyboard 0627|plugged|hot-plug|  keyboard: slot' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBPLUG TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBPLUG TEST FAILED"
exit 1
