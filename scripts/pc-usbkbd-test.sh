#!/usr/bin/env bash
# PC track step 3 (5 Nov 2026): xHCI and a USB keyboard.
#
# Boots the PC kernel (q35 + UEFI) with an xHCI controller (QEMU `qemu-xhci`) and a USB
# keyboard on it (`usb-kbd`). Nothing is typed on the serial line: scripts/lib/usb-type.py
# presses keys on the USB keyboard through QMP, and the shell's answers are read from COM1.
# Graded: the controller and the keyboard are enumerated, a command typed on the USB
# keyboard runs (with a shifted character in it), and `lsusb`, also typed on it, reports
# the keyboard's report and key counters.
#   scripts/pc-usbkbd-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-usbkbd-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
WORD="usbkbd-OK_5nov"

grade() { # grade <serial capture>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^\[xhci\] controller [0-9a-f]{2}:[0-9a-f]{2}\.[0-7] \(xHCI [0-9]+\.[0-9]+\), [0-9]+ ports, [0-9]+ slots' ||
        _f "the xHCI controller did not come up"
    has '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+, interrupt-IN endpoint [0-9]+' ||
        _f "no USB keyboard was enumerated"
    # the typed command and its output: the echo line, then the word alone on the next line
    printf '%s\n' "$S" | grep -aqE "^afrisovera> echo $WORD$" || _f "the command typed on the USB keyboard did not reach the shell"
    printf '%s\n' "$S" | grep -aqE "^$WORD$" || _f "the shell did not run the command typed on the USB keyboard"
    local k
    k="$(printf '%s\n' "$S" | grep -aoE '^  keyboard: slot [0-9]+, [0-9]+ reports, [0-9]+ keys typed$' | tail -1 | awk '{print $6}')"
    if [ -z "$k" ]; then
        _f "lsusb (typed on the USB keyboard) did not report the keyboard"
    elif [ "$k" -lt $(( ${#WORD} + 6 )) ]; then
        _f "the keyboard counted only $k keys"
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
  keyboard: slot 1, 44 reports, 21 keys typed
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-usbkbd-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-controller red "$(printf '%s\n' "$GREEN" | grep -v '^\[xhci\] controller')"
    arm no-keyboard red "$(printf '%s\n' "$GREEN" | grep -v 'USB keyboard')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v "^afrisovera> echo")"
    arm shift-lost red "$(printf '%s\n' "$GREEN" | sed 's/usbkbd-OK_5nov/usbkbd-ok-5nov/')"
    arm not-run red "$(printf '%s\n' "$GREEN" | grep -vx "$WORD")"
    arm no-lsusb red "$(printf '%s\n' "$GREEN" | grep -v '^  keyboard:')"
    arm too-few-keys red "$(printf '%s\n' "$GREEN" | sed 's/21 keys typed/3 keys typed/')"
    arm command-failed red "$(printf '%s\n[xhci] control request 0x06 timed out' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 11 ]
}

echo "== PC xHCI + USB keyboard (PC track step 3) =="
if ! selftest; then echo "PC USBKBD TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC USBKBD SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC USBKBD TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC USBKBD TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-usbkbd.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4954,server,nowait" -qmp "tcp:127.0.0.1:4955,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 4954 4955 "$W/serial.log" "echo $WORD" "lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[xhci\]|^afrisovera> (echo|lsusb)|^usb:|^  (port|keyboard)|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC USBKBD TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC USBKBD TEST FAILED"
exit 1
