#!/usr/bin/env bash
# PC track step 6 groundwork (5 Nov 2026): the Tonga desktop's dock answers the USB mouse.
#
# Boots the PC kernel with a USB keyboard and mouse. scripts/lib/usb-steps.py types `desk` on
# the keyboard (since 9 Nov 2026, field item F8: `desk` alone opens it, as `gui` does) (the shared desktop, desk.rs: its dock of launcher, Files, Terminal, Settings,
# About), moves the pointer from 320,240 onto the Files tile (281,444), clicks, and types
# `lsusb`. Since 8 Nov 2026 (docs/design/desktop.md) a dock click opens that app's window on
# the desktop instead of running a command on the text console. Graded: the Files window
# opened after the click, the desktop still on screen, lsusb counting one dock click, and
# `desk status` (typed last) showing the desktop on screen with that one click.
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
    has '^\[desk\] desktop opened' || _f "desk (typed on the USB keyboard) did not open the desktop"
    if has '\[desk\] [a-z]+ profile, hidden;'; then _f "desk alone printed the hidden desktop's status instead of opening it"; fi
    printf '%s\n' "$S" | sed -n '/^\[desk\] desktop opened/,$p' | grep -aqE '\[desk\] open files$' ||
        _f "the Files tile did not open the Files window"
    if printf '%s\n' "$S" | sed -n '/^\[desk\] desktop opened/,$p' | grep -aqE '^afrisovera> clear$'; then
        _f "the dock click left the desktop for the console"
    fi
    has '^  mouse: 1 dock click\(s\) on the desktop$' || _f "lsusb does not count exactly one dock click"
    has '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 1 dock click\(s\)' ||
        _f "desk status does not show the desktop on screen with one dock click"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> desk
[desk] desktop opened; type 'desk status' for its state, 'clear' to return to console
afrisovera> [desk] open files
lsusb
  mouse: 1 dock click(s) on the desktop
afrisovera> desk status
[desk] pointer profile, on screen; 3 frame(s) drawn; 1 dock click(s); 1 notice(s); 0 event(s) dropped
[desk] window files at 36,44 size 372x250 focused"

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
    arm green-2026-11-08 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-desktop red "$(printf '%s\n' "$GREEN" | grep -v '^\[desk\] desktop opened')"
    arm status-only-2026-11-08 red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] desktop opened.*$/[desk] pointer profile, hidden; 0 frame(s) drawn; 0 dock click(s); 1 notice(s); 0 event(s) dropped/')"
    arm no-status red "$(printf '%s\n' "$GREEN" | grep -v 'on screen;')"
    arm click-ignored red "$(printf '%s\n' "$GREEN" | grep -v 'open files')"
    arm wrong-tile red "$(printf '%s\n' "$GREEN" | sed 's/open files/open terminal/')"
    arm left-desktop red "$(printf '%s\n' "$GREEN" | sed 's/^lsusb$/afrisovera> clear\nlsusb/')"
    arm no-count red "$(printf '%s\n' "$GREEN" | sed 's/1 dock click/0 dock click/')"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
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
    "type:desk" "move:-39,204" down up "wait:3" "type:lsusb" "type:desk status"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '\[desk\]|^afrisovera> (desk|desk status|clear|lsusb)$|dock click' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC DESKTOP TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DESKTOP TEST FAILED"
exit 1
