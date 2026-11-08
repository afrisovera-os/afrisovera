#!/usr/bin/env bash
# PC track step 6 groundwork (5 Nov 2026): the Tonga desktop's dock answers the USB mouse.
#
# Boots the PC kernel with a USB keyboard and mouse. scripts/lib/usb-steps.py types `gui` on
# the keyboard (the shared Tonga desktop, its dock of SHELL FILES STORE ASILI-AI DEVICES),
# moves the pointer from 320,240 onto the FILES tile (201,430), clicks, and types `lsusb`.
# A dock click runs that tile's command as if typed: `clear` (back to the console) then
# `ls`. Graded: the file list printed after the click, and lsusb counting one dock click.
#   scripts/pc-desktop-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-desktop-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <serial>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[gui\] Tonga desktop rendered' || _f "gui (typed on the USB keyboard) did not draw the desktop"
    printf '%s\n' "$S" | sed -n '/^\[gui\] Tonga desktop rendered/,$p' | grep -aqE '^afrisovera> clear$' ||
        _f "the dock click did not take the screen back to the console"
    printf '%s\n' "$S" | sed -n '/^\[gui\] Tonga desktop rendered/,$p' | grep -aqE '^afrisovera> ls$' ||
        _f "the FILES tile did not run ls"
    has '^ +[0-9]+  hello\.wasm$' || _f "ls did not list the files"
    has '^  mouse: 1 dock click\(s\) on the desktop$' || _f "lsusb does not count exactly one dock click"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> clear
afrisovera> ls
   117  hello.wasm
afrisovera> lsusb
  mouse: 1 dock click(s) on the desktop"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-desktop-selftest.XXXXXX")" || return 1
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
    arm no-desktop red "$(printf '%s\n' "$GREEN" | grep -v '^\[gui\]')"
    arm click-ignored red "$(printf '%s\n' "$GREEN" | grep -v '^afrisovera> \(clear\|ls\)$' | grep -v hello)"
    arm wrong-tile red "$(printf '%s\n' "$GREEN" | sed 's/^afrisovera> ls$/afrisovera> store/' | grep -v hello)"
    arm no-count red "$(printf '%s\n' "$GREEN" | sed 's/1 dock click/0 dock click/')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 7 ]
}

echo "== PC desktop dock and the USB mouse =="
if ! selftest; then echo "PC DESKTOP TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC DESKTOP SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC DESKTOP TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DESKTOP TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-desktop.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4814,server,nowait" -qmp "tcp:127.0.0.1:4815,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4814 4815 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:gui" "move:-119,190" down up "wait:3" "type:lsusb"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[gui\]|^afrisovera> (gui|clear|ls|lsusb)$|hello\.wasm|dock click' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC DESKTOP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DESKTOP TEST FAILED"
exit 1
