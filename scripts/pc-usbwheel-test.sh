#!/usr/bin/env bash
# PC track (9 Nov 2026): the USB mouse's wheel scrolls the console (F7 for USB mice).
#
# A boot report (HID B.2) has no wheel, so a USB mouse whose HID report descriptor has one is
# now kept in the report protocol and its reports are read by the descriptor's layout.
# Boots the PC kernel with a USB keyboard and a USB mouse (QEMU qemu-xhci + usb-kbd +
# usb-mouse, whose descriptor has buttons, X, Y and a wheel). scripts/lib/usb-steps.py moves
# the pointer from 320,240 by +100,+60 slowly, clicks the left button once, rolls the wheel
# up, up, up, down and types `lsusb`. Graded: the mouse is read in the report protocol with
# a wheel; the move still lands exactly at 420,300 and the click is counted (so X, Y and the
# buttons are read from the right bits); the wheel scrolls the console 3, 6, 9 then 6 lines
# back (COM1 notes, as the PS/2 wheel in pc-ps2mouse); lsusb counts 4 notches.
#   scripts/pc-usbwheel-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbwheel-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S ups
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[xhci\] port [0-9]+: USB mouse ' || _f "no USB mouse was enumerated"
    has '^\[xhci\] mouse: wheel, report protocol: report id [0-9]+, [1-3] buttons, x [0-9]+ bits at bit [0-9]+, y [0-9]+ bits at bit [0-9]+, wheel [0-9]+ bits at bit [0-9]+$' ||
        _f "the mouse was not kept in the report protocol with a wheel: $(printf '%s\n' "$S" | grep -aE '^\[xhci\] mouse:' | head -1)"
    has '^  mouse: slot [0-9]+, [0-9]+ reports, pointer at 420,300, buttons 000, 1 clicks$' ||
        _f "the move of +100,+60 and one click did not land exactly at 420,300 with 1 click: $(printf '%s\n' "$S" | grep -aE '^  mouse: slot' | tail -1)"
    ups="$(printf '%s\n' "$S" | grep -aoE '^\[con\] scrollback: [0-9]+ lines up' | awk '{print $3}' | tr '\n' ' ')"
    [ "$ups" = "3 6 9 6 " ] || _f "the wheel did not scroll the console 3, 6, 9 then 6 lines back: '${ups}'"
    has '^  mouse: wheel on \(report protocol\); 4 notch\(es\), 3 console lines each$' ||
        _f "lsusb does not count 4 wheel notches: $(printf '%s\n' "$S" | grep -aE '^  mouse: (no )?wheel' | tail -1)"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:'; then _f "the kernel panicked or a process faulted"; fi
    return "$fail"
}

GREEN="[xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
[xhci] mouse: wheel, report protocol: report id 0, 3 buttons, x 8 bits at bit 8, y 8 bits at bit 16, wheel 8 bits at bit 24
afrisovera>
[con] scrollback: 3 lines up of 160 kept; top line: [pc] SMP: 1 core online
[con] scrollback: 6 lines up of 160 kept; top line: [pc] memory map: 101 entries
[con] scrollback: 9 lines up of 160 kept; top line: [pc] GOP: 1280x800
[con] scrollback: 6 lines up of 160 kept; top line: [pc] memory map: 101 entries
lsusb
  mouse: slot 2, 9 reports, pointer at 420,300, buttons 000, 1 clicks
  mouse: 0 window drag(s)
  mouse: acceleration on, 0 report(s) sped up
  mouse: wheel on (report protocol); 4 notch(es), 3 console lines each"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbwheel-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-shape-2026-11-09 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm boot-protocol red "$(printf '%s\n' "$GREEN" | sed 's/^\[xhci\] mouse: wheel.*/[xhci] mouse: no wheel in its report descriptor, boot protocol/; s/^  mouse: wheel on.*/  mouse: no wheel (boot protocol)/' | grep -v '^\[con\]')"
    arm move-misread red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 420,300/pointer at 420,240/')"
    arm click-lost red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000, 1 clicks/buttons 000, 0 clicks/')"
    arm button-stuck red "$(printf '%s\n' "$GREEN" | sed 's/buttons 000, 1 clicks/buttons 001, 1 clicks/')"
    arm wheel-no-scroll red "$(printf '%s\n' "$GREEN" | grep -v '^\[con\]')"
    arm wheel-backwards red "$(printf '%s\n' "$GREEN" | sed 's/^\[con\] scrollback: 9 lines up/[con] scrollback: 3 lines up/')"
    arm wheel-uncounted red "$(printf '%s\n' "$GREEN" | sed 's/4 notch(es)/0 notch(es)/')"
    arm no-mouse red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm panic red "$(printf '%s\n%s\n' "$GREEN" 'KERNEL PANIC at xhci.rs')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC USB mouse wheel =="
if ! selftest; then echo "PC USBWHEEL TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBWHEEL SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBWHEEL TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBWHEEL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbwheel.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5744,server,nowait" -qmp "tcp:127.0.0.1:5745,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5744 5745 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "move:100,60" "down" "up" "wheel:up" "wheel:up" "wheel:up" "wheel:down" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'USB mouse|^\[xhci\] mouse|^\[con\]|lsusb|  mouse:' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBWHEEL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBWHEEL TEST FAILED"
exit 1
