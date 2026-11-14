#!/usr/bin/env bash
# The AfriSovera desktop on the ARM (phone) kernel.
#
# Since 11 Nov 2026 (step P2 of docs/design/afriui-phone-plan.md) the phone builds the Sovereign
# UI, the same desktop as the PC (kernel/src/afriui/, docs/design/afriui.md): the Spine down the
# left edge, cut corners, each app's woven cloth on the focused window's lintel, the Circle. It
# draws straight into the 640x480 ramfb framebuffer (kernel/src/afriui/arm_canvas.rs). The 8 Nov
# desktop (kernel/src/desk.rs) and its top bar and dock are gone; this suite keeps every
# behaviour the old one graded, at the new places, and adds the keyboard walk and desk verify.
#
# Boots the ARM kernel with ramfb, types `gui` and a script of `desk` commands over the serial
# line (the same hit-testing a touch or a mouse reaches), then grades:
#   - touch profile (the phone's default): an app opens maximised right of the 56 px Spine
#     (56,0 584x480) and its 44 px close button closes it;
#   - pointer profile (the phone docked to a screen): Files, About and Terminal open at their
#     homes round the middle; a tap on a lintel focuses; a lintel drag moves the window by
#     exactly the drag (a tap alone moves nothing); a tap on a close button closes; a Spine tap
#     on the focused app minimises it and a second restores it; alt-tab cycles focus; the
#     Circle opens from the Spine's top tile and its Settings tile opens Settings; the Settings
#     clock switch flips both ways; the keyboard walk (Super, Right, Enter) focuses Terminal; a
#     notification is posted;
#   - `desk list`: every window's place and size, the focus, the stacking order, 3 Spine clicks;
#   - `desk verify`: the partial frames left exactly what a full redraw draws (0 of 480 rows);
#   - the PIXELS of the screenshot, by role: the Spine's night and its woven edge, Terminal's
#     cloth (malachite) on its lintel and the lintel under it, its body, the shell's text
#     inside it, and the wallpaper in the far corner.
#   scripts/desk-test.sh              # grader selftest, then the QEMU boot
#   scripts/desk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# The places at 640x480, as kernel/src/afriui/desk.rs works them out (home, win_rect,
# title_buttons, dock_tile, launcher_tile):
#   touch: the window 56,0 584x480; its close button 592,5 44x27 (tapped at 614,18)
#   pointer Spine (44 px): the Circle button 4,6; Files' tile 4,52; tiles every 40 px
#   pointer homes: Files 48,45 372x250; Terminal 177,160 410x260; Settings 82,60 380x300;
#     About 287,70 330x300; a lintel is 26 px, its close button 26 px in from the right edge
#   the Circle (pointer): 192,62 300x300; Settings (the lower right tile of five, 13 Nov: section 32) at 400,286
#   Settings' body starts under its lintel (y 86); the clock row is 134 to 154 below that (the language row came first, 13 Nov)
TX=177 TY=160
POINTS="20,250 40,4 40,20 42,8 $((TX + 98)),$TY $((TX + 98)),$((TY + 7)) $((TX + 385)),$((TY + 245)) 636,476"
want_px() { # want_px <point> -> RRGGBB
    case "$1" in
        20,250) echo 100C0A ;;   # the Spine (night)
        40,4) echo C8963A ;;     # its woven edge: gold-deep
        40,20) echo 2E9E6A ;;    #                 malachite
        42,8) echo C1121F ;;     #                 red (second column, offset half a block)
        275,160) echo 2E9E6A ;;  # Terminal (focused): its cloth's malachite, 98 px in on its top row
        275,167) echo 2A211B ;;  # Terminal's lintel under the cloth
        562,405) echo 0E0B09 ;;  # Terminal's body, near its bottom right
    esac
}

grade() { # grade <serial> <pixels>
    local fail=0 S P p got n
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    has() { grep -aqE "$1" <<< "$S"; }
    # each step's line, in order: the serial log after `gui` must hold them as a subsequence
    local steps=(
        '^\[gui\] Tonga desktop rendered'
        '\[desk\] open about$'
        '\[desk\] window about at 56,0 size 584x480 focused$'
        '\[desk\] close about$'
        '\[desk\] input profile: pointer$'
        '\[desk\] open files$'
        '\[desk\] open about$'
        '\[desk\] open terminal$'
        '\[desk\] focus files$'
        '\[desk\] moved files to 88,75$'
        '\[desk\] close about$'
        '\[desk\] minimise files$'
        '\[desk\] focus files$'
        '\[desk\] focus terminal$'
        '\[desk\] launcher open$'
        '\[desk\] open settings$'
        '\[desk\] settings: 24-hour clock off$'
        '\[desk\] settings: 24-hour clock on$'
        '\[desk\] launcher open$'
        '\[desk\] circle: terminal$'
        '\[desk\] focus terminal$'
        '\[desk\] notified: Tests running$'
        '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 3 dock click\(s\); 2 notice\(s\); 0 event\(s\) dropped$'
        '\[desk\] window files at 88,75 size 372x250$'
        "\\[desk\\] window terminal at $TX,$TY size 410x260 focused\$"
        '\[desk\] window settings at 82,60 size 380x300$'
        '\[desk\] front to back: terminal settings files$'
        '^\[desk\] verify: 0 of 480 rows differ after a full redraw \([1-9][0-9]* partial frame'
    )
    # one awk pass: the patterns must match in order (each after the line the one before it matched)
    # (patterns go through ENVIRON, which awk does not escape-process, unlike -v)
    p="$(printf '%s\n' "$S" | DESK_PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    if has '\[desk\] moved files to 48,45'; then _f "a tap without a move was logged as a move"; fi
    # the last window list (pointer profile) must not hold About: its close button closed it
    if printf '%s\n' "$S" | sed -n '/pointer profile, on screen/,$p' | grep -aq '^\[desk\] window about'; then
        _f "About is still open after its close button"
    fi
    for p in $POINTS; do
        [ "$p" = 636,476 ] && continue
        got="$(printf '%s\n' "$P" | sed -n "s/^$p=\([0-9A-F]\{6\}\)$/\1/p" | head -1)"
        [ "$got" = "$(want_px "$p")" ] || _f "pixel $p is ${got:-missing}, expected $(want_px "$p")"
    done
    got="$(printf '%s\n' "$P" | sed -n 's/^636,476=\([0-9A-F]\{6\}\)$/\1/p' | head -1)"
    case "$got" in
        ""|000000|080B12) _f "the far corner is ${got:-missing}: not the desktop's wallpaper" ;;
    esac
    n="$(printf '%s\n' "$P" | sed -n 's/^count:[^=]*=\([0-9]*\)$/\1/p' | head -1)"
    [ -n "$n" ] && [ "$n" -ge 300 ] || _f "only ${n:-no} pixels of shell text inside the Terminal window"
    if has 'KERNEL PANIC|SYNC EXCEPTION'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk open about
[desk] open about
afrisovera> desk list
[desk] touch profile, on screen; 2 frame(s) drawn; 0 dock click(s); 1 notice(s); 0 event(s) dropped
[desk] window about at 56,0 size 584x480 focused
[desk] front to back: about
afrisovera> desk tap 614 18
[desk] close about
afrisovera> desk mode pointer
[desk] input profile: pointer
afrisovera> desk open files
[desk] open files
afrisovera> desk open about
[desk] open about
afrisovera> desk open terminal
[desk] open terminal
afrisovera> desk tap 150 55
[desk] focus files
afrisovera> desk drag 150 55 40 30
[desk] moved files to 88,75
afrisovera> desk tap 602 84
[desk] close about
afrisovera> desk tap 20 68
[desk] minimise files
afrisovera> desk tap 20 68
[desk] focus files
afrisovera> desk key alt-tab
[desk] focus terminal
afrisovera> desk tap 20 22
[desk] launcher open
afrisovera> desk tap 400 286
[desk] open settings
afrisovera> desk tap 200 226
[desk] settings: 24-hour clock off
afrisovera> desk tap 200 226
[desk] settings: 24-hour clock on
afrisovera> desk key super
[desk] launcher open
afrisovera> desk key right
[desk] circle: terminal
afrisovera> desk key enter
[desk] focus terminal
afrisovera> desk notify Tests running
[desk] notified: Tests running
afrisovera> desk list
[desk] pointer profile, on screen; 21 frame(s) drawn; 3 dock click(s); 2 notice(s); 0 event(s) dropped
[desk] window files at 88,75 size 372x250
[desk] window terminal at 177,160 size 410x260 focused
[desk] window settings at 82,60 size 380x300
[desk] front to back: terminal settings files
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (17 partial frame(s) drawn before it)"
GPX="$(for p in $POINTS; do [ "$p" = 636,476 ] && echo "$p=0B0908" || echo "$p=$(want_px "$p")"; done; echo "count:$((TX + 2)),$((TY + 28)),400,228,F2EADC=812")"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/desk-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expected green|red> <serial> <pixels>
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
    arm touch-not-maximised red "$(printf '%s\n' "$GREEN" | sed 's/about at 56,0 size 584x480/about at 287,70 size 330x300/')" "$GPX"
    arm touch-under-old-top-bar red "$(printf '%s\n' "$GREEN" | sed 's/about at 56,0 size 584x480/about at 0,27 size 640x397/')" "$GPX"
    arm drag-off-by-one red "$(printf '%s\n' "$GREEN" | sed 's/moved files to 88,75/moved files to 89,75/')" "$GPX"
    arm tap-logged-as-move red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] focus files$/[desk] focus files\n[desk] moved files to 48,45/')" "$GPX"
    arm no-minimise red "$(printf '%s\n' "$GREEN" | grep -v 'minimise files')" "$GPX"
    arm out-of-order red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] notified: Tests running$/XX/; s/^\[desk\] open settings$/[desk] notified: Tests running/')" "$GPX"
    arm wrong-stack red "$(printf '%s\n' "$GREEN" | sed 's/front to back: terminal settings files/front to back: settings terminal files/')" "$GPX"
    arm wrong-focus red "$(printf '%s\n' "$GREEN" | sed 's/size 410x260 focused/size 410x260/')" "$GPX"
    arm dock-count red "$(printf '%s\n' "$GREEN" | sed 's/3 dock click/2 dock click/')" "$GPX"
    arm switch-stuck red "$(printf '%s\n' "$GREEN" | grep -v 'clock on')" "$GPX"
    arm walk-stuck red "$(printf '%s\n' "$GREEN" | grep -v '^\[desk\] circle: terminal$')" "$GPX"
    arm about-left-open red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] window settings at/[desk] window about at 287,70 size 330x300\n[desk] window settings at/')" "$GPX"
    arm stale-rows red "$(printf '%s\n' "$GREEN" | sed 's/verify: 0 of/verify: 12 of/')" "$GPX"
    arm no-verify red "$(printf '%s\n' "$GREEN" | grep -v '^\[desk\] verify')" "$GPX"
    arm no-cloth red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^275,160=.*/275,160=1C1613/')"
    arm no-weave red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^40,20=.*/40,20=100C0A/')"
    arm empty-terminal red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^\(count:[^=]*\)=.*/\1=0/')"
    arm corner-is-console red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^636,476=.*/636,476=080B12/')"
    arm console-not-desktop red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/=[0-9A-F]\{6\}$/=080B12/')"
    arm no-screenshot red "$GREEN" "error=not-a-png"
    arm panic red "$GREEN
KERNEL PANIC at desk.rs" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 25 ]
}

echo "== the Sovereign desktop on the ARM kernel (touch, pointer, windows, the Circle, apps) =="
if ! selftest; then echo "DESK TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK SELFTEST ONLY: no measurement taken"; exit 0; }

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK TEST FAILED (harness: $BIN was not built)"; exit 1; }
W="$(mktemp -d "$T/desk.XXXXXX")"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4940,server,nowait \
    -qmp tcp:127.0.0.1:4941,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py 4940 4941 "$W/screen.png" \
    "gui" "desk open about" "desk list" "desk tap 614 18" "desk mode pointer" \
    "desk open files" "desk open about" "desk open terminal" "desk tap 150 55" "desk drag 150 55 40 30" \
    "desk tap 602 84" "desk tap 20 68" "desk tap 20 68" "desk key alt-tab" "desk tap 20 22" \
    "desk tap 400 286" "desk tap 200 226" "desk tap 200 226" "desk key super" "desk key right" "desk key enter" \
    "desk notify Tests running" "desk list" "desk verify" \
    > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
python3 scripts/lib/png-px.py "$W/screen.png" $POINTS "count:$((TX + 2)),$((TY + 28)),400,228,F2EADC" > "$W/pixels.txt" 2>&1
tr -d '\r' < "$W/serial.log" | grep -aE '^\[gui\]|\[desk\]' | sed 's/^/  | /'
sed 's/^/  px /' "$W/pixels.txt"
[ -n "${DESK_SHOT:-}" ] && cp "$W/screen.png" "$DESK_SHOT"
[ -n "${DESK_SHOT_DIR:-}" ] && cp "$W/screen.png" "$DESK_SHOT_DIR/desk.png"

echo "  --- grading ---"
if grade "$W/serial.log" "$W/pixels.txt"; then
    rm -rf "$W"
    echo "DESK TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK TEST FAILED"
exit 1
