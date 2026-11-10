#!/usr/bin/env bash
# The AfriSovera desktop on the PC, driven by a USB mouse and keyboard (8 Nov 2026,
# docs/design/desktop.md section 6).
#
# Since 10 Nov 2026 the PC desktop is the Sovereign UI (docs/design/sovereign-ui.md): the Spine
# down the left edge instead of a top bar and dock, cut corners, each app's woven cloth on the
# focused window's lintel. The phone keeps the 8 Nov desktop (scripts/desk-test.sh).
# Section 13: the desktop fills the screen at its own resolution. Section 14 (slice 2): windows
# open round the middle of the free space, so their places depend on the screen's size; this
# script works them out the way the kernel does (layout below), for the screen it asks for.
#
# Boots the PC kernel with a USB keyboard and mouse. scripts/lib/usb-steps.py types `gui`,
# then, from the pointer's start at 320,240:
#   click the Spine's Terminal tile (20,108)            -> Terminal opens at its home
#   click the Spine's Files tile (20,68)                -> Files opens at its home, focused
#   press on Files' lintel, move +60,+40, release        -> Files moved by exactly that
#   Alt+Tab                                             -> Terminal focused
#   Super (alone), then Esc                             -> the Circle opens, closes
#   Super+Down                                          -> Terminal minimised (no Circle)
#   Alt+F4                                              -> Files closed
#   Alt+Tab                                             -> the minimised Terminal comes back
#   Super, Right, Right, Enter (the keyboard walk)      -> the Circle walks to Terminal, then
#                                                          Settings, and Enter opens Settings
#   Super+2                                             -> Terminal (the Spine's second app) focused
# and types `lsusb`, `desk list`, `screen ui` and `desk perf` on the keyboard.
# Graded: every step's serial line in order, 2 dock clicks, the final window list, and the
# screenshot's PIXELS: the Spine and its woven edge, Terminal's cloth (malachite) on its lintel
# and the lintel under it, Terminal's body, and the shell's own text drawn inside the Terminal;
# and the whole screen: the screenshot is exactly the desktop's size times its UI scale (no
# frame), the Spine reaches the bottom, the far corner is the wallpaper, not the frame (#0A0E14).
# PC_DESK_SCREEN=WxH asks for that screen (pc-desk1080, pc-desk768); PC_DESK_UI=N types
# `screen ui N` after gui; PC_DESK_CAPPED=1 expects the desktop cut to 1920x1200 (pc-deskbig).
#   scripts/pc-desk-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-desk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# The desktop's layout for the screen asked for, as kernel/src/pc/canvas.rs (setup) and
# kernel/src/sovereign/desk.rs (home) work it out. Sets DW DH SC, and the homes of Files (FX FY),
# Terminal (TX TY) and Settings (SX SY). QEMU's own screen mode is 1280x800.
layout() {
    local scr="${PC_DESK_SCREEN:-1280x800}" sw sh s spine=44 cx cy
    sw="${scr%x*}" sh="${scr#*x}"
    [ "$sw" -gt 4096 ] && sw=4096
    if [ -n "${PC_DESK_UI:-}" ]; then s="$PC_DESK_UI"; else
        s=$((sw / 800)); [ $((sh / 500)) -lt "$s" ] && s=$((sh / 500))
    fi
    [ "$s" -lt 1 ] && s=1
    [ "$s" -gt 4 ] && s=4
    while [ "$s" -gt 1 ] && { [ $((sw / s)) -lt 640 ] || [ $((sh / s)) -lt 480 ]; }; do s=$((s - 1)); done
    SC="$s" DW=$((sw / s)) DH=$((sh / s))
    [ "$DW" -lt 640 ] && DW=640
    [ "$DW" -gt 1920 ] && DW=1920
    [ "$DH" -lt 480 ] && DH=480
    [ "$DH" -gt 1200 ] && DH=1200
    cx=$((spine + (DW - spine) / 2)) cy=$((DH / 2))
    home() { # home <dx> <dy> <w> <h> -> "x y"
        local w="$3" h="$4" x y
        [ "$w" -gt $((DW - spine - 8)) ] && w=$((DW - spine - 8))
        [ "$h" -gt $((DH - 8)) ] && h=$((DH - 8))
        x=$((cx + $1 - w / 2)) y=$((cy + $2 - h / 2))
        [ "$x" -lt $((spine + 4)) ] && x=$((spine + 4))
        [ "$x" -gt $((DW - w)) ] && x=$((DW - w))
        [ "$y" -lt 0 ] && y=0
        [ "$y" -gt $((DH - h)) ] && y=$((DH - h))
        echo "$x $y"
    }
    read -r FX FY <<< "$(home -150 -70 372 250)"
    read -r TX TY <<< "$(home 40 50 410 260)"
    read -r SX SY <<< "$(home -70 -30 380 300)"
}

# fixed points (the Spine) and the points on Terminal, named by role (want_px)
POINTS="20,250 40,4 40,20 42,8"
want_px() {
    case "$1" in
        20,250) echo 100C0A ;;   # the Spine (night)
        40,4) echo C8963A ;;     # its woven edge: gold-deep
        40,20) echo 2E9E6A ;;    #                 malachite
        42,8) echo C1121F ;;     #                 red (second column, offset half a block)
        cloth) echo 2E9E6A ;;    # Terminal (focused): its cloth's malachite, 98 px in on its top row
        lintel) echo 2A211B ;;   # Terminal's lintel under the cloth
        tbody) echo 0E0B09 ;;    # Terminal's body, near its bottom right
    esac
}

grade() { # grade <serial> <pixels>
    local fail=0 S P p got n DW DH SC FX FY TX TY SX SY
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    layout
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    local steps=(
        '^\[gui\] Tonga desktop rendered'
        '\[desk\] open terminal$'
        '\[desk\] open files$'
        "\\[desk\\] moved files to $((FX + 60)),$((FY + 40))\$"
        '\[desk\] focus terminal$'
        '\[desk\] launcher open$'
        '\[desk\] launcher closed$'
        '\[desk\] minimise terminal$'
        '\[desk\] close files$'
        '\[desk\] focus terminal$'
        '\[desk\] launcher open$'
        '\[desk\] circle: terminal$'
        '\[desk\] circle: settings$'
        '\[desk\] open settings$'
        '\[desk\] focus terminal$'
        '^  mouse: 2 dock click\(s\) on the desktop$'
        '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 2 dock click\(s\); 1 notice\(s\); 0 event\(s\) dropped$'
        "\\[desk\\] window terminal at $TX,$TY size 410x260 focused\$"
        "\\[desk\\] window settings at $SX,$SY size 380x300\$"
        '\[desk\] front to back: terminal settings$'
    )
    p="$(printf '%s\n' "$S" | DESK_PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    if [ "$(printf '%s\n' "$S" | grep -acE '\[desk\] launcher open$')" -gt 2 ]; then
        _f "the Circle opened more than twice (Super+Down and Super+2 must not open it)"
    fi
    if printf '%s\n' "$S" | grep -aqE '\[desk\] window files'; then _f "Files is still open after Alt+F4"; fi
    for p in $POINTS cloth lintel tbody; do
        got="$(printf '%s\n' "$P" | sed -n "s/^$p=\([0-9A-F]\{6\}\)$/\1/p" | head -1)"
        [ "$got" = "$(want_px "$p")" ] || _f "pixel $p is ${got:-missing}, expected $(want_px "$p")"
    done
    n="$(printf '%s\n' "$P" | sed -n 's/^count=\([0-9]*\)$/\1/p' | head -1)"
    [ -n "$n" ] && [ "$n" -ge 300 ] || _f "only ${n:-no} pixels of shell text inside the Terminal window"
    # the whole screen (section 13)
    local ui dw dh sc size
    ui="$(printf '%s\n' "$S" | sed -n 's/^screen: the desktop is \([0-9]*\)x\([0-9]*\) logical pixels at UI scale \([0-9]\)x.*$/\1 \2 \3/p' | tail -1)"
    if [ -z "$ui" ]; then
        _f "screen ui did not name the desktop's size and scale"
    else
        read -r dw dh sc <<< "$ui"
        [ "$dw" = "$DW" ] && [ "$dh" = "$DH" ] && [ "$sc" = "$SC" ] ||
            _f "the desktop is ${dw}x${dh} at ${sc}x; this screen calls for ${DW}x${DH} at ${SC}x"
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

# a green run's serial log and pixels for the screen the environment asks for (selftest)
green_log() {
    local DW DH SC FX FY TX TY SX SY
    layout
    printf '%s\n' "afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> [desk] open terminal
[desk] open files
[desk] moved files to $((FX + 60)),$((FY + 40))
[desk] focus terminal
[desk] launcher open
[desk] launcher closed
[desk] minimise terminal
[desk] close files
[desk] focus terminal
[desk] launcher open
[desk] circle: terminal
[desk] circle: settings
[desk] open settings
[desk] focus terminal
lsusb
usb: xHCI 1.0 at 0xfebf0000
  mouse: 2 dock click(s) on the desktop
afrisovera> desk list
[desk] pointer profile, on screen; 14 frame(s) drawn; 2 dock click(s); 1 notice(s); 0 event(s) dropped
[desk] window terminal at $TX,$TY size 410x260 focused
[desk] window settings at $SX,$SY size 380x300
[desk] front to back: terminal settings
afrisovera> screen ui
screen: the desktop is ${DW}x${DH} logical pixels at UI scale ${SC}x; type: screen ui 1, 2, 3, 4 or auto
afrisovera> desk perf
[desk] perf: ${DW}x${DH} at UI scale ${SC}x; draw 2210 us (worst 4120 us) over 31 frame(s); copy to the screen 180 us (worst 2950 us) over 40 copies"
}
green_px() {
    local DW DH SC FX FY TX TY SX SY p
    layout
    for p in $POINTS cloth lintel tbody; do echo "$p=$(want_px "$p")"; done
    echo "count=812"
    if [ -n "${PC_DESK_CAPPED:-}" ]; then echo "size=${PC_DESK_SCREEN:-1280x800}"; echo "band=0A0E14"
    else echo "size=$((DW * SC))x$((DH * SC))"; fi
    echo "spinefoot=100C0A"
    echo "corner=0B0908"
}

selftest() {
    local PC_DESK_SCREEN="" PC_DESK_CAPPED="" PC_DESK_UI="" # the arms choose their own screen
    local D pass=0 bad=0 G GPX
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
    G="$(green_log)" GPX="$(green_px)"
    arm green green "$G" "$GPX"
    arm crlf green "$(printf '%s\n' "$G" | sed 's/$/\r/')" "$GPX"
    arm no-desktop red "$(printf '%s\n' "$G" | grep -v '^\[gui\]')" "$GPX"
    arm click-missed red "$(printf '%s\n' "$G" | sed 's/open terminal$/nothing/')" "$GPX"
    arm drag-wrong red "$(printf '%s\n' "$G" | sed 's/^\[desk\] moved files to \([0-9]*\),/[desk] moved files to 1\1,/')" "$GPX"
    arm no-alt-tab red "$(printf '%s\n' "$G" | sed '0,/^\[desk\] focus terminal$/{/^\[desk\] focus terminal$/d}')" "$GPX"
    arm super-down-opened-circle red "$(printf '%s\n' "$G" | sed 's/^\[desk\] minimise terminal$/[desk] launcher open\n[desk] minimise terminal/')" "$GPX"
    arm alt-f4-ignored red "$(printf '%s\n' "$G" | grep -v 'close files' | sed 's/^\[desk\] window terminal/[desk] window files at 1,1 size 372x250\n[desk] window terminal/')" "$GPX"
    arm dock-count red "$(printf '%s\n' "$G" | sed 's/2 dock click/3 dock click/g')" "$GPX"
    arm not-restored red "$(printf '%s\n' "$G" | sed 's/size 410x260 focused/size 410x260 minimised/')" "$GPX"
    arm walk-stuck red "$(printf '%s\n' "$G" | grep -v '^\[desk\] circle: settings$')" "$GPX"
    arm enter-ignored red "$(printf '%s\n' "$G" | grep -v '^\[desk\] open settings$' | grep -v '^\[desk\] window settings')" "$GPX"
    arm super-2-ignored red "$(printf '%s\n' "$G" | sed 's/^\[desk\] front to back: terminal settings$/[desk] front to back: settings terminal/')" "$GPX"
    arm window-not-at-home red "$(printf '%s\n' "$G" | sed 's/^\[desk\] window terminal at [0-9]*,/[desk] window terminal at 210,/')" "$GPX"
    arm no-focus-line red "$G" "$(printf '%s\n' "$GPX" | sed 's/^cloth=.*/cloth=1C1613/')"
    arm empty-terminal red "$G" "$(printf '%s\n' "$GPX" | sed 's/^count=.*/count=0/')"
    arm console-not-desktop red "$G" "$(printf '%s\n' "$GPX" | sed 's/=[0-9A-F]\{6\}$/=080B12/')"
    arm no-screenshot red "$G" "error=not-a-png"
    arm empty red "" ""
    arm boxed-640x480 red "$(printf '%s\n' "$G" | sed 's/is 1280x800 logical/is 640x480 logical/')" "$GPX"
    arm frame-around red "$G" "$(printf '%s\n' "$GPX" | sed 's/^size=.*/size=1920x1080/')"
    arm spine-short red "$G" "$(printf '%s\n' "$GPX" | sed 's/^spinefoot=.*/spinefoot=0B0908/')"
    arm corner-is-frame red "$G" "$(printf '%s\n' "$GPX" | sed 's/^corner=.*/corner=0A0E14/')"
    arm no-screen-ui red "$(printf '%s\n' "$G" | grep -v '^screen: the desktop')" "$GPX"
    arm no-perf red "$(printf '%s\n' "$G" | grep -v '^\[desk\] perf')" "$GPX"
    PC_DESK_SCREEN=1920x1080 arm wrong-screen-size red "$G" "$GPX"
    # the other screens the wrapper suites ask for: their own green logs pass
    G1="$(PC_DESK_SCREEN=1920x1080 green_log)" P1="$(PC_DESK_SCREEN=1920x1080 green_px)"
    PC_DESK_SCREEN=1920x1080 arm green-1920x1080 green "$G1" "$P1"
    G2="$(PC_DESK_SCREEN=1360x768 green_log)" P2="$(PC_DESK_SCREEN=1360x768 green_px)"
    PC_DESK_SCREEN=1360x768 arm green-1360x768 green "$G2" "$P2"
    local BIG BIGPX
    BIG="$(PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 green_log)"
    BIGPX="$(PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 green_px)"
    PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 arm capped-1920x1200 green "$BIG" "$BIGPX"
    PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 arm drew-past-the-buffer red "$BIG" "$(printf '%s\n' "$BIGPX" | sed 's/^band=.*/band=0B0908/')"
    PC_DESK_SCREEN=2560x1440 PC_DESK_UI=1 PC_DESK_CAPPED=1 arm not-cut red "$(printf '%s\n' "$BIG" | sed 's/is 1920x1200 logical/is 2560x1440 logical/')" "$BIGPX"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 31 ]
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

layout
W="$(mktemp -d "$T/pc-desk.XXXXXX")"
# PC_DESK_SCREEN=WxH: the firmware is offered that screen mode through QEMU's VGA (EDID)
VGA=()
[ -n "${PC_DESK_SCREEN:-}" ] && VGA=(-vga none -device "VGA,xres=${PC_DESK_SCREEN%x*},yres=${PC_DESK_SCREEN#*x}")
qemu-system-aarch64 -M virt -m 256M -display none "${VGA[@]}" \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 -device usb-mouse,bus=xhci.0 \
    -serial "tcp:127.0.0.1:4942,server,nowait" -qmp "tcp:127.0.0.1:4943,server,nowait" &
QPID=$!
# PC_DESK_UI=N: `screen ui N` right after gui, so the whole walk runs at that scale
UI=()
[ -n "${PC_DESK_UI:-}" ] && UI=("type:screen ui $PC_DESK_UI")
# from the Files tile (20,68) to the middle of Files' lintel, 140 px in and 14 px down
python3 scripts/lib/usb-steps.py 4942 4943 "$W/serial.log" "$(winpath "$W/shot.png")" \
    "type:gui" "${UI[@]}" "move:-300,-132" down up "wait:1" "move:0,-40" down up "wait:1" \
    "move:$((FX + 140 - 20)),$((FY + 14 - 68))" down "move:60,40" up "wait:1" \
    "keys:alt+tab" "keys:meta_l" "keys:esc" "keys:meta_l+down" "keys:alt+f4" "keys:alt+tab" "wait:1" \
    "keys:meta_l" "keys:right" "keys:right" "keys:ret" "wait:1" "keys:meta_l+2" "wait:1" \
    "type:lsusb" "type:desk list" "type:screen ui" "type:desk perf" "wait:2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
{
    python3 scripts/lib/png-px.py "$W/shot.png" --desk "$SC" $POINTS size
    python3 scripts/lib/png-px.py "$W/shot.png" --desk "$SC" "$((TX + 98)),$TY" "$((TX + 98)),$((TY + 7))" \
        "$((TX + 385)),$((TY + 245))" "count:$((TX + 2)),$((TY + 28)),400,228,D8CEBE" \
        "4,$((DH - 150))" "$((DW - 6)),$((DH - 6))" |
        sed "1s/^[^=]*=/cloth=/; 2s/^[^=]*=/lintel=/; 3s/^[^=]*=/tbody=/; 4s/^[^=]*=/count=/; 5s/^[^=]*=/spinefoot=/; 6s/^[^=]*=/corner=/"
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
