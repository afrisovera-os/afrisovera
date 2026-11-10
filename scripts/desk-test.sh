#!/usr/bin/env bash
# The AfriSovera desktop on the ARM (phone) kernel (8 Nov 2026, docs/design/desktop.md).
#
# Boots the ARM kernel with ramfb, types `gui` and a script of `desk` commands over the serial
# line (the same hit-testing a touch or a mouse reaches), then grades:
#   - touch profile (the phone's default): an app opens maximised under the top bar
#     (0,25 640x399) and its large close button closes it;
#   - pointer profile (the phone docked to a screen): Files, About and Terminal open; a tap on
#     a title bar focuses; a title-bar drag moves the window by exactly the drag; a tap on a
#     close button closes; a dock tap on the focused app minimises it and a second restores
#     it; alt-tab cycles focus; the launcher opens from the dock and its Settings tile opens
#     Settings; the Settings clock switch flips both ways; a notification is posted;
#   - `desk list`: every window's place and size, the focus, the stacking order, 3 dock clicks;
#   - the PIXELS of the screenshot: the top bar, its gold hairline and kente band, the dock,
#     the focused window's gold line, an unfocused title bar and the Terminal's body.
#   scripts/desk-test.sh              # grader selftest, then the QEMU boot
#   scripts/desk-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# framebuffer points graded on the screenshot, and the colour each must be (desk.rs palette)
POINTS="5,5 320,22 4,23 20,23 36,23 198,446 300,60 540,120 595,365"
want_px() { # want_px <point> -> RRGGBB
    case "$1" in
        5,5) echo 0B0B0D ;;      # the top bar (night)
        320,22) echo 5C4A1E ;;   # the gold hairline under it
        4,23) echo C99E44 ;;     # kente band: gold
        20,23) echo 1B7A3D ;;    #             green
        36,23) echo C1121F ;;    #             red
        198,446) echo 141519 ;;  # the dock
        300,60) echo F4CB57 ;;   # Settings (focused): the gold line along its title bar
        540,120) echo 18191E ;;  # Terminal (unfocused): its title bar
        595,365) echo 0C0C0E ;;  # Terminal's body
    esac
}

grade() { # grade <serial> <pixels>
    local fail=0 S P p got
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$2" 2> /dev/null)"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    # each step's line, in order: the serial log after `gui` must hold them as a subsequence
    local steps=(
        '\[gui\] Tonga desktop rendered'
        '\[desk\] open about'
        '\[desk\] window about at 0,27 size 640x397 focused'
        '\[desk\] close about'
        '\[desk\] input profile: pointer'
        '\[desk\] open files'
        '\[desk\] open about'
        '\[desk\] open terminal'
        '\[desk\] focus files'
        '\[desk\] moved files to 76,74'
        '\[desk\] close about'
        '\[desk\] minimise files'
        '\[desk\] focus files'
        '\[desk\] focus terminal'
        '\[desk\] launcher open'
        '\[desk\] open settings'
        '\[desk\] settings: 24-hour clock off'
        '\[desk\] settings: 24-hour clock on'
        '\[desk\] notified: Tests running'
        '\[desk\] pointer profile, on screen; [0-9]+ frame\(s\) drawn; 3 dock click\(s\); 2 notice\(s\); 0 event\(s\) dropped'
        '\[desk\] window files at 76,74 size 372x250$'
        '\[desk\] window terminal at 190,112 size 410x260$'
        '\[desk\] window settings at 110,60 size 380x300 focused$'
        '\[desk\] front to back: settings terminal files$'
    )
    # one awk pass: the patterns must match in order (each after the line the one before it matched)
    # (patterns go through ENVIRON, which awk does not escape-process, unlike -v)
    p="$(printf '%s\n' "$S" | DESK_PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    if has '\[desk\] moved files to 36,44'; then _f "a tap without a move was logged as a move"; fi
    for p in $POINTS; do
        got="$(printf '%s\n' "$P" | sed -n "s/^$p=\([0-9A-F]\{6\}\)$/\1/p" | head -1)"
        [ "$got" = "$(want_px "$p")" ] || _f "pixel $p is ${got:-missing}, expected $(want_px "$p")"
    done
    if has 'KERNEL PANIC|SYNC EXCEPTION'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk open about
[desk] open about
afrisovera> desk list
[desk] touch profile, on screen; 2 frame(s) drawn; 0 dock click(s); 1 notice(s); 0 event(s) dropped
[desk] window about at 0,27 size 640x397 focused
[desk] front to back: about
afrisovera> desk tap 620 38
[desk] close about
afrisovera> desk mode pointer
[desk] input profile: pointer
afrisovera> desk open files
[desk] open files
afrisovera> desk open about
[desk] open about
afrisovera> desk open terminal
[desk] open terminal
afrisovera> desk tap 150 50
[desk] focus files
afrisovera> desk drag 150 50 40 30
[desk] moved files to 76,74
afrisovera> desk tap 557 80
[desk] close about
afrisovera> desk tap 281 444
[desk] minimise files
afrisovera> desk tap 281 444
[desk] focus files
afrisovera> desk key alt-tab
[desk] focus terminal
afrisovera> desk tap 221 444
[desk] launcher open
afrisovera> desk tap 361 270
[desk] open settings
afrisovera> desk tap 300 205
[desk] settings: 24-hour clock off
afrisovera> desk tap 300 205
[desk] settings: 24-hour clock on
afrisovera> desk notify Tests running
[desk] notified: Tests running
afrisovera> desk list
[desk] pointer profile, on screen; 17 frame(s) drawn; 3 dock click(s); 2 notice(s); 0 event(s) dropped
[desk] window files at 76,74 size 372x250
[desk] window terminal at 190,112 size 410x260
[desk] window settings at 110,60 size 380x300 focused
[desk] front to back: settings terminal files"
GPX="$(for p in $POINTS; do echo "$p=$(want_px "$p")"; done)"

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
    arm touch-not-maximised red "$(printf '%s\n' "$GREEN" | sed 's/about at 0,27 size 640x397/about at 240,70 size 330x300/')" "$GPX"
    arm drag-off-by-one red "$(printf '%s\n' "$GREEN" | sed 's/moved files to 76,74/moved files to 77,74/')" "$GPX"
    arm tap-logged-as-move red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] focus files$/[desk] focus files\n[desk] moved files to 36,44/')" "$GPX"
    arm no-minimise red "$(printf '%s\n' "$GREEN" | grep -v 'minimise files')" "$GPX"
    arm out-of-order red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] focus terminal$/XX/; s/^\[desk\] open terminal$/[desk] focus terminal/')" "$GPX"
    arm wrong-stack red "$(printf '%s\n' "$GREEN" | sed 's/front to back: settings terminal files/front to back: settings files terminal/')" "$GPX"
    arm wrong-focus red "$(printf '%s\n' "$GREEN" | sed 's/size 380x300 focused/size 380x300/')" "$GPX"
    arm dock-count red "$(printf '%s\n' "$GREEN" | sed 's/3 dock click/2 dock click/')" "$GPX"
    arm switch-stuck red "$(printf '%s\n' "$GREEN" | grep -v 'clock on')" "$GPX"
    arm no-gold-focus red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^300,60=.*/300,60=22232A/')"
    arm no-kente red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/^20,23=.*/20,23=0B0B0D/')"
    arm console-not-desktop red "$GREEN" "$(printf '%s\n' "$GPX" | sed 's/=.*/=080B12/')"
    arm no-screenshot red "$GREEN" "error=not-a-png"
    arm panic red "$GREEN
KERNEL PANIC at desk.rs" "$GPX"
    arm empty red "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== the desktop on the ARM kernel (touch, pointer, windows, launcher, apps) =="
if ! selftest; then echo "DESK TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK SELFTEST ONLY -- no measurement taken"; exit 0; }

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
    "gui" "desk open about" "desk list" "desk tap 620 38" "desk mode pointer" \
    "desk open files" "desk open about" "desk open terminal" "desk tap 150 50" "desk drag 150 50 40 30" \
    "desk tap 557 80" "desk tap 281 444" "desk tap 281 444" "desk key alt-tab" "desk tap 221 444" \
    "desk tap 361 270" "desk tap 300 205" "desk tap 300 205" "desk notify Tests running" "desk list" \
    > "$W/serial.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
python3 scripts/lib/png-px.py "$W/screen.png" $POINTS > "$W/pixels.txt" 2>&1
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
