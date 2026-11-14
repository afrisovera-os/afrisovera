#!/usr/bin/env bash
# The faster console (11 Nov 2026, afriui.md section 18; the architect's F6 item 2): one console line with a scroll
# must take under 5 ms at opt-level 0 on both kernels, because `uart::_print` holds interrupts masked while the
# framebuffer console draws it.
# Boots the ARM kernel (ramfb) and then the PC kernel, and types `fbcon time` on each. The verb writes 8 full-width
# lines through `kprintln!` at the console's bottom row (so each ends in a scroll) and times them, then times the
# scroll on its own and checks it (the moved rows hash the same, the new bottom row is all background) and checks
# the last line's pixels against the font. Graded per kernel: the fastest of the 8 lines under FBCON_BUDGET_US
# (default 5000; QEMU's timing varies, so the slowest is printed, not graded), 8 of 8 scroll checks, 0 glyph pixels
# differ, no panic.
# Then `fbcon blit` in the same boots (afriui.md section 19): the fastest console-block blit under BLIT_BUDGET_US
# (default 5000), 192 of 192 pixels in the copy check, 200 of 200 inside and the edge unchanged in the clip check.
# Then `fbcon wm` (afriui.md section 20): win_create at 240x130 and 40x20 each under WM_BUDGET_US (default 1000) and
# win_present under WM_PRESENT_BUDGET_US (default 3000: it repaints the whole wallpaper, and a whole-screen blit alone
# takes 0.8 to 1.3 ms on ARM; the old code took about 6000), a re-created window surface all zero (31200 of 31200), and the screen after win_present with two
# patterned windows hashing to the old code's hash for that kernel (WM_HASH_ARM, WM_HASH_PC, set from the probe
# commit's the cloud runner run; empty = printed, not graded). On ARM, `wm` then leaves 3 app windows in use, `clear` brings the console
# back, and a second `fbcon wm` must refuse with one line rather than reset them (the architect's condition 1).
# `fbcon blit` also times draw_string (A to Z at scales 1 and 2) and draw_char x26, each under TEXT_BUDGET_US
# (default 500; afriui.md section 23: the old code took 593 to 1471 us, the new 240 to 365), and grades its text check exactly: the old code's screen hash and end x,
# 90 pixels lit by text hanging off the bottom right corner, the rest unchanged, and far text colouring nothing.
# `fbcon wm` also times win_fill over a whole 240x130 and a whole 40x20 window (afriui.md section 21), each under
# WM_BUDGET_US, and checks the clip: 1200 of 1200 inside for a fill hanging past the edges, 100 of 100 for one with
# usize::MAX width and height, 0 pixels from a fill at the right edge or 0 wide, 0 from text far outside the surface,
# nothing else changed.
#   scripts/fbcon-time-test.sh              # grader selftest, then the two boots
#   scripts/fbcon-time-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
BUDGET="${FBCON_BUDGET_US:-5000}"
BBUDGET="${BLIT_BUDGET_US:-5000}"
WBUDGET="${WM_BUDGET_US:-1000}"
PBUDGET="${WM_PRESENT_BUDGET_US:-3000}"
TBUDGET="${TEXT_BUDGET_US:-500}"
WM_HASH_ARM="${WM_HASH_ARM-d6da56a7c9f713a4}"  # afriui.md section 22: the cloud runner afriui-bar-job (578f8757), was 3962d25c879f188c
WM_HASH_PC="${WM_HASH_PC-d6da56a7c9f713a4}"

grade_one() { # grade_one <serial> <kernel name>
    local fail=0 S got
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[fbcon\] one line with a scroll: fastest ([0-9]+) us, slowest [0-9]+ us over 8 lines; the scroll alone: [0-9]+ us$/\1/p' | head -1)"
    if [ -z "$got" ]; then _f "no timing line" "$2"
    elif [ "$got" -ge "$BUDGET" ]; then _f "the fastest line took $got us, not under $BUDGET us" "$2"; fi
    printf '%s\n' "$S" | grep -aqE '^\[fbcon\] scroll check: 8 of 8 scrolls moved every row up one text row$' ||
        _f "a scroll did not move every row up one text row" "$2"
    printf '%s\n' "$S" | grep -aqE '^\[fbcon\] glyph check: 76 chars, 0 pixels differ$' ||
        _f "the console's text does not match the font" "$2"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[fbblit\] console block 616x384: fastest ([0-9]+) us; whole screen 640x480: fastest [0-9]+ us$/\1/p' | head -1)"
    if [ -z "$got" ]; then _f "no blit timing line" "$2"
    elif [ "$got" -ge "$BBUDGET" ]; then _f "the fastest console-block blit took $got us, not under $BBUDGET us" "$2"; fi
    printf '%s\n' "$S" | grep -aqE '^\[fbblit\] copy check: 192 of 192 pixels right; clip check: 200 of 200 inside, edge unchanged$' ||
        _f "a blit check failed" "$2"
    local tl t1 t2 t3
    tl="$(printf '%s\n' "$S" | sed -nE 's/^\[fbtext\] draw_string 26 chars at scale 1: fastest ([0-9]+) us; at scale 2: fastest ([0-9]+) us; draw_char 26 at scale 1: fastest ([0-9]+) us$/\1 \2 \3/p' | head -1)"
    if [ -z "$tl" ]; then _f "no text timing line" "$2"
    else
        read -r t1 t2 t3 <<< "$tl"
        [ "$t1" -lt "$TBUDGET" ] || _f "draw_string at scale 1 took $t1 us, not under $TBUDGET us" "$2"
        [ "$t2" -lt "$TBUDGET" ] || _f "draw_string at scale 2 took $t2 us, not under $TBUDGET us" "$2"
        [ "$t3" -lt "$TBUDGET" ] || _f "draw_char x26 took $t3 us, not under $TBUDGET us" "$2"
    fi
    printf '%s\n' "$S" | grep -aqxF '[fbtext] text check: screen hash 146c9edc133e9276, end x 1003; clip check: 90 lit in the corner, rest unchanged, far text 0' ||
        _f "the text check differs from the old code's pixels (or far text was drawn)" "$2"
    local wl c1 c2 pr want hash
    wl="$(printf '%s\n' "$S" | sed -nE 's/^\[fbwm\] win_create 240x130: fastest ([0-9]+) us; 40x20: fastest ([0-9]+) us; win_present: fastest ([0-9]+) us \(wallpaper [0-9]+, text [0-9]+, windows [0-9]+\)$/\1 \2 \3/p' | head -1)"
    if [ -z "$wl" ]; then _f "no window-server timing line" "$2"
    else
        read -r c1 c2 pr <<< "$wl"
        [ "$c1" -lt "$WBUDGET" ] || _f "win_create 240x130 took $c1 us, not under $WBUDGET us" "$2"
        [ "$c2" -lt "$WBUDGET" ] || _f "win_create 40x20 took $c2 us, not under $WBUDGET us" "$2"
        [ "$pr" -lt "$PBUDGET" ] || _f "win_present took $pr us, not under $PBUDGET us" "$2"
    fi
    hash="$(printf '%s\n' "$S" | sed -nE 's/^\[fbwm\] clear check: 31200 of 31200 pixels zero; screen hash ([0-9a-f]{16})$/\1/p' | head -1)"
    if [ -z "$hash" ]; then _f "no clear-check line, or a new window's surface was not all zero" "$2"
    else
        if [ "$2" = arm ]; then want="$WM_HASH_ARM"; else want="$WM_HASH_PC"; fi
        if [ -z "$want" ]; then echo "  note ($2): screen hash $hash (no expected hash set yet, not graded)"
        elif [ "$hash" != "$want" ]; then _f "the screen after win_present hashes $hash, not the old code's $want" "$2"; fi
    fi
    local fl f1 f2 fclip
    fl="$(printf '%s\n' "$S" | sed -nE 's/^\[fbwm\] win_fill 240x130: fastest ([0-9]+) us; 40x20: fastest ([0-9]+) us; clip check: (.*)$/\1 \2 \3/p' | head -1)"
    if [ -z "$fl" ]; then _f "no window-fill line" "$2"
    else
        read -r f1 f2 fclip <<< "$fl"
        [ "$f1" -lt "$WBUDGET" ] || _f "win_fill 240x130 took $f1 us, not under $WBUDGET us" "$2"
        [ "$f2" -lt "$WBUDGET" ] || _f "win_fill 40x20 took $f2 us, not under $WBUDGET us" "$2"
        [ "$fclip" = "1200 of 1200 inside, 100 of 100 corner, 0 from empty fills, 0 from far text, rest unchanged" ] ||
            _f "win_fill clip check: $fclip (want 1200 of 1200 inside, 100 of 100 corner, 0 from empty fills, 0 from far text, rest unchanged)" "$2"
    fi
    if [ "$2" = arm ]; then
        printf '%s\n' "$S" | grep -aqE '^\[fbwm\] refused: [1-9] window\(s\) in use, desk hidden, console shown ' ||
            _f "fbcon wm did not refuse while app windows were in use" "$2"
    fi
    if printf '%s\n' "$S" | grep -aqE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked" "$2"; fi
    return "$fail"
}

grade() { # grade <arm serial> <pc serial>
    local fail=0
    grade_one "$1" arm || fail=1
    grade_one "$2" pc || fail=1
    return "$fail"
}

GREEN="afrisovera> fbcon time
[FBCON] LINE 8 OF 8: 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 ABCDEF
[fbcon] one line with a scroll: fastest 1800 us, slowest 4100 us over 8 lines; the scroll alone: 900 us
[fbcon] scroll check: 8 of 8 scrolls moved every row up one text row
[fbcon] glyph check: 76 chars, 0 pixels differ
afrisovera> fbcon blit
[fbblit] console block 616x384: fastest 900 us; whole screen 640x480: fastest 1500 us
[fbblit] copy check: 192 of 192 pixels right; clip check: 200 of 200 inside, edge unchanged
[fbtext] draw_string 26 chars at scale 1: fastest 120 us; at scale 2: fastest 180 us; draw_char 26 at scale 1: fastest 90 us
[fbtext] text check: screen hash 146c9edc133e9276, end x 1003; clip check: 90 lit in the corner, rest unchanged, far text 0
afrisovera> fbcon wm
[fbwm] win_create 240x130: fastest 80 us; 40x20: fastest 79 us; win_present: fastest 700 us (wallpaper 400, text 200, windows 100)
[fbwm] clear check: 31200 of 31200 pixels zero; screen hash 0123456789abcdef
[fbwm] win_fill 240x130: fastest 60 us; 40x20: fastest 20 us; clip check: 1200 of 1200 inside, 100 of 100 corner, 0 from empty fills, 0 from far text, rest unchanged
afrisovera> fbcon wm
[fbwm] refused: 3 window(s) in use, desk hidden, console shown (fbcon wm needs no windows and the console shown)"

selftest() {
    local D pass=0 bad=0
    local WM_HASH_ARM=0123456789abcdef WM_HASH_PC=0123456789abcdef # the samples' hash, not the real screen's
    D="$(mktemp -d "$T/fbcon-time-selftest.XXXXXX")" || return 1
    arm() {
        local got
        printf '%s\n' "$3" > "$D/a"
        printf '%s\n' "$4" > "$D/p"
        if grade "$D/a" "$D/p" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$GREEN" "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')" "$GREEN"
    arm arm-slow red "$(printf '%s\n' "$GREEN" | sed 's/fastest 1800 us/fastest 5000 us/')" "$GREEN"
    arm pc-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/fastest 1800 us/fastest 114000 us/')"
    arm slowest-not-graded green "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/slowest 4100 us/slowest 90000 us/')"
    arm scroll-bad red "$(printf '%s\n' "$GREEN" | sed 's/8 of 8 scrolls/7 of 8 scrolls/')" "$GREEN"
    arm glyph-bad red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/0 pixels differ/12 pixels differ/')"
    arm no-timing red "$(printf '%s\n' "$GREEN" | grep -v 'one line with a scroll')" "$GREEN"
    arm pc-missing red "$GREEN" ""
    arm panic red "$GREEN" "$(printf '%s\nKERNEL PANIC at framebuffer.rs\n' "$GREEN")"
    arm empty red "" ""
    arm blit-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/fastest 900 us/fastest 61000 us/')"
    arm blit-missing red "$(printf '%s\n' "$GREEN" | grep -v 'console block')" "$GREEN"
    arm copy-bad red "$(printf '%s\n' "$GREEN" | sed 's/192 of 192/191 of 192/')" "$GREEN"
    arm clip-bad red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/200 of 200/180 of 200/')"
    arm edge-changed red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/edge unchanged/edge CHANGED/')"
    arm whole-screen-not-graded green "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/fastest 1500 us/fastest 90000 us/')"
    arm create-slow red "$(printf '%s\n' "$GREEN" | sed 's/240x130: fastest 80 us/240x130: fastest 5120 us/')" "$GREEN"
    arm small-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/40x20: fastest 79 us/40x20: fastest 1000 us/')"
    arm present-slow red "$(printf '%s\n' "$GREEN" | sed 's/win_present: fastest 700 us/win_present: fastest 5874 us/')" "$GREEN"
    arm present-in-budget green "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/win_present: fastest 700 us/win_present: fastest 2999 us/')"
    arm present-at-budget red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/win_present: fastest 700 us/win_present: fastest 3000 us/')"
    arm parts-not-graded green "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/wallpaper 400/wallpaper 9000/')"
    arm wm-missing red "$GREEN" "$(printf '%s\n' "$GREEN" | grep -v '^\[fbwm\] win_create')"
    arm refuse-missing red "$(printf '%s\n' "$GREEN" | grep -v 'refused')" "$GREEN"
    arm refuse-pc-not-needed green "$GREEN" "$(printf '%s\n' "$GREEN" | grep -v 'refused')"
    arm fill-slow red "$(printf '%s\n' "$GREEN" | sed 's/win_fill 240x130: fastest 60 us/win_fill 240x130: fastest 5770 us/')" "$GREEN"
    arm fill-small-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/40x20: fastest 20 us/40x20: fastest 1000 us/')"
    arm fill-missing red "$GREEN" "$(printf '%s\n' "$GREEN" | grep -v 'win_fill')"
    arm fill-clip-bad red "$(printf '%s\n' "$GREEN" | sed 's/1200 of 1200 inside/1199 of 1200 inside/')" "$GREEN"
    arm corner-not-run red "$(printf '%s\n' "$GREEN" | sed 's/100 of 100 corner/corner not run/')" "$GREEN"
    arm empty-fill-coloured red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/0 from empty fills/3 from empty fills/')"
    arm far-text-not-run red "$(printf '%s\n' "$GREEN" | sed 's/0 from far text/far text not run/')" "$GREEN"
    arm fill-rest-changed red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/far text, rest unchanged/far text, rest CHANGED/')"
    arm text-slow red "$(printf '%s\n' "$GREEN" | sed 's/scale 1: fastest 120 us/scale 1: fastest 852 us/')" "$GREEN"
    arm text2-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/scale 2: fastest 180 us/scale 2: fastest 500 us/')"
    arm char-slow red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/draw_char 26 at scale 1: fastest 90 us/draw_char 26 at scale 1: fastest 593 us/')"
    arm text-missing red "$(printf '%s\n' "$GREEN" | grep -v 'draw_string 26')" "$GREEN"
    arm text-hash-bad red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/hash 146c9edc133e9276/hash 146c9edc133e9277/')"
    arm text-end-bad red "$(printf '%s\n' "$GREEN" | sed 's/end x 1003/end x 1002/')" "$GREEN"
    arm text-corner-bad red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/90 lit in the corner/91 lit in the corner/')"
    arm text-rest-changed red "$(printf '%s\n' "$GREEN" | sed 's/corner, rest unchanged/corner, rest CHANGED/')" "$GREEN"
    arm far-text-lit red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/far text 0$/far text 16/')"
    arm far-text-skipped red "$(printf '%s\n' "$GREEN" | sed 's/far text 0$/far text not run/')" "$GREEN"
    arm text-check-missing red "$GREEN" "$(printf '%s\n' "$GREEN" | grep -v 'text check')"
    arm clear-bad red "$(printf '%s\n' "$GREEN" | sed 's/31200 of 31200/31199 of 31200/')" "$GREEN"
    WM_HASH_ARM=0123456789abcdef WM_HASH_PC=0123456789abcdef arm hash-match green "$GREEN" "$GREEN"
    WM_HASH_ARM=0123456789abcdef WM_HASH_PC=0123456789abcdef arm hash-differs red "$GREEN" "$(printf '%s\n' "$GREEN" | sed 's/hash 0123456789abcdef/hash 0123456789abcdee/')"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 48 ]
}

echo "== the faster console: one console line with a scroll, and a blit, on both kernels =="
if ! selftest; then echo "FBCON TIME TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "FBCON TIME SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/fbcon-time.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "FBCON TIME TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5980,server,nowait \
    -qmp tcp:127.0.0.1:5981,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py 5980 5981 "$(winpath "$W/arm.png")" "fbcon time" "fbcon blit" "fbcon wm" "wm" "clear" "fbcon wm" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[fb(con|blit|text|wm)\]|panic|PANIC' | sed 's/^/  arm | /'

echo "  --- PC kernel ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
if command -v qemu-system-x86_64 > /dev/null; then
    cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
    export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
    if [ -r "$PC_EFI" ]; then
        export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
        chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
        PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
            -serial "tcp:127.0.0.1:5982,server,nowait" -qmp "tcp:127.0.0.1:5983,server,nowait" &
        QPID=$!
        AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5982 5983 "$(winpath "$W/pc.png")" \
            "fbcon time" "fbcon blit" "fbcon wm" > "$W/pc.log" 2>&1
        kill "$QPID" 2> /dev/null
        wait "$QPID" 2> /dev/null
        tr -d '\r' < "$W/pc.log" | grep -aE '^\[fb(con|blit|text|wm)\]|panic|PANIC' | sed 's/^/  pc  | /'
    else
        echo "  (harness: $PC_EFI was not built)"
    fi
else
    echo "  (harness: qemu-system-x86_64 not on PATH)"
fi

echo "  --- grading (budget $BUDGET us) ---"
if grade "$W/arm.log" "$W/pc.log"; then
    rm -rf "$W"
    echo "FBCON TIME TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "FBCON TIME TEST FAILED"
exit 1
