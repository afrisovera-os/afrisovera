#!/usr/bin/env bash
# PC track step 3 (13 Nov 2026): Caps Lock on a USB keyboard.
#
# Before, a USB keyboard's Caps Lock key did nothing (only the PS/2 keyboard had it). Now it
# turns letters to capitals, Shift with it gives small letters, digits are not touched, and the
# keyboard's own Caps Lock light is set by a HID SET_REPORT from the idle loop.
# Boots the PC kernel with QEMU's qemu-xhci and usb-kbd; scripts/lib/usb-type.py types on the
# USB keyboard through QMP: "echo ab", Caps Lock, "cd1E", Caps Lock, "fg" and Enter, then lsusb.
# Graded: the shell got "echo abCD1efg" and printed it, and lsusb says Caps Lock is off again,
# the light set twice (once on, once off) and no request refused.
#   scripts/pc-usbcaps-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbcaps-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
WORD="abCD1efg"

grade() { # grade <serial capture>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' ||
        _f "no USB keyboard was enumerated"
    printf '%s\n' "$S" | grep -aqE "^afrisovera> echo $WORD$" ||
        _f "the line typed with Caps Lock did not reach the shell as echo $WORD"
    printf '%s\n' "$S" | grep -aqE "^$WORD$" || _f "the shell did not print $WORD"
    local l n r
    l="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: caps lock (on|off), its light set [0-9]+ times, [0-9]+ refused$' | tail -1)"
    if [ -z "$l" ]; then
        _f "lsusb did not report Caps Lock"
    else
        n="$(awk '{print $8}' <<< "$l")"
        r="$(awk '{print $10}' <<< "$l")"
        grep -q 'caps lock off' <<< "$l" || _f "Caps Lock was left on: $l"
        [ "$n" -ge 2 ] || _f "the keyboard's light was set only $n times (want 2: on, then off)"
        [ "$r" -eq 0 ] || _f "the keyboard refused $r light requests"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)'; then
        _f "$(printf '%s\n' "$S" | grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)|^\[xhci\] .*(failed|timed out|did not)' | head -1)"
    fi
    return "$fail"
}

GREEN="[xhci] controller 00:03.0 (xHCI 1.0), 8 ports, 64 slots, 32-byte contexts
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  port 1: connected, enabled, speed 1  <- keyboard
  keyboard: slot 1, 40 reports, 20 keys typed
  keyboard: 0 repeats while a key was held
  keyboard: caps lock off, its light set 2 times, 0 refused
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbcaps-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-keyboard red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard')"
    arm caps-ignored red "$(printf '%s\n' "$GREEN" | sed 's/abCD1efg/abcd1Efg/')"
    arm shift-not-inverted red "$(printf '%s\n' "$GREEN" | sed 's/abCD1efg/abCD1Efg/')"
    arm not-run red "$(printf '%s\n' "$GREEN" | grep -vx "$WORD")"
    arm no-caps-line red "$(printf '%s\n' "$GREEN" | grep -v 'caps lock')"
    arm left-on red "$(printf '%s\n' "$GREEN" | sed 's/caps lock off/caps lock on/')"
    arm light-once red "$(printf '%s\n' "$GREEN" | sed 's/set 2 times/set 1 times/')"
    arm refused red "$(printf '%s\n' "$GREEN" | sed 's/, 0 refused/, 2 refused/')"
    arm command-failed red "$(printf '%s\n[xhci] control request 0x09 timed out' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== PC USB keyboard Caps Lock (PC track step 3) =="
if ! selftest; then echo "PC USBCAPS TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBCAPS SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBCAPS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBCAPS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbcaps.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4978,server,nowait" -qmp "tcp:127.0.0.1:4979,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 4978 4979 "$W/serial.log" \
    "@raw echo ab" "@key caps_lock" "@raw cd1E" "@key caps_lock" "fg" "lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> (echo|lsusb)|^usb:|^  (port|keyboard)|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBCAPS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBCAPS TEST FAILED"
exit 1
