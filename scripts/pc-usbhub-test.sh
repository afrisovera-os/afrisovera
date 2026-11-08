#!/usr/bin/env bash
# PC track (5 Nov 2026): a USB hub -- the keyboard and the mouse plugged into it, not into the PC.
#
# Boots the PC kernel with a USB hub on the xHCI (QEMU usb-hub) and a USB keyboard and a USB
# mouse on the HUB's ports 1 and 3. xhci.rs must find the hub, power and reset its ports and
# enumerate both devices through it (route strings). Then, as in pc-wmdrag, everything is
# done through the hub: scripts/lib/usb-steps.py types `wm` on the USB keyboard (three ring-3
# programs draw windows at 30,44 / 330,44 / 30,244), moves the pointer from 320,240 onto
# window 0's title bar, presses the left button, moves +100,+80, releases, types `wmlist`,
# holds `x` for 1.5 s, and types `lsusb`. Graded: the hub and both devices set up, window 0
# now at 130,124 and raised to the front, the other two unmoved, one drag counted, and the
# held key repeated (about 30 times: USB keyboards leave repeat to the host).
#   scripts/pc-usbhub-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbhub-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[xhci\] port [0-9]+: USB hub [0-9a-f]{4}:[0-9a-f]{4} \([a-z]+ speed\), slot [0-9]+, [0-9]+ ports$' || _f "the USB hub was not set up"
    has '^\[xhci\] port [0-9]+: USB keyboard ' || _f "no USB keyboard was enumerated behind the hub"
    has '^\[xhci\] port [0-9]+: USB mouse ' || _f "no USB mouse was enumerated behind the hub"
    has '^  hubs: 1 with [0-9]+ ports between them$' || _f "lsusb does not report the hub"
    local rp
    rp="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: [0-9]+ repeats while a key was held$' | tail -1 | awk '{print $2}')"
    [ -n "$rp" ] && [ "$rp" -ge 15 ] || _f "holding a key 1.5 s gave ${rp:-no} repeats (expected about 30)"
    has '^    \[wm\] 3 ring-3 window program\(s\) drew and exited \(3 clean\)$' || _f "wm (typed on the USB keyboard) did not draw three windows"
    has '^wm: window 0 at 130,124 size 200x100$' || _f "window 0 was not dragged from 30,44 to 130,124"
    has '^wm: window 1 at 330,44 size 200x100$' || _f "window 1 moved, but only window 0 was dragged"
    has '^wm: window 2 at 30,244 size 200x100$' || _f "window 2 moved, but only window 0 was dragged"
    has '^wm: 3 window\(s\), on screen, front to back \[0, ' || _f "the dragged window 0 was not raised to the front"
    has '^  mouse: slot [0-9]+, [0-9]+ reports, pointer at 150,134, buttons 000, 1 clicks$' ||
        _f "lsusb does not show the pointer at 150,134, released, one click"
    has '^  mouse: 1 window drag\(s\)$' || _f "lsusb does not count exactly one window drag"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:'; then _f "the kernel panicked or a process faulted"; fi
    return "$fail"
}

GREEN="[xhci] port 5: USB hub 0409:55aa (full speed), slot 1, 8 ports
[xhci] port 5: USB keyboard 0627:0001 (full speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] port 5: USB mouse 0627:0001 (full speed), slot 3, interrupt-IN endpoint 1 every 8000 us
afrisovera> wm
    [wm] 3 ring-3 window program(s) drew and exited (3 clean)
afrisovera> wmlist
wm: window 0 at 130,124 size 200x100
wm: window 1 at 330,44 size 200x100
wm: window 2 at 30,244 size 200x100
wm: 3 window(s), on screen, front to back [0, 3, 2, 1]
afrisovera> lsusb
  hubs: 1 with 8 ports between them
  keyboard: 29 repeats while a key was held
  mouse: slot 2, 12 reports, pointer at 150,134, buttons 000, 1 clicks
  mouse: 1 window drag(s)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbhub-selftest.XXXXXX")" || return 1
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
    arm not-dragged red "$(printf '%s\n' "$GREEN" | sed 's/window 0 at 130,124/window 0 at 30,44/')"
    arm wrong-window red "$(printf '%s\n' "$GREEN" | sed 's/window 1 at 330,44/window 1 at 430,124/')"
    arm not-raised red "$(printf '%s\n' "$GREEN" | sed 's/front to back \[0, 3, 2, 1\]/front to back [3, 2, 1, 0]/')"
    arm no-drag-count red "$(printf '%s\n' "$GREEN" | sed 's/1 window drag(s)/0 window drag(s)/')"
    arm button-stuck red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000/buttons 001/')"
    arm no-hub red "$(printf '%s\n' "$GREEN" | grep -v 'USB hub')"
    arm no-hub-lsusb red "$(printf '%s\n' "$GREEN" | grep -v '  hubs:')"
    arm no-repeat red "$(printf '%s\n' "$GREEN" | sed 's/29 repeats/0 repeats/')"
    arm no-mouse red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC USB hub (keyboard + mouse behind it) =="
if ! selftest; then echo "PC USBHUB TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBHUB SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBHUB TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBHUB TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbhub.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-hub,bus=xhci.0,port=1 \
    -device usb-kbd,bus=xhci.0,port=1.1 -device usb-mouse,bus=xhci.0,port=1.3 \
    -serial "tcp:127.0.0.1:4864,server,nowait" -qmp "tcp:127.0.0.1:4865,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4864 4865 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:wm" "move:-270,-186" down "move:100,80" up "type:wmlist" "hold:x,1500" "type:" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'USB (hub|keyboard|mouse)|\[wm\] 3|^wm:|  mouse:|  hubs:' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBHUB TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBHUB TEST FAILED"
exit 1
