#!/usr/bin/env bash
# Motion (11 Nov 2026, AfriUI slice 6, docs/design/afriui.md sections 7 and 17): a window unfolds from
# its lintel in 3 frames, the Circle grows from its Spine button, 2 ticks (20 ms) a frame, from idle();
# a Settings row and `desk motion on|off` switch it; `desk motion hold` (test only) stops a motion at a
# frame until the next desk command or input. The PC kernel at 640x480 (UI scale 1, the phone's size),
# two boots over COM1:
#   boot 1, the walk: motion is on by default; `desk open about` draws frames 1, 2 and 3, each at least
#     2 ticks after the one before (a late frame may be skipped, never queued); a held Files is finished
#     by the next command (`desk verify`); a tap on the Settings tile while the Circle is held at frame 1
#     opens Settings (input uses the final geometry), and the Circle's motion ends early (one motion at a
#     time); the Settings row turns motion off, after which an open runs no motion; `desk motion on`;
#     desk verify shows 0 rows after every case; desk perf names the slowest motion frame.
#   boot 2, the picture: About open, then Files held at frame 1 over it. Below Files' first third the
#     screen shows About's body (a frame repaints what is under it), above it Files' body, and left of
#     About the wallpaper, not Files.
#   scripts/pc-deskmotion-test.sh              # grader selftest, then the QEMU boots
#   scripts/pc-deskmotion-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

# a motion's ticks "A B C": frame 1 at A, frame k at least 2(k-1) ticks after it, frame 3 drawn,
# frame 2 drawn after frame 1 or skipped (0)
ticks_ok() {
    local a=$1 b=$2 c=$3
    [ "$a" -gt 0 ] && [ "$c" -ge $((a + 4)) ] || return 1
    [ "$b" -eq 0 ] || { [ "$b" -ge $((a + 2)) ] && [ "$c" -gt "$b" ]; }
}

grade() { # grade <serial 1> <serial 2> <pixels 1> <pixels 2>
    local fail=0 S S2 P P2 got m
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    S2="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    P="$(tr -d '\r' < "$3" 2> /dev/null)"
    P2="$(tr -d '\r' < "$4" 2> /dev/null)"
    local steps=(
        '^screen: the desktop is 640x480 logical pixels at UI scale 1x'
        '^\[desk\] motion on; 0 motion\(s\) run$'
        '\[desk\] open about$'
        '^\[desk\] motion on; 1 motion\(s\) run$'
        '^\[desk\] motion last: unfold about ticks [0-9]+ [0-9]+ [0-9]+$'
        '\[desk\] verify: 0 of 480 rows differ'
        '\[desk\] open files$'
        '^\[desk\] motion: unfold window held at 1/3$'
        '\[desk\] verify: 0 of 480 rows differ'
        '\[desk\] launcher open$'
        '^\[desk\] motion: grow circle held at 1/3$'
        '\[desk\] open settings$'
        '^\[desk\] motion on; 4 motion\(s\) run$'
        '^\[desk\] motion last: unfold settings ticks [0-9]+ [0-9]+ [0-9]+$'
        '^\[desk\] motion before: grow circle ticks [0-9]+ 0 0 early$'
        '\[desk\] verify: 0 of 480 rows differ'
        '\[desk\] settings: motion off$'
        '\[desk\] close about$'
        '\[desk\] open about$'
        '^\[desk\] motion off; 4 motion\(s\) run$'
        '\[desk\] motion on$'
        '\[desk\] verify: 0 of 480 rows differ'
        '^\[desk\] perf: the slowest motion frame [0-9]+ us over [1-9][0-9]* motion frame\(s\)$'
    )
    local p
    p="$(printf '%s\n' "$S" | PATS="$(printf '%s\n' "${steps[@]}")" awk '
        BEGIN { n = split(ENVIRON["PATS"], P, "\n"); if (P[n] == "") n--; k = 1 }
        k <= n && $0 ~ P[k] { k++ }
        END { if (k <= n) printf "%d %s", k, P[k] }')"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)"
    # the frame clock: each frame at least 2 ticks after the one before, frame 3 drawn
    for m in about settings; do
        got="$(printf '%s\n' "$S" | sed -n "s/^\[desk\] motion last: unfold $m ticks \([0-9]*\) \([0-9]*\) \([0-9]*\)$/\1 \2 \3/p" | head -1)"
        if [ -n "$got" ]; then
            # shellcheck disable=SC2086
            ticks_ok $got || _f "unfold $m drew its frames at ticks $got (frame k must come at least 2(k-1) ticks after frame 1)"
        fi
    done
    # the walk ends in one place: the Spine's night
    got="$(printf '%s\n' "$P" | sed -n 's/^20,250=//p' | head -1)"
    [ "$got" = 100C0A ] || _f "the desktop's Spine is ${got:-missing} after the walk, not its night #100C0A"
    # boot 2: Files held at frame 1 over About
    printf '%s\n' "$S2" | grep -aqE '^\[desk\] motion: unfold window held at 1/3$' || _f "boot 2: Files was not held at frame 1"
    got="$(printf '%s\n' "$P2" | sed -n 's/^300,262=//p' | head -1)"
    [ "$got" = 0C0907 ] || _f "below Files' first third (300,262) the screen is ${got:-missing}, not About's body #0C0907 (frame 1 must repaint what lies under it)"
    got="$(printf '%s\n' "$P2" | sed -n 's/^300,120=//p' | head -1)"
    [ "$got" = 1E1814 ] || _f "in Files' first third (300,120) the screen is ${got:-missing}, not Files' body #1E1814"
    got="$(printf '%s\n' "$P2" | sed -n 's/^150,262=//p' | head -1)"
    { [ -n "$got" ] && [ "$got" != 1E1814 ]; } || _f "below Files' first third, left of About (150,262), the screen is ${got:-missing}: Files' body was drawn below its clip"
    if printf '%s\n%s\n' "$S" "$S2" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="afrisovera> screen ui
screen: the desktop is 640x480 logical pixels at UI scale 1x; type: screen ui 1, 2, 3, 4 or auto
afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk motion
[desk] motion on; 0 motion(s) run
afrisovera> desk open about
[desk] open about
afrisovera> desk motion
[desk] motion on; 1 motion(s) run
[desk] motion last: unfold about ticks 1200 1202 1204
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (4 partial frame(s) drawn before it)
afrisovera> desk motion hold 1 files
[desk] open files
[desk] motion: unfold window held at 1/3
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (6 partial frame(s) drawn before it)
afrisovera> desk motion hold 1 circle
[desk] launcher open
[desk] motion: grow circle held at 1/3
afrisovera> desk tap 342 305
[desk] open settings
afrisovera> desk motion
[desk] motion on; 4 motion(s) run
[desk] motion last: unfold settings ticks 1500 0 1506
[desk] motion before: grow circle ticks 1460 0 0 early
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (9 partial frame(s) drawn before it)
afrisovera> desk tap 425 230
[desk] settings: motion off
afrisovera> desk close about
[desk] close about
afrisovera> desk open about
[desk] open about
afrisovera> desk motion
[desk] motion off; 4 motion(s) run
[desk] motion last: unfold settings ticks 1500 0 1506
[desk] motion before: grow circle ticks 1460 0 0 early
afrisovera> desk motion on
[desk] motion on
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (12 partial frame(s) drawn before it)
afrisovera> desk perf
[desk] perf: the slowest motion frame 2400 us over 4 motion frame(s)"
GREEN2="afrisovera> gui
[gui] Tonga desktop rendered; type 'clear' to return to console
afrisovera> desk open about
[desk] open about
afrisovera> desk motion hold 1 files
[desk] open files
[desk] motion: unfold window held at 1/3"
GPX="20,250=100C0A"
GPX2="300,262=0C0907
300,120=1E1814
150,262=2E2318"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-deskmotion-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial 1> <serial 2> <pixels 1> <pixels 2>
        local got
        printf '%s\n' "$3" > "$D/s"
        printf '%s\n' "$4" > "$D/s2"
        printf '%s\n' "$5" > "$D/p"
        printf '%s\n' "$6" > "$D/p2"
        if grade "$D/s" "$D/s2" "$D/p" "$D/p2" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GREEN2" "$GPX" "$GPX2"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GREEN2" "$GPX" "$GPX2"
    arm off-by-default red "$(printf '%s\n' "$GREEN" | sed '0,/motion on; 0 motion/s//motion off; 0 motion/')" "$GREEN2" "$GPX" "$GPX2"
    arm frames-too-fast red "$(printf '%s\n' "$GREEN" | sed 's/unfold about ticks 1200 1202 1204/unfold about ticks 1200 1201 1204/')" "$GREEN2" "$GPX" "$GPX2"
    arm no-frame-3 red "$(printf '%s\n' "$GREEN" | sed 's/unfold about ticks 1200 1202 1204/unfold about ticks 1200 1202 0/')" "$GREEN2" "$GPX" "$GPX2"
    arm tap-missed red "$(printf '%s\n' "$GREEN" | grep -v 'open settings')" "$GREEN2" "$GPX" "$GPX2"
    arm two-at-once red "$(printf '%s\n' "$GREEN" | sed 's/grow circle ticks 1460 0 0 early/grow circle ticks 1460 1462 1464/')" "$GREEN2" "$GPX" "$GPX2"
    arm off-still-moves red "$(printf '%s\n' "$GREEN" | sed 's/motion off; 4 motion/motion off; 5 motion/')" "$GREEN2" "$GPX" "$GPX2"
    arm stale-rows red "$(printf '%s\n' "$GREEN" | sed '0,/verify: 0 of/s//verify: 7 of/')" "$GREEN2" "$GPX" "$GPX2"
    arm no-perf red "$(printf '%s\n' "$GREEN" | grep -v 'slowest motion frame')" "$GREEN2" "$GPX" "$GPX2"
    arm not-held red "$GREEN" "$(printf '%s\n' "$GREEN2" | grep -v 'held at')" "$GPX" "$GPX2"
    arm under-not-repainted red "$GREEN" "$GREEN2" "$GPX" "$(printf '%s\n' "$GPX2" | sed 's/^300,262=.*/300,262=1E1814/')"
    arm drawn-below-clip red "$GREEN" "$GREEN2" "$GPX" "$(printf '%s\n' "$GPX2" | sed 's/^150,262=.*/150,262=1E1814/')"
    arm empty red "" "" "" ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

echo "== motion: a window unfolds, the Circle grows, the off switch, a hold =="
if ! selftest; then echo "PC DESKMOTION TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC DESKMOTION SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC DESKMOTION TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC DESKMOTION TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

W="$(mktemp -d "$T/pc-deskmotion.XXXXXX")"
boot() { # boot <serial port> <qmp port> <log> <png> <command>...
    local sp=$1 qp=$2 log=$3 png=$4 qpid
    shift 4
    qemu-system-aarch64 -M virt -m 256M -display none -vga none -device VGA,xres=640,yres=480 \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$qp,server,nowait" &
    qpid=$!
    AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py "$sp" "$qp" "$(winpath "$png")" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
boot 5976 5977 "$W/serial1.log" "$W/shot1.png" \
    "screen ui" "gui" "desk motion" "desk open about" "desk motion" "desk verify" \
    "desk motion hold 1 files" "desk verify" "desk motion hold 1 circle" "desk tap 342 305" "desk motion" \
    "desk verify" "desk tap 425 230" "desk close about" "desk open about" "desk motion" "desk motion on" \
    "desk verify" "desk perf"
boot 5978 5979 "$W/serial2.log" "$W/shot2.png" "gui" "desk open about" "desk motion hold 1 files"
python3 scripts/lib/png-px.py "$W/shot1.png" --desk 1 20,250 > "$W/pixels1.txt" 2>&1
python3 scripts/lib/png-px.py "$W/shot2.png" --desk 1 300,262 300,120 150,262 > "$W/pixels2.txt" 2>&1
for n in 1 2; do
    tr -d '\r' < "$W/serial$n.log" | grep -aE '^\[gui\]|\[desk\]|^screen: the desktop' | sed "s/^/  | boot $n: /"
    sed "s/^/  px boot $n: /" "$W/pixels$n.txt"
done
[ -n "${DESK_SHOT:-}" ] && cp "$W/shot2.png" "$DESK_SHOT"

echo "  --- grading ---"
if grade "$W/serial1.log" "$W/serial2.log" "$W/pixels1.txt" "$W/pixels2.txt"; then
    rm -rf "$W"
    echo "PC DESKMOTION TEST PASSED"
    exit 0
fi
echo "  logs and screenshots kept in $W"
echo "PC DESKMOTION TEST FAILED"
exit 1
