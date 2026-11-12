#!/usr/bin/env bash
# PC track step 3 (8 Nov 2026): hot-plug BEHIND a USB hub.
#
# Hubs were read once at power-on: a keyboard, mouse or stick plugged into a hub (a desk hub,
# a monitor's USB ports, a laptop dock) afterwards was never seen, and one pulled out stayed
# "there". Now each hub's status-change endpoint (interrupt IN, one bit per port) runs, and a
# port it reports is served like a root port: a new device reset and enumerated (with its
# route string and transaction translator), a gone one detached with everything behind it.
# One power-on, one QEMU `qemu-xhci` with a `usb-hub` on port 1 and NOTHING behind it. Through
# QMP (scripts/lib/usb-steps.py):
#   a USB keyboard plugged into the hub's port 2, `lsusb` typed on it, unplugged;
#   a second hub plugged into the hub's port 4, a USB keyboard into ITS port 3 (two tiers),
#   `lsusb` typed on that keyboard, the keyboard unplugged, then the second hub;
#   `lsusb` once more (typed on the PS/2 keyboard).
# Graded: both hubs watched for hot-plug, every plug enumerated and every unplug seen on the
# right hub port, lsusb's hub and hot-plug counts at each step, and the USB keyboards' own
# key count (the PS/2 keyboard cannot move it).
#   scripts/pc-usbhubplug-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbhubplug-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S n k seq
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    count() { printf '%s\n' "$S" | grep -acE "$1"; }
    n=$(count '^(afrisovera> )?\[xhci\] port [0-9]+: USB hub [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, 8 ports$')
    [ "$n" -eq 2 ] || _f "expected 2 hubs set up (one at power-on, one plugged in), saw $n"
    n=$(count '^(afrisovera> )?\[xhci\] hub slot [0-9]+: watching its 8 ports for hot-plug \(every [0-9]+ us\)$')
    [ "$n" -eq 2 ] || _f "expected 2 hubs watched for hot-plug, saw $n"
    n=$(count '^(afrisovera> )?\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, interrupt-IN endpoint')
    [ "$n" -eq 2 ] || _f "expected 2 keyboards enumerated behind the hubs, saw $n"
    for p in 2 4 3; do
        has "\[xhci\] hub port $p: device plugged in and ready$" || _f "the device plugged into hub port $p was not enumerated"
        has "\[xhci\] hub port $p: device unplugged$" || _f "the device pulled out of hub port $p was not seen going"
    done
    seq="$(printf '%s\n' "$S" | awk '/^  hubs: [0-9]+ with [0-9]+ ports between them$/{printf "%s/%s ", $2, $4}')"
    [ "$seq" = "1/8 2/16 1/8 " ] || _f "lsusb's hub counts were '${seq:-nothing}', expected '1/8 2/16 1/8 '"
    seq="$(printf '%s\n' "$S" | awk '/^  hubs: [0-9]+ watched for hot-plug behind them$/{printf "%s ", $2}')"
    [ "$seq" = "1 2 1 " ] || _f "lsusb's watched-hub counts were '${seq:-nothing}', expected '1 2 1 '"
    seq="$(printf '%s\n' "$S" | awk '/^  hot-plug: [0-9]+ plugged in, [0-9]+ unplugged since boot$/{printf "%s/%s ", $2, $5}')"
    [ "$seq" = "1/0 3/1 3/3 " ] || _f "lsusb's hot-plug counts were '${seq:-nothing}', expected '1/0 3/1 3/3 '"
    k="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' | tail -1 | awk '{print $6}')"
    if [ -z "$k" ]; then
        _f "lsusb (typed on the USB keyboards) did not report a keyboard"
    elif [ "$k" -lt 12 ]; then
        _f "the USB keyboards counted only $k keys (two lsusb + Enter = 12; typed on PS/2?)"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|\[xhci\] .*(failed|timed out|did not|no room)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|\[xhci\] .*(failed|timed out|did not|no room)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 5: USB hub 0409:55aa (full speed), slot 1, 8 ports
[xhci] hub slot 1: watching its 8 ports for hot-plug (every 128000 us)
[xhci] 1 device(s) connected, no USB keyboard
[xhci] no USB mouse
AfriSh ready. Type 'help' for commands.
afrisovera> [xhci] port 5: USB keyboard 0627:0001 (full speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] hub port 2: device plugged in and ready
lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hubs: 1 with 8 ports between them
  hubs: 1 watched for hot-plug behind them
  hot-plug: 1 plugged in, 0 unplugged since boot
  port 5: connected, enabled, speed 1  <- keyboard
  keyboard: slot 2, 12 reports, 6 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> [xhci] hub port 2: device unplugged
[xhci] port 5: USB hub 0409:55aa (full speed), slot 2, 8 ports
[xhci] hub slot 2: watching its 8 ports for hot-plug (every 128000 us)
[xhci] hub port 4: device plugged in and ready
[xhci] port 5: USB keyboard 0627:0001 (full speed), slot 3, interrupt-IN endpoint 1 every 8000 us
[xhci] hub port 3: device plugged in and ready
lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hubs: 2 with 16 ports between them
  hubs: 2 watched for hot-plug behind them
  hot-plug: 3 plugged in, 1 unplugged since boot
  port 5: connected, enabled, speed 1  <- keyboard
  keyboard: slot 3, 24 reports, 12 keys typed
  keyboard: 0 repeats while a key was held
afrisovera> [xhci] hub port 3: device unplugged
[xhci] hub port 4: device unplugged
lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  hubs: 1 with 8 ports between them
  hubs: 1 watched for hot-plug behind them
  hot-plug: 3 plugged in, 3 unplugged since boot
  port 5: connected, enabled, speed 1
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbhubplug-selftest.XXXXXX")" || return 1
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
    # the old driver: hubs read once at power-on, nothing behind them ever seen afterwards
    # (every lsusb typed on PS/2, the hub counts never moving)
    arm hubs-read-once red "$(printf '%s\n' "$GREEN" | grep -v 'watching its\|watched for\|hub port\|USB keyboard\|keyboard:\|hot-plug:\|slot 2, 8 ports' |
        sed 's/hubs: 2 with 16/hubs: 1 with 8/')"
    arm nested-unseen red "$(printf '%s\n' "$GREEN" | grep -v 'hub port 3: device plugged in')"
    arm hub-stays red "$(printf '%s\n' "$GREEN" | grep -v 'hub port 4: device unplugged' |
        awk '/^  hubs: 1 with 8/{n++; if(n==2){print "  hubs: 2 with 16 ports between them"; next}} {print}')"
    arm second-hub-unwatched red "$(printf '%s\n' "$GREEN" | sed '0,/hub slot 2: watching/{/hub slot 2: watching/d}')"
    arm typed-on-ps2 red "$(printf '%s\n' "$GREEN" | sed 's/24 reports, 12 keys typed/24 reports, 6 keys typed/')"
    arm uncounted red "$(printf '%s\n' "$GREEN" | sed 's/3 plugged in, 3 unplugged/3 plugged in, 2 unplugged/')"
    arm timed-out red "$(printf '%s\n[xhci] control request 0x00 timed out' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC USB hot-plug behind hubs (PC track step 3) =="
if ! selftest; then echo "PC USBHUBPLUG TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBHUBPLUG SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBHUBPLUG TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBHUBPLUG TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbhubplug.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-hub,bus=xhci.0,port=1 \
    -serial "tcp:127.0.0.1:5010,server,nowait" -qmp "tcp:127.0.0.1:5011,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5010 5011 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "plug:usb-kbd,k1,xhci,1.2" "type:lsusb" "unplug:k1" \
    "plug:usb-hub,h2,xhci,1.4" "plug:usb-kbd,k2,xhci,1.4.3" "type:lsusb" "unplug:k2" "unplug:h2" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '\[xhci\]|^usb:|hot-plug|  hubs:|<- keyboard|  keyboard: slot' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBHUBPLUG TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBHUBPLUG TEST FAILED"
exit 1
