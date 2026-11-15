#!/usr/bin/env bash
# Window controls that work (afriui.md section 33, the architect's verdict of 15 Nov), on both kernels.
#
# Every step goes through `desk press <app> <part> [dx dy]`, which presses a window's part where a person would,
# through the same press, move and release as the pointer, so the hit tests are tested; `desk verify` follows each
# step and must find 0 rows. One pass, at 1x on ARM (640x480) and on the PC at 1x and again at 2x (1920x1080):
#   maximise by the button, restore by it (the normal place comes back exactly), maximise by a double click on the
#   title, restore by Super+Down, maximise by Super+Up, drag the maximised title (it restores under the pointer);
#   minimise by Super+Down, bring it back by Alt+Tab; snap left by Super+Left, back by Super+Left, snap right by
#   Super+Right, restore by `desk restore`; snap left by dragging the title past the left edge;
#   resize by the grip (+50 +30), to the minimum (240x150), by the right edge (+40) and the bottom edge (+40).
# Then on the PC: maximise, change the scale (1.5, then 2x-blocks): still maximised, filling the space; restore.
# Then touch mode: the minimise button works there too.
#   scripts/desk-windows-test.sh              # grader selftest, then the two boots
#   scripts/desk-windows-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"

PASS=(
    "desk open files" "desk list"
    "desk press files max" "desk list" "~0.3" "!maximised" "desk verify"
    "desk press files max" "desk list" "desk verify"
    "desk press files title2" "desk key super-down" "desk key super-up" "desk verify"
    "desk press files title 120 60" "desk list" "desk verify"
    "desk key super-down" "~0.3" "!minimised" "desk verify" "desk key alt-tab" "desk list"
    "desk key super-left" "desk list" "~0.3" "!snapped-left" "desk verify" "desk key super-left" "desk key super-right" "desk verify"
    "desk restore files" "desk press files title -3000 0" "desk list" "desk verify"
    "desk restore files" "desk press files grip 50 30" "desk press files grip -1000 -1000" "desk list"
    "desk press files right 40 0" "desk press files bottom 0 40" "desk list" "desk verify"
    "desk close files"
)
# the patterns one pass leaves, in order (each after the line the one before matched)
STEPS=(
    '^\[desk\] open files$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$'
    '^\[desk\] maximise files$'
    '^\[desk\] window files at [0-9]+,[0-9]+ size [0-9]+x[0-9]+ maximised focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] maximise files$'
    '^\[desk\] restore files$'
    '^\[desk\] maximise files$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] moved files to '
    '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] minimise files$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] focus files$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$'
    '^\[desk\] snap left files$'
    '^\[desk\] window files at [0-9]+,[0-9]+ size [0-9]+x[0-9]+ snapped left focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] snap right files$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] snap left files$'
    '^\[desk\] window files at [0-9]+,[0-9]+ size [0-9]+x[0-9]+ snapped left focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] resized files to [0-9]+x[0-9]+$'
    '^\[desk\] resized files to 240x150$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size 240x150 focused$'
    '^\[desk\] resized files to 280x150$'
    '^\[desk\] resized files to 280x190$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size 280x190 focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] close files$'
)
PC_SCALE=("desk open files" "desk press files max" "screen ui 1.5" "desk list" "desk verify" "screen ui 2x-blocks"
          "desk list" "desk verify" "desk press files max" "desk list" "desk verify" "desk close files")
SCALE_STEPS=(
    '^\[desk\] maximise files$'
    '^screen: the desktop is now [0-9]+x[0-9]+ logical pixels at scale 1.50'
    '^\[desk\] window files at [0-9]+,0 size [0-9]+x[0-9]+ maximised focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] window files at [0-9]+,0 size [0-9]+x[0-9]+ maximised focused$'
    '^\[desk\] verify: 0 of '
    '^\[desk\] restore files$'
    '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$'
    '^\[desk\] verify: 0 of '
)
TOUCH=("desk mode touch" "desk open terminal" "desk press terminal min" "desk list" "desk verify" "desk mode pointer")
TOUCH_STEPS=('^\[desk\] minimise terminal$' '^\[desk\] window terminal at .* minimised$' '^\[desk\] verify: 0 of ')

# in_order <log> <pattern>...: prints the first pattern not found in order, nothing when all are
in_order() {
    local f="$1"; shift
    DESK_PATS="$(printf '%s\n' "$@")" awk '
        BEGIN { n = split(ENVIRON["DESK_PATS"], p, "\n"); k = 1 }
        k <= n && $0 ~ p[k] { k++ }
        END { if (k <= n) print k " " p[k] }' "$f"
}

grade() { # grade <serial log> <kernel name> <passes: 1 or 2> <pc: 1 for the scale and touch checks>
    local fail=0 S p n i want
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | tr -d '\000-\010\013-\037\177' | sed 's/^[[:space:]]*//; s/[[:space:]]*$//')"
    printf '%s\n' "$S" > "$1.clean"
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    local all=()
    for ((i = 0; i < $3; i++)); do all+=("${STEPS[@]}"); done
    [ "$4" = 1 ] && all+=("${SCALE_STEPS[@]}")
    all+=("${TOUCH_STEPS[@]}")
    p="$(in_order "$1.clean" "${all[@]}")"
    [ -z "$p" ] || _f "step ${p%% *}: no line matching '${p#* }' (after the step before it)" "$2"
    # the normal place comes back exactly after a maximise and a restore (each pass)
    n="$(grep -aE '^\[desk\] window files at [0-9-]+,[0-9]+ size [0-9]+x[0-9]+ focused$' <<< "$S" | awk 'NR % 6 == 1 { a = $0 } NR % 6 == 2 && $0 != a { bad++ } END { print bad + 0 }')"
    [ "$n" -eq 0 ] || _f "a restore did not bring back the window's normal place exactly" "$2"
    grep -aqE '^\[desk\] verify: [1-9][0-9]* of ' <<< "$S" && _f "a desk verify found rows that differ" "$2"
    want=$(( $3 * 9 + 1 ))
    [ "$4" = 1 ] && want=$(( want + 3 ))
    n="$(grep -acE '^\[desk\] verify: 0 of ' <<< "$S")"
    [ "$n" -eq "$want" ] || _f "$n desk verify runs found 0 rows, not $want" "$2"
    return "$fail"
}

# a green pass, as the grader reads it
green_pass() {
    cat << 'EOF'
[desk] open files
[desk] window files at 48,45 size 372x250 focused
[desk] maximise files
[desk] window files at 44,0 size 596x480 maximised focused
[desk] verify: 0 of 480 rows differ after a full redraw (2 partial frame(s) drawn before it)
[desk] restore files
[desk] window files at 48,45 size 372x250 focused
[desk] verify: 0 of 480 rows differ after a full redraw (3 partial frame(s) drawn before it)
[desk] maximise files
[desk] restore files
[desk] maximise files
[desk] verify: 0 of 480 rows differ after a full redraw (6 partial frame(s) drawn before it)
[desk] restore files
[desk] moved files to 104,60
[desk] window files at 104,60 size 372x250 focused
[desk] verify: 0 of 480 rows differ after a full redraw (8 partial frame(s) drawn before it)
[desk] minimise files
[desk] verify: 0 of 480 rows differ after a full redraw (9 partial frame(s) drawn before it)
[desk] focus files
[desk] window files at 104,60 size 372x250 focused
[desk] snap left files
[desk] window files at 44,0 size 298x480 snapped left focused
[desk] verify: 0 of 480 rows differ after a full redraw (11 partial frame(s) drawn before it)
[desk] restore files
[desk] snap right files
[desk] verify: 0 of 480 rows differ after a full redraw (13 partial frame(s) drawn before it)
[desk] restore files
[desk] snap left files
[desk] window files at 44,0 size 298x480 snapped left focused
[desk] verify: 0 of 480 rows differ after a full redraw (15 partial frame(s) drawn before it)
[desk] restore files
[desk] resized files to 422x280
[desk] resized files to 240x150
[desk] window files at 104,60 size 240x150 focused
[desk] resized files to 280x150
[desk] resized files to 280x190
[desk] window files at 104,60 size 280x190 focused
[desk] verify: 0 of 480 rows differ after a full redraw (19 partial frame(s) drawn before it)
[desk] close files
EOF
}
green_tail() {
    cat << 'EOF'
[desk] minimise terminal
[desk] window terminal at 44,0 size 596x480 minimised
[desk] verify: 0 of 480 rows differ after a full redraw (21 partial frame(s) drawn before it)
EOF
}
green_scale() {
    cat << 'EOF'
[desk] maximise files
screen: the desktop is now 1280x720 logical pixels at scale 1.50 on the screen's own 1920x1080 pixels
[desk] window files at 44,0 size 1236x720 maximised focused
[desk] verify: 0 of 1080 rows differ after a full redraw (25 partial frame(s) drawn before it)
[desk] window files at 44,0 size 916x540 maximised focused
[desk] verify: 0 of 540 rows differ after a full redraw (26 partial frame(s) drawn before it)
[desk] restore files
[desk] window files at 104,60 size 280x190 focused
[desk] verify: 0 of 540 rows differ after a full redraw (27 partial frame(s) drawn before it)
EOF
}

selftest() {
    local D pass=0 bad=0 ARM PC
    D="$(mktemp -d "$T/desk-windows-selftest.XXXXXX")" || return 1
    ARM="$(green_pass; green_tail)"
    PC="$(green_pass; green_pass; green_scale; green_tail)"
    case_() { # case_ <name> <expected green|red> <log> <passes> <pc>
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k "$4" "$5" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ arm-green green "$ARM" 1 0
    case_ pc-green green "$PC" 2 1
    case_ empty red "" 1 0
    case_ panic red "$(printf '%s\nKERNEL PANIC at desk.rs\n' "$ARM")" 1 0
    case_ no-maximise red "$(sed '0,/maximise files/s/maximise files/maximize files/' <<< "$ARM")" 1 0
    case_ rest-moved red "$(sed '0,/at 48,45 size 372x250 focused/! s/at 48,45 size 372x250 focused/at 50,45 size 372x250 focused/' <<< "$ARM")" 1 0
    case_ stale-rows red "$(sed '0,/verify: 0 of 480/s/verify: 0 of 480/verify: 9 of 480/' <<< "$ARM")" 1 0
    case_ min-not-clamped red "$(sed 's/resized files to 240x150/resized files to 200x100/; s/size 240x150 focused/size 200x100 focused/' <<< "$ARM")" 1 0
    case_ no-snap red "$(sed 's/snap right files/moved files to 300,60/' <<< "$ARM")" 1 0
    case_ touch-no-min red "$(sed 's/minimise terminal/focus terminal/' <<< "$ARM")" 1 0
    case_ scale-lost-max red "$(sed 's/size 916x540 maximised focused/size 372x250 focused/' <<< "$PC")" 2 1
    case_ second-pass-missing red "$(green_pass; green_scale; green_tail)" 2 1
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 12 ]
}

echo "== window controls: maximise, restore, minimise, snap and resize, with verify after each step =="
if ! selftest; then echo "DESK WINDOWS TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK WINDOWS SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-windows.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0
show() { tr -d '\r' < "$1" | grep -aE '^\[desk\] (open|close|maximise|restore|minimise|focus|snap|resized|moved|window|verify)|^screen: the desktop is now|panic|PANIC' | sed "s/^/  $2 | /" | head -120; }
mkdir -p "$W/arm" "$W/pc"

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK WINDOWS TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:6150,server,nowait \
    -qmp tcp:127.0.0.1:6151,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
$PY scripts/lib/cell-keys.py 6150 6151 "$(winpath "$W/arm")" "gui" "desk mode pointer" "${PASS[@]}" "${TOUCH[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/arm.log" arm
grade "$W/arm.log" arm 1 0 || fails=$((fails + 1))

echo "  --- PC kernel (1920x1080, at 1x and at 2x) ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK WINDOWS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1920,yres=1080 \
    -serial "tcp:127.0.0.1:6152,server,nowait" -qmp "tcp:127.0.0.1:6153,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 6152 6153 "$(winpath "$W/pc")" "screen ui 1" "gui" "desk mode pointer" "${PASS[@]}" \
    "screen ui 2" "${PASS[@]}" "screen ui 1" "${PC_SCALE[@]}" "${TOUCH[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/pc.log" pc
grade "$W/pc.log" pc 2 1 || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "DESK WINDOWS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK WINDOWS TEST FAILED"
exit 1
