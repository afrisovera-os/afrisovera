#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): hot-plug on EVERY USB 3 controller.
#
# AMD boards and recent Intel laptops have two or more xHCI controllers. The driver used to
# run only the controllers that had a device at power-on (suites pc-xhci2, pc-xhci-multi), so
# a keyboard or stick plugged later into a port of an empty controller was never seen. Now
# every controller runs from boot; an empty one says "nothing yet: for hot-plug".
# One power-on with two QEMU `qemu-xhci` controllers (00:06.0 = x1, 00:07.0 = x2) and NOTHING
# plugged in. Through QMP (scripts/lib/usb-steps.py):
#   a USB keyboard plugged into the SECOND controller, `lsusb` typed on it, unplugged;
#   a USB keyboard plugged into the FIRST controller, `lsusb` typed on it, unplugged;
#   `lsusb` once more (typed on the PS/2 keyboard).
# Graded: both controllers up and empty at boot, each plug enumerated and each unplug seen,
# lsusb listing the keyboard under the controller it was plugged into, the hot-plug counts,
# and the USB keyboard's own key count (the PS/2 keyboard cannot move it).
#   scripts/pc-xhci-hotplug-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-xhci-hotplug-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S n kc k
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    count() { printf '%s\n' "$S" | grep -acE "$1"; }
    has '^\[xhci\] 1 of 2: 00:06\.0 \(1b36:000d\), 0 device\(s\) connected <- used \(nothing yet: for hot-plug\)$' ||
        _f "00:06.0 was not named as running, empty, for hot-plug"
    has '^\[xhci\] 2 of 2: 00:07\.0 \(1b36:000d\), 0 device\(s\) connected <- used \(nothing yet: for hot-plug\)$' ||
        _f "00:07.0 was not named as running, empty, for hot-plug"
    for c in 00:06 00:07; do
        has "^\[xhci\] controller ${c}\.0 \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots" ||
            _f "the empty controller $c.0 was not started"
    done
    has '^\[xhci\] 2 controllers driven$' || _f "the boot did not say both controllers are driven"
    has '^\[xhci\] 0 device\(s\) connected, no USB keyboard$' || _f "the boot did not start with no USB keyboard"
    n=$(count '^(afrisovera> )?\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint')
    [ "$n" -eq 2 ] || _f "expected 2 keyboards enumerated after boot (one per controller), saw $n"
    n=$(count '\[xhci\] port [0-9]+: device plugged in and ready$')
    [ "$n" -eq 2 ] || _f "expected 2 plug-ins reported, saw $n"
    n=$(count '\[xhci\] port [0-9]+: device unplugged$')
    [ "$n" -eq 2 ] || _f "expected 2 unplugs seen, saw $n"
    has '^  hot-plug: 1 plugged in, 0 unplugged since boot$' || _f "lsusb on the keyboard in 00:07.0 did not run (or count the plug)"
    has '^  hot-plug: 2 plugged in, 1 unplugged since boot$' || _f "lsusb on the keyboard in 00:06.0 did not run (or count it)"
    has '^  hot-plug: 2 plugged in, 2 unplugged since boot$' || _f "lsusb after both unplugs does not count them"
    kc="$(printf '%s\n' "$S" | awk '/^  controller [0-9a-f:.]+:$/{c=$2} /^usb: /{c="none"} /<- keyboard$/{printf "%s ", c}')"
    [ "$kc" = "00:07.0: 00:06.0: " ] ||
        _f "lsusb did not list the keyboard under 00:07.0 then 00:06.0 (got '${kc:-nothing}')"
    k="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' | tail -1 | awk '{print $6}')"
    if [ -z "$k" ]; then
        _f "lsusb (typed on the USB keyboards) did not report a keyboard"
    elif [ "$k" -lt 12 ]; then
        _f "the USB keyboards counted only $k keys (two lsusb + Enter = 12; typed on PS/2?)"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] 1 of 2: 00:06.0 (1b36:000d), 0 device(s) connected <- used (nothing yet: for hot-plug)
[xhci] 2 of 2: 00:07.0 (1b36:000d), 0 device(s) connected <- used (nothing yet: for hot-plug)
[xhci] controller 00:06.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] controller 00:07.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] 2 controllers driven
[xhci] 0 device(s) connected, no USB keyboard
[xhci] no USB mouse
AfriSh ready. Type 'help' for commands.
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
lsusb
usb: xHCI controller 00:06.0, 8 ports, 0 connected
usb: xHCI controller 00:07.0, 8 ports, 1 connected
  hot-plug: 1 plugged in, 0 unplugged since boot
  controller 00:06.0:
  controller 00:07.0:
  port 5: connected, enabled, speed 3  <- keyboard
  keyboard: slot 1, 12 reports, 6 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> [xhci] port 5: device unplugged
[xhci] port 5: USB keyboard 0627:0001 (high speed), slot 1, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: device plugged in and ready
lsusb
usb: xHCI controller 00:06.0, 8 ports, 1 connected
usb: xHCI controller 00:07.0, 8 ports, 0 connected
  hot-plug: 2 plugged in, 1 unplugged since boot
  controller 00:06.0:
  port 5: connected, enabled, speed 3  <- keyboard
  controller 00:07.0:
  keyboard: slot 1, 24 reports, 12 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> [xhci] port 5: device unplugged
lsusb
usb: xHCI controller 00:06.0, 8 ports, 0 connected
usb: xHCI controller 00:07.0, 8 ports, 0 connected
  hot-plug: 2 plugged in, 2 unplugged since boot
  controller 00:06.0:
  controller 00:07.0:
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-xhci-hotplug-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    # the pc-xhci-multi driver: with nothing at power-on only the first controller ran, so the
    # keyboard in 00:07.0 was never seen (one plug, keys typed on PS/2 until the second)
    arm second-never-seen red "$(printf '%s\n' "$GREEN" | sed 's/^\(\[xhci\] 2 of 2: .*\) <- used (nothing yet: for hot-plug)$/\1: left off (nothing connected at power-on)/' |
        awk 'BEGIN{skip=0} /^afrisovera> \[xhci\] port 5: USB keyboard/ && !seen {seen=1; print "afrisovera> lsusb"; skip=1; next} skip && /^afrisovera> \[xhci\] port 5: device unplugged$/ {skip=0; print "afrisovera> "; next} !skip' |
        grep -v 'controller 00:07\.0 (xHCI\|controllers driven')"
    arm one-plug-only red "$(printf '%s\n' "$GREEN" | sed '0,/device plugged in and ready/{/device plugged in and ready/d}')"
    arm unplug-unseen red "$(printf '%s\n' "$GREEN" | sed '0,/device unplugged$/{s/^afrisovera> \[xhci\] port 5: device unplugged$/afrisovera> /}')"
    arm wrong-controller red "$(printf '%s\n' "$GREEN" | sed '0,/^  controller 00:06\.0:$/{s/^  controller 00:06\.0:$/X/}; 0,/^  controller 00:07\.0:$/{s/^  controller 00:07\.0:$/  controller 00:06.0:/}; s/^X$/  controller 00:07.0:/')"
    arm typed-on-ps2 red "$(printf '%s\n' "$GREEN" | sed 's/24 reports, 12 keys typed/24 reports, 6 keys typed/')"
    arm uncounted red "$(printf '%s\n' "$GREEN" | sed 's/2 plugged in, 2 unplugged/2 plugged in, 1 unplugged/')"
    arm device-at-boot red "$(printf '%s\n' "$GREEN" | grep -v 'no USB keyboard')"
    arm timed-out red "$(printf '%s\n[xhci] control request 0x06 timed out' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== PC USB hot-plug on every xHCI controller (PC track step 3) =="
if ! selftest; then echo "PC XHCI-HOTPLUG TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC XHCI-HOTPLUG SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC XHCI-HOTPLUG TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC XHCI-HOTPLUG TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-xhci-hotplug.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=x1,addr=0x6 -device qemu-xhci,id=x2,addr=0x7 \
    -serial "tcp:127.0.0.1:4990,server,nowait" -qmp "tcp:127.0.0.1:4991,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4990 4991 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "plug:usb-kbd,k1,x2" "type:lsusb" "unplug:k1" "plug:usb-kbd,k2,x1" "type:lsusb" "unplug:k2" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '\[xhci\]|^usb:|hot-plug|  controller|<- keyboard|  keyboard: slot' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC XHCI-HOTPLUG TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC XHCI-HOTPLUG TEST FAILED"
exit 1
