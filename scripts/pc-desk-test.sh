#!/usr/bin/env bash
# The AfriSovera desktop on the PC, driven by a USB mouse and keyboard (8 Nov 2026,
# docs/design/desktop.md section 6).
#
# Since 10 Nov 2026 the PC desktop is the Sovereign UI (docs/design/sovereign-ui.md): the Spine
# down the left edge instead of a top bar and dock, cut corners, each app's woven cloth on the
# focused window's lintel. The phone keeps the 8 Nov desktop (scripts/desk-test.sh).
# Boots the PC kernel with a USB keyboard and mouse. scripts/lib/usb-steps.py types `gui`,
# then, from the pointer's start at 320,240:
#   click the Spine's Terminal tile (20,108)            -> Terminal opens at 210,120
#   click the Spine's Files tile (20,68)                -> Files opens at 60,20, focused
#   press on Files' lintel at 200,34, move +60,+40, release -> Files moved to 120,60
#   Alt+Tab                                             -> Terminal focused
#   Super (alone), then Esc                             -> launcher opens, closes
#   Super+Down                                          -> Terminal minimised (no launcher)
#   Alt+F4                                              -> Files closed
#   Alt+Tab                                             -> the minimised Terminal comes back
# and types `lsusb`, `desk list` and `screen ui` on the keyboard (into the shell the Terminal shows).
# Graded: every step's serial line in order, 2 dock clicks, the final window list, and the
# screenshot's PIXELS: the Spine and its woven edge, Terminal's cloth (malachite) on its lintel
# and the lintel under it, Terminal's body, and the shell's own text drawn inside the Terminal.
# Since 10 Nov 2026 (sovereign-ui.md section 13, field item F9) the desktop fills the screen at
# its own resolution: `screen ui` names the desktop's size and UI scale, and the screenshot must
# be exactly that size times the scale (no frame), the Spine must reach the bottom of the
# screen, and the far corner must be the desktop's wallpaper, not the frame (#0A0E14).
#   scripts/pc-desk-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-desk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

POINTS="20,250 40,4 40,20 42,8 308,120 308,127 595,365 count:212,148,400,228,D8CEBE"
want_px() {
    case "$1" in
        20,250) echo 100C0A ;;   # the Spine (night)
        40,4) echo C8963A ;;     # its woven edge: gold-deep
        40,20) echo 2E9E6A ;;    #                 malachite
        42,8) echo C1121F ;;     #                 red (second column, offset half a block)
        308,120) echo 2E9E6A ;;  # Terminal (focused): its cloth's malachite, on top of its lintel
        308,127) echo 2A211B ;;  # Terminal's lintel under the cloth
        595,365) echo 0E0B09 ;;  # Terminal's body
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
        '\[desk\] moved files to 120,60$'
        '\[desk\] focus terminal$'
        '\[desk\] launcher open$'
        '\[desk\] launcher closed$'
        '\[desk\] minimise terminal$'
        '\[desk\] close files$'
        '\[desk\] focus terminal$'
        '^  mouse: 2 dock click\(s\) on the desktop$'
        '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 2 dock click\(s\); 1 notice\(s\); 0 event\(s\) dropped$'
        '\[desk\] window terminal at 210,120 size 410x260 focused$'
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
    # the whole screen (section 13)
    local ui dw dh sc size
    ui="$(printf '%s\n' "$S" | sed -n 's/^screen: the desktop is \([0-9]*\)x\([0-9]*\) logical pixels at UI scale \([0-9]\)x.*$/\1 \2 \3/p' | tail -1)"
    if [ -z "$ui" ]; then
        _f "screen ui did not name the desktop's size and scale"
    else
        read -r dw dh sc <<< "$ui"
        size="$(printf '%s\n' "$P" | sed -n 's/^size=//p' | head -1)"
        if [ -n "${PC_DESK_CAPPED:-}" ]; then
            # a screen bigger than the canvas at this scale (pc-deskbig): the desktop stops at the
            # canvas's 1920x1200, centred, and the frame shows round it; nothing is drawn past it
            [ "$dw" = 1920 ] && [ "$dh" = 1200 ] || _f "a desktop bigger than the screen buffer was not cut to 1920x1200 (it is ${dw}x${dh})"
            got="$(printf '%s\n' "$P" | sed -n 's/^band=//p' | head -1)"
            [ "$got" = 0A0E14 ] || _f "the screen's corner outside the 1920x1200 desktop is ${got:-missing}, not the frame #0A0E14"
        else
            [ "$size" = "$((dw * sc))x$((dh * sc))" ] || _f "the screenshot is ${size:-missing}, but a desktop of ${dw}x${dh} at ${sc}x fills $((dw * sc))x$((dh * sc)): it does not fill the screen"
        fi
        [ "$dw" -gt 640 ] || [ "$dh" -gt 480 ] || _f "the desktop is still 640x480"
        if [ -n "${PC_DESK_SCREEN:-}" ] && [ "$size" != "$PC_DESK_SCREEN" ]; then
            _f "asked for a $PC_DESK_SCREEN screen, the screenshot is ${size:-missing}"
        fi
        got="$(printf '%s\n' "$P" | sed -n 's/^spinefoot=//p' | head -1)"
        [ "$got" = 100C0A ] || _f "the Spine does not reach the bottom of the screen (pixel ${got:-missing} near the foot)"
        got="$(printf '%s\n' "$P" | sed -n 's/^corner=//p' | head -1)"
        case "$got" in
            ""|0A0E14|000000|outside) _f "the far corner is ${got:-missing}: the frame, not the desktop" ;;
        esac
    fi
    printf '%s\n' "$S" | grep -aqE '^\[desk\] perf: [0-9]+x[0-9]+ at UI scale [0-9]x; draw [0-9]+ us \(worst [0-9]+ us\)' ||
        _f "desk perf did not report the draw and copy times"
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> [desk] open terminal
[desk] open files
[desk] moved files to 120,60
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
[desk] window terminal at 210,120 size 410x260 focused
[desk] front to back: terminal
afrisovera> screen ui
screen: the desktop is 1280x800 logical pixels at UI scale 1x; type: screen ui 1, 2, 3, 4 or auto
afrisovera> desk perf
[desk] perf: 1280x800 at UI scale 1x; draw 2210 us (worst 4120 us) over 31 frame(s); copy to the screen 180 us (worst 2950 us) over 40 copies"
GPX="$(for p in $POINTS; do case "$p" in count:*) echo "$p=812" ;; *) echo "$p=$(want_px "$p")" ;; esac; done)
size=1280x800
spinefoot=100C0A
corner=0B0908"

selftest() {
    local PC_DESK_SCREEN="" PC_DESK_CAPPED="" # the arms choose their own screen size
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
    arm drag-wrong red "$(printf '%s\n' "$GREEN" | sed 's/moved files to 120,60/moved files to 119,60/')" "$GPX"
    arm no-alt-tab red "$(printf '%s\n' "$GREEN" | sed '0,/^\[desk\] focus terminal$/{/^\[desk\] focus terminal$/d}')" "$GPX"
    arm super-down-opened-launcher red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] minimise terminal$/[desk] launcher open\n[desk] minimise terminal/')" "$GPX"
    arm alt-f4-ignored red "$(printf '%s\n' "$GREEN" | grep -v 'close files' | sed 's/^\[desk\] window terminal/[desk] window files at 120,60 size 372x250\n[desk] window terminal/')" "$GPX"
    arm dock-count red "$(printf '%s\n' "$GREEN" | sed 's/2 dock click/3 dock click/g')" "$GPX"
    arm not-restored red "$(printf '%s\n' "$GREEN" | sed 's/size 410x260 focused/size 410x260 minimised/')" "$GPX"
    arm no-focus-line red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^308,120=.*/308,120=1C1613/')"
    arm empty-terminal red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^\(count:[^=]*\)=.*/\1=0/')"
    arm console-not-desktop red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/=[0-9A-F]\{6\}$/=080B12/')"
    arm no-screenshot red "$GREEN" "error=not-a-png"
    arm empty red "" ""
    arm boxed-640x480 red "$(printf '%s\n' "$GREEN" | sed 's/is 1280x800 logical/is 640x480 logical/')" "$(printf '%s\n' "$GPX" | sed 's/^size=.*/size=1280x800/')"
    arm frame-around red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^size=.*/size=1920x1080/')"
    arm spine-short red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^spinefoot=.*/spinefoot=0B0908/')"
    arm corner-is-frame red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^corner=.*/corner=0A0E14/')"
    arm no-screen-ui red "$(printf '%s\n' "$GREEN" | grep -v '^screen: the desktop')" "$GPX"
    arm no-perf red "$(printf '%s\n' "$GREEN" | grep -v '^\[desk\] perf')" "$GPX"
    PC_DESK_SCREEN=1920x1080 arm wrong-screen-size red "$GREEN" "$GPX"
    PC_DESK_SCREEN=1280x800 arm asked-screen-size green "$GREEN" "$GPX"
    local BIG BIGPX
    BIG="$(printf '%s\n' "$GREEN" | sed 's/is 1280x800 logical/is 1920x1200 logical/')"
    BIGPX="$(printf '%s\n' "$GPX" | sed 's/^size=.*/size=2560x1440/')
band=0A0E14"
    PC_DESK_SCREEN=2560x1440 PC_DESK_CAPPED=1 arm capped-1920x1200 green "$BIG" "$BIGPX"
    PC_DESK_SCREEN=2560x1440 PC_DESK_CAPPED=1 arm drew-past-the-buffer red "$BIG" "$(printf '%s\n' "$BIGPX" | sed 's/^band=.*/band=0B0908/')"
    PC_DESK_SCREEN=2560x1440 PC_DESK_CAPPED=1 arm not-cut red "$(printf '%s\n' "$GREEN" | sed 's/is 1280x800 logical/is 2560x1440 logical/')" "$BIGPX"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 26 ]
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
# PC_DESK_SCREEN=WxH (suites pc-desk1080, pc-desk768): the firmware is offered that screen mode
# through QEMU's VGA (EDID), and the screenshot must come back at exactly that size
VGA=()
[ -n "${PC_DESK_SCREEN:-}" ] && VGA=(-vga none -device "VGA,xres=${PC_DESK_SCREEN%x*},yres=${PC_DESK_SCREEN#*x}")
qemu-system-aarch64 -M virt -m 256M -display none "${VGA[@]}" \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4942,server,nowait" -qmp "tcp:127.0.0.1:4943,server,nowait" &
QPID=$!
# PC_DESK_UI=N (suite pc-deskbig): `screen ui N` is typed right after gui, so the walk runs at that scale
UI=()
[ -n "${PC_DESK_UI:-}" ] && UI=("type:screen ui $PC_DESK_UI")
python3 scripts/lib/usb-steps.py 4942 4943 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:gui" "${UI[@]}" "move:-300,-132" down up "wait:1" "move:0,-40" down up "wait:1" \
    "move:180,-34" down "move:60,40" up "wait:1" \
    "keys:alt+tab" "keys:meta_l" "keys:esc" "keys:meta_l+down" "keys:alt+f4" "keys:alt+tab" "wait:1" \
    "type:lsusb" "type:desk list" "type:screen ui" "type:desk perf" "wait:2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
# the desktop's size and UI scale, as `screen ui` named them (1x if it did not: the grader fails it)
read -r DW DH SC <<< "$(tr -d '\r' < "$W/serial.log" | sed -n 's/^screen: the desktop is \([0-9]*\)x\([0-9]*\) logical pixels at UI scale \([0-9]\)x.*$/\1 \2 \3/p' | tail -1)"
DW="${DW:-640}" DH="${DH:-480}" SC="${SC:-1}"
{
    python3 scripts/lib/png-px.py "$W/shot.png" --desk "$SC" $POINTS size
    python3 scripts/lib/png-px.py "$W/shot.png" --desk "$SC" "4,$((DH - 150))" "$((DW - 6)),$((DH - 6))" |
        sed "1s/^[^=]*=/spinefoot=/; 2s/^[^=]*=/corner=/"
    python3 scripts/lib/png-px.py "$W/shot.png" raw:2,2 | sed 's/^[^=]*=/band=/'
} > "$W/pixels.txt" 2>&1
tr -d '\r' < "$W/serial.log" | grep -aE '^\[gui\]|\[desk\]|dock click|^screen: the desktop' | sed 's/^/  | /'
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
