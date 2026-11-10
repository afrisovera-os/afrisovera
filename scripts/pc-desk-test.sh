#!/usr/bin/env bash
# The AfriSovera desktop on the PC, driven by a USB mouse and keyboard (8 Nov 2026,
# docs/design/desktop.md section 6).
#
# Boots the PC kernel with a USB keyboard and mouse. scripts/lib/usb-steps.py types `gui`,
# then, from the pointer's start at 320,240:
#   click the dock's Terminal tile (327,444)            -> Terminal opens at 190,112
#   click the dock's Files tile (281,444)               -> Files opens at 36,44, focused
#   press on Files' title bar at 200,50, move +60,+40, release -> Files moved to 96,84
#   Alt+Tab                                             -> Terminal focused
#   Super (alone), then Esc                             -> launcher opens, closes
#   Super+Down                                          -> Terminal minimised (no launcher)
#   Alt+F4                                              -> Files closed
#   Alt+Tab                                             -> the minimised Terminal comes back
# and types `lsusb` and `desk list` on the keyboard (into the shell the Terminal shows).
# Graded: every step's serial line in order, 2 dock clicks, the final window list, and the
# screenshot's PIXELS: top bar, kente band, dock, Terminal's gold focus line and body, and
# the shell's own text drawn inside the Terminal window.
#   scripts/pc-desk-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-desk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

POINTS="5,5 4,23 20,23 198,446 300,112 595,365 count:192,134,406,236,C8C2B6"
want_px() {
    case "$1" in
        5,5) echo 0B0B0D ;;      # the top bar
        4,23) echo C99E44 ;;     # kente band: gold
        20,23) echo 1B7A3D ;;    #             green
        198,446) echo 141519 ;;  # the dock
        300,112) echo F4CB57 ;;  # Terminal (focused): gold line along its title bar
        595,365) echo 0C0C0E ;;  # Terminal's body
    esac
}

grade() { # grade <serial> <pixels>
    local fail=0 S P p got n
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    local steps=(
        '^\[gui\] Tonga desktop rendered'
        '\[desk\] open terminal$'
        '\[desk\] open files$'
        '\[desk\] moved files to 96,84$'
        '\[desk\] focus terminal$'
        '\[desk\] launcher open$'
        '\[desk\] launcher closed$'
        '\[desk\] minimise terminal$'
        '\[desk\] close files$'
        '\[desk\] focus terminal$'
        '^  mouse: 2 dock click\(s\) on the desktop$'
        '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 2 dock click\(s\); 1 notice\(s\); 0 event\(s\) dropped$'
        '\[desk\] window terminal at 190,112 size 410x260 focused$'
        '\[desk\] front to back: terminal$'
    )
    p="$(printf '%s\n' "$S" | DESK_PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    if printf '%s\n' "$S" | grep -aqE '\[desk\] launcher open$' && [ "$(printf '%s\n' "$S" | grep -acE '\[desk\] launcher open$')" -ne 1 ]; then
        _f "the launcher opened more than once (Super+Down must not open it)"
    fi
    if printf '%s\n' "$S" | grep -aqE '\[desk\] window files'; then _f "Files is still open after Alt+F4"; fi
    for p in $POINTS; do
        case "$p" in count:*) continue ;; esac
        got="$(printf '%s\n' "$P" | sed -n "s/^$p=\([0-9A-F]\{6\}\)$/\1/p" | head -1)"
        [ "$got" = "$(want_px "$p")" ] || _f "pixel $p is ${got:-missing}, expected $(want_px "$p")"
    done
    n="$(printf '%s\n' "$P" | sed -n 's/^count:[^=]*=\([0-9]*\)$/\1/p' | head -1)"
    [ -n "$n" ] && [ "$n" -ge 300 ] || _f "only ${n:-no} pixels of shell text inside the Terminal window"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> [desk] open terminal
[desk] open files
[desk] moved files to 96,84
[desk] focus terminal
[desk] launcher open
[desk] launcher closed
[desk] minimise terminal
[desk] close files
[desk] focus terminal
lsusb
usb: xHCI 1.0 at 0xfebf0000
  mouse: 2 dock click(s) on the desktop
afrisovera> desk list
[desk] pointer profile, on screen; 14 frame(s) drawn; 2 dock click(s); 1 notice(s); 0 event(s) dropped
[desk] window terminal at 190,112 size 410x260 focused
[desk] front to back: terminal"
GPX="$(for p in $POINTS; do case "$p" in count:*) echo "$p=812" ;; *) echo "$p=$(want_px "$p")" ;; esac; done)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-desk-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/s"
        printf '%s\n' "$4" > "$D/p"
        if grade "$D/s" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GPX"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GPX"
    arm no-desktop red "$(printf '%s\n' "$GREEN" | grep -v '^\[gui\]')" "$GPX"
    arm click-missed red "$(printf '%s\n' "$GREEN" | sed 's/open terminal$/nothing/')" "$GPX"
    arm drag-wrong red "$(printf '%s\n' "$GREEN" | sed 's/moved files to 96,84/moved files to 95,84/')" "$GPX"
    arm no-alt-tab red "$(printf '%s\n' "$GREEN" | sed '0,/^\[desk\] focus terminal$/{/^\[desk\] focus terminal$/d}')" "$GPX"
    arm super-down-opened-launcher red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] minimise terminal$/[desk] launcher open\n[desk] minimise terminal/')" "$GPX"
    arm alt-f4-ignored red "$(printf '%s\n' "$GREEN" | grep -v 'close files' | sed 's/^\[desk\] window terminal/[desk] window files at 96,84 size 372x250\n[desk] window terminal/')" "$GPX"
    arm dock-count red "$(printf '%s\n' "$GREEN" | sed 's/2 dock click/3 dock click/g')" "$GPX"
    arm not-restored red "$(printf '%s\n' "$GREEN" | sed 's/size 410x260 focused/size 410x260 minimised/')" "$GPX"
    arm no-focus-line red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^300,112=.*/300,112=18191E/')"
    arm empty-terminal red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^\(count:[^=]*\)=.*/\1=0/')"
    arm console-not-desktop red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/=[0-9A-F]\{6\}$/=080B12/')"
    arm no-screenshot red "$GREEN" "error=not-a-png"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== the desktop on the PC: USB mouse and keyboard =="
if ! selftest; then echo "PC DESK TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC DESK SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC DESK TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DESK TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-desk.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4942,server,nowait" -qmp "tcp:127.0.0.1:4943,server,nowait" &
QPID=$!
python3 scripts/lib/usb-steps.py 4942 4943 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:gui" "move:7,204" down up "wait:1" "move:-46,0" down up "wait:1" \
    "move:-81,-394" down "move:60,40" up "wait:1" \
    "keys:alt+tab" "keys:meta_l" "keys:esc" "keys:meta_l+down" "keys:alt+f4" "keys:alt+tab" "wait:1" \
    "type:lsusb" "type:desk list" "wait:2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
python3 scripts/lib/png-px.py "$W/shot.png" $POINTS > "$W/pixels.txt" 2>&1
tr -d '\r' < "$W/serial.log" | grep -aE '^\[gui\]|\[desk\]|dock click' | sed 's/^/  | /'
sed 's/^/  px /' "$W/pixels.txt"
[ -n "${DESK_SHOT:-}" ] && cp "$W/shot.png" "$DESK_SHOT"
[ -n "${DESK_SHOT_DIR:-}" ] && cp "$W/shot.png" "$DESK_SHOT_DIR/pc-desk.png"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/pixels.txt"; then
    rm -rf "$W"
    echo "PC DESK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC DESK TEST FAILED"
exit 1
