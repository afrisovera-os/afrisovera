#!/usr/bin/env bash
# PC track (9 Nov 2026): the USB mouse gets the touchpad's pointer acceleration.
#
# Boots the PC kernel with a USB keyboard and a USB mouse (QEMU qemu-xhci + usb-kbd +
# usb-mouse). scripts/lib/usb-steps.py moves the pointer from 320,240 by -300,-200 slowly
# (40 counts a tenth of a second), then flicks it +300,0 in one event (QEMU hands that out
# as back-to-back reports of up to 127 counts), types `accel off`, flicks -300,0 the same
# way, and types `accel on`, with `lsusb` after each move. Graded: the slow move lands
# exactly at 20,40 with no report sped up; the flick goes further than its 300 counts
# (x at least 340, y still 40) with at least one report sped up; with acceleration off the
# same flick is exactly 1:1 (x down by 300) and nothing more is sped up; `accel` names the
# USB mouse's state each time.
#   scripts/pc-usbaccel-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbaccel-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S P A px1 py1 px2 py2 px3 py3 a1 n1 a2 n2 a3 n3
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[xhci\] port [0-9]+: USB mouse ' || _f "no USB mouse was enumerated"
    P="$(printf '%s\n' "$S" | grep -aoE '^  mouse: slot [0-9]+, [0-9]+ reports, pointer at [0-9]+,[0-9]+, buttons 000, 0 clicks$' |
        sed -E 's/.*pointer at ([0-9]+),([0-9]+),.*/\1 \2/')"
    A="$(printf '%s\n' "$S" | grep -aoE '^  mouse: acceleration (on|off), [0-9]+ report\(s\) sped up$' |
        sed -E 's/.*acceleration (on|off), ([0-9]+) .*/\1 \2/')"
    if [ "$(printf '%s\n' "$P" | grep -c .)" -ne 3 ] || [ "$(printf '%s\n' "$A" | grep -c .)" -ne 3 ]; then
        _f "expected three lsusb mouse readings (pointer and acceleration), got: $(printf '%s' "$P" | tr '\n' ';') / $(printf '%s' "$A" | tr '\n' ';')"
    else
        read -r px1 py1 px2 py2 px3 py3 <<< "$(echo $P)"
        read -r a1 n1 a2 n2 a3 n3 <<< "$(echo $A)"
        [ "$px1,$py1" = "20,40" ] || _f "the slow move did not land exactly at 20,40 (1:1): $px1,$py1"
        [ "$a1 $n1" = "on 0" ] || _f "after the slow move: expected acceleration on with 0 reports sped up, got $a1 $n1"
        [ "$py2" = "40" ] || _f "the sideways flick moved the pointer up or down: y $py2"
        [ "$px2" -ge 340 ] || _f "the flick of 300 counts was not sped up: x $px2 (1:1 is 320)"
        { [ "$a2" = "on" ] && [ "$n2" -ge 1 ]; } || _f "after the flick: expected acceleration on with at least 1 report sped up, got $a2 $n2"
        [ "$px3,$py3" = "$((px2 - 300)),40" ] || _f "with acceleration off the flick of -300 was not 1:1: $px2 -> $px3,$py3"
        [ "$a3 $n3" = "off $n2" ] || _f "with acceleration off: expected off with still $n2 sped up, got $a3 $n3"
    fi
    has '^usb: mouse acceleration off \(1:1\); [0-9]+ report\(s\) sped up before$' || _f "\`accel off\` did not name the USB mouse"
    has '^usb: mouse acceleration on, the same curve at 16 counts/mm \(400 dpi\); [0-9]+ report\(s\) sped up$' ||
        _f "\`accel on\` did not name the USB mouse"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[P[0-9]+\] fault:'; then _f "the kernel panicked or a process faulted"; fi
    return "$fail"
}

GREEN="[xhci] port 6: USB mouse 0627:0001 (high speed), slot 2, interrupt-IN endpoint 1 every 8000 us
afrisovera> lsusb
  mouse: slot 2, 8 reports, pointer at 20,40, buttons 000, 0 clicks
  mouse: 0 window drag(s)
  mouse: acceleration on, 0 report(s) sped up
afrisovera> lsusb
  mouse: slot 2, 11 reports, pointer at 639,40, buttons 000, 0 clicks
  mouse: 0 window drag(s)
  mouse: acceleration on, 2 report(s) sped up
afrisovera> accel off
ps2: acceleration off (1:1); \`accel on\` switches it on
usb: mouse acceleration off (1:1); 2 report(s) sped up before
afrisovera> lsusb
  mouse: slot 2, 14 reports, pointer at 339,40, buttons 000, 0 clicks
  mouse: 0 window drag(s)
  mouse: acceleration off, 2 report(s) sped up
afrisovera> accel on
usb: mouse acceleration on, the same curve at 16 counts/mm (400 dpi); 2 report(s) sped up"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbaccel-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-09 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm slow-not-exact red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 20,40/pointer at 23,42/')"
    arm slow-sped-up red "$(printf '%s\n' "$GREEN" | sed '0,/acceleration on, 0 report/s//acceleration on, 1 report/')"
    arm flick-not-faster red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 639,40/pointer at 320,40/;s/pointer at 339,40/pointer at 20,40/')"
    arm flick-moved-y red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 639,40/pointer at 639,44/')"
    arm flick-none-sped red "$(printf '%s\n' "$GREEN" | sed 's/acceleration on, 2 report(s) sped up/acceleration on, 0 report(s) sped up/;s/acceleration off, 2 report/acceleration off, 0 report/')"
    arm off-not-1to1 red "$(printf '%s\n' "$GREEN" | sed 's/pointer at 339,40/pointer at 200,40/')"
    arm off-still-sped red "$(printf '%s\n' "$GREEN" | sed 's/acceleration off, 2 report/acceleration off, 3 report/')"
    arm off-not-off red "$(printf '%s\n' "$GREEN" | sed 's/  mouse: acceleration off,/  mouse: acceleration on,/')"
    arm no-off-line red "$(printf '%s\n' "$GREEN" | grep -v '^usb: mouse acceleration off')"
    arm no-on-line red "$(printf '%s\n' "$GREEN" | grep -v '^usb: mouse acceleration on')"
    arm two-readings red "$(printf '%s\n' "$GREEN" | grep -v 'pointer at 339,40')"
    arm no-mouse red "$(printf '%s\n' "$GREEN" | grep -v 'USB mouse')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC USB mouse acceleration =="
if ! selftest; then echo "PC USBACCEL TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBACCEL SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBACCEL TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBACCEL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-usbaccel.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5740,server,nowait" -qmp "tcp:127.0.0.1:5741,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 5740 5741 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "move:-300,-200" "type:lsusb" "fast:300,0" "type:lsusb" "type:accel off" "fast:-300,0" "type:lsusb" "type:accel on"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE 'USB mouse|^afrisovera> (lsusb|accel)|  mouse:|^usb: mouse|^ps2: accel' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBACCEL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBACCEL TEST FAILED"
exit 1
