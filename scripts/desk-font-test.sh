#!/usr/bin/env bash
# Smooth text that scales (afriui.md section 26, premium look step 1, 13 Nov 2026), on both kernels.
#
# Boots each kernel (the PC on a 1920x1080 screen, the desk at 2x), shows the desk with Settings open and runs
# `desk font probe`, which prints:
#   the speed of the A-to-Z line and a 40-character title, bitmap face against the outline face (cold, warm);
#   a hash of the outline face's coverage for a fixed word with African letters, at cap 9 and 18 (the same bytes on
#     ARM and PC, graded against FONT_HASH_9 and FONT_HASH_18);
#   'o' moved half a pixel keeps its ink (within 2%);
#   U+E000 (not in the font) draws the box;
#   UTF-8 with six African letters and a sequence cut short at the end: one bad character, no read past the end;
#   the glyph cache filled until it empties itself, then the same text draws the same pixels;
#   the largest size (a cap of 1000 asked, 45 given), no line dropped;
#   text across a window's right edge and its cut corner draws nothing outside it;
#   a full frame in each face, the first frame after an empty cache, and a partial frame in each face.
# Then `desk verify` (0 rows differ) in the outline face, after `desk font bitmap`, and after `desk font outline`.
# Then the narrowest cell window title (the architect's follow-up 1): a test label with a long unsigned name and Close
# anyway offered, the window resized to 40 pixels, which must stop at the lintel's minimum width; `desk cell
# titlecheck` must find the mark whole and nothing over Close anyway, and the screenshot ends on that title.
# Graded per kernel: the lines above; the budgets of the architect (13 Nov): a full frame no more than 15% slower
# than the bitmap face, and the first frame after an empty cache under 100 ms on the PC at 1920x1080.
#   scripts/desk-font-test.sh              # grader selftest, then the two boots
#   scripts/desk-font-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
FONT_HASH_9="${FONT_HASH_9-e650b7175d399587}"    # the cloud runner 13 Nov on 7193f02c, the same on both kernels
FONT_HASH_18="${FONT_HASH_18-f792bfcbb21451de}"
FRAME_PCT_MAX=115     # outline full frame as a percentage of the bitmap one
COLD_US_MAX=100000    # the first frame after an empty cache, PC at 1920x1080 (960x540 at 2x)

CMDS=(
    "gui" "desk mode pointer" "desk open settings" "desk font probe" "desk verify"
    "desk font bitmap" "desk verify" "desk font outline" "desk verify" "desk font"
    "desk cell open" "desk cell titletest averyveryverylongname.wasm" "desk cell resize 40 30" "desk verify"
    "desk cell titlecheck"
)

grade() { # grade <serial log> <kernel name> <pc: 1 to grade the cold-frame budget>
    local fail=0 S got a b
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxF "$1" <<< "$S" || _f "no line: $1" "$2"; }
    hasre() { grep -aqE "$1" <<< "$S" || _f "$3" "$2"; }
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    hasre '^\[desk\] font: A to Z at cap 9: bitmap [0-9]+ us, outline cold [0-9]+ us, warm [0-9]+ us \(worst of 20 [0-9]+ us\); at cap 18: bitmap [0-9]+ us, outline cold [0-9]+ us, warm [0-9]+ us \(worst of 20 [0-9]+ us\)$' "$2" "no A to Z speed line"
    hasre '^\[desk\] font: 40-character title at cap 9: bitmap [0-9]+ us, outline cold [0-9]+ us, warm [0-9]+ us \(worst of 20 [0-9]+ us\); at cap 18: ' "$2" "no title speed line"
    a="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] font: coverage hash cap 9 ([0-9a-f]{16}), cap 18 ([0-9a-f]{16})$/\1 \2/p' | head -1)"
    if [ -z "$a" ]; then _f "no coverage hash line" "$2"
    else
        [ -z "$FONT_HASH_9" ] || [ "${a% *}" = "$FONT_HASH_9" ] || _f "the cap 9 coverage hashes ${a% *}, not $FONT_HASH_9" "$2"
        [ -z "$FONT_HASH_18" ] || [ "${a#* }" = "$FONT_HASH_18" ] || _f "the cap 18 coverage hashes ${a#* }, not $FONT_HASH_18" "$2"
    fi
    hasre "^\[desk\] font: half pixel 'o' at cap 18: ink [0-9]+ and [0-9]+ \(within 2%\), " "$2" "half a pixel changed the ink by more than 2%"
    has "[desk] font: U+E000 is not in the font and 7 pixels wide (the box is 7)" "$2"
    has "[desk] font: utf-8: 12 characters, 11 in the font, 1 bad, ended at byte 22 of 22" "$2"
    hasre '^\[desk\] font: cache: [0-9]+ glyphs asked for, emptied [1-9][0-9]* time\(s\), the same text after: same pixels$' "$2" "the cache did not empty itself, or drew different pixels after"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] font: largest: cap 45 when 1000 is asked, boxes up to ([0-9]+)x([0-9]+), [1-9][0-9]* full pixels, lines dropped 0$/\1 \2/p' | head -1)"
    if [ -z "$got" ]; then _f "no largest-size line with cap 45, full pixels and 0 lines dropped" "$2"
    elif [ "${got% *}" -gt 128 ] || [ "${got#* }" -gt 128 ]; then _f "a glyph box ${got/ /x} is past the 128x128 cells" "$2"; fi
    hasre '^\[desk\] font: clip: outside the window and its cut corner unchanged, [1-9][0-9]* pixels drawn inside$' "$2" "text drew outside its window, or nothing inside"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] font: frames at [0-9]+x[0-9]+: full bitmap [0-9]+ us, outline [0-9]+ us \(([0-9]+)%\), the first after an empty cache ([0-9]+) us; partial 300x24 bitmap [0-9]+ us, outline [0-9]+ us$/\1 \2/p' | head -1)"
    if [ -z "$got" ]; then _f "no frames line" "$2"
    else
        [ "${got% *}" -le "$FRAME_PCT_MAX" ] || _f "a full frame in the outline face is ${got% *}% of the bitmap one (budget $FRAME_PCT_MAX%)" "$2"
        if [ "$3" = 1 ] && [ "${got#* }" -ge "$COLD_US_MAX" ]; then _f "the first frame after an empty cache took ${got#* } us (budget $COLD_US_MAX)" "$2"; fi
    fi
    has "[desk] font: bitmap (the Sovereign face)" "$2"
    has "[desk] font: outline (AfriSovera Sans)" "$2"
    has "[desk] font: outline (desk font outline|bitmap|probe|sample)" "$2"
    has "[desk] cell titletest: averyveryverylongname.wa (unsigned, Close anyway offered)" "$2"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] cell title: window ([0-9]+) wide \(at least ([0-9]+)\), mark [0-9]+\.\.[0-9]+ whole, Close anyway [0-9]+\.\.[0-9]+, buttons from [0-9]+; nothing over Close anyway; name cut$/\1 \2/p' | head -1)"
    if [ -z "$got" ]; then _f "no narrow title line with the mark whole, nothing over Close anyway and the name cut" "$2"
    elif [ "${got% *}" != "${got#* }" ]; then _f "the narrow cell window is ${got% *} wide, not its minimum ${got#* }" "$2"; fi
    got="$(printf '%s\n' "$S" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw')"
    [ "$got" -eq 4 ] || _f "$got of 4 desk verify runs found 0 rows differ" "$2"
    return "$fail"
}

GREEN="afrisovera> desk font probe
[desk] font: A to Z at cap 9: bitmap 789 us, outline cold 6572 us, warm 419 us (worst of 20 520 us); at cap 18: bitmap 1245 us, outline cold 12172 us, warm 865 us (worst of 20 990 us)
[desk] font: 40-character title at cap 9: bitmap 754 us, outline cold 8140 us, warm 507 us (worst of 20 640 us); at cap 18: bitmap 1158 us, outline cold 14367 us, warm 978 us (worst of 20 1100 us)
[desk] font: coverage hash cap 9 0123456789abcdef, cap 18 1123456789abcdef
[desk] font: half pixel 'o' at cap 18: ink 19502 and 19502 (within 2%), 162 of 208 pixels differ, 3 full in both, same box no
[desk] font: U+E000 is not in the font and 7 pixels wide (the box is 7)
[desk] font: utf-8: 12 characters, 11 in the font, 1 bad, ended at byte 22 of 22
[desk] font: cache: 1456 glyphs asked for, emptied 2 time(s), the same text after: same pixels
[desk] font: largest: cap 45 when 1000 is asked, boxes up to 58x57, 1817 full pixels, lines dropped 0
[desk] font: clip: outside the window and its cut corner unchanged, 325 pixels drawn inside
[desk] font: frames at 960x540: full bitmap 49371 us, outline 51667 us (104%), the first after an empty cache 84463 us; partial 300x24 bitmap 3019 us, outline 3834 us
[desk] font: cache 7933 bytes in 116 glyphs; made 1884, hits 2229, emptied 7
[desk] verify: 0 of 540 rows differ after a full redraw (3 partial frame(s) drawn before it)
[desk] font: bitmap (the Sovereign face)
[desk] verify: 0 of 540 rows differ after a full redraw (38 partial frame(s) drawn before it)
[desk] font: outline (AfriSovera Sans)
[desk] verify: 0 of 540 rows differ after a full redraw (40 partial frame(s) drawn before it)
[desk] font: outline (desk font outline|bitmap|probe|sample)
[desk] cell: 65x24 grid
[desk] cell titletest: averyveryverylongname.wa (unsigned, Close anyway offered)
[desk] cell: 37x0 grid after resize
[desk] verify: 0 of 540 rows differ after a full redraw (41 partial frame(s) drawn before it)
[desk] cell title: window 259 wide (at least 259), mark 45..97 whole, Close anyway 105..189, buttons from 207; nothing over Close anyway; name cut"

selftest() {
    local D pass=0 bad=0
    local FONT_HASH_9=0123456789abcdef FONT_HASH_18=1123456789abcdef # the sample's hashes
    D="$(mktemp -d "$T/desk-font-selftest.XXXXXX")" || return 1
    case_() { # case_ <name> <expected green|red> <log> [pc]
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k "${4:-1}" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    case_ empty red ""
    case_ panic red "$(printf '%s\nKERNEL PANIC at vector.rs\n' "$GREEN")"
    case_ speed-missing red "$(printf '%s\n' "$GREEN" | grep -v 'A to Z')"
    case_ hash9-bad red "$(printf '%s\n' "$GREEN" | sed 's/cap 9 0123456789abcdef/cap 9 0123456789abcdee/')"
    case_ hash18-bad red "$(printf '%s\n' "$GREEN" | sed 's/cap 18 1123456789abcdef/cap 18 1123456789abcdee/')"
    case_ ink-moved red "$(printf '%s\n' "$GREEN" | sed 's/(within 2%)/(NOT within 2%)/')"
    case_ box-missing red "$(printf '%s\n' "$GREEN" | sed 's/U+E000 is not in the font/U+E000 is IN the font/')"
    case_ utf8-two-bad red "$(printf '%s\n' "$GREEN" | sed 's/11 in the font, 1 bad/10 in the font, 2 bad/')"
    case_ utf8-overread red "$(printf '%s\n' "$GREEN" | sed 's/ended at byte 22 of 22/ended at byte 23 of 22/')"
    case_ cache-never-emptied red "$(printf '%s\n' "$GREEN" | sed 's/emptied 2 time(s), the same/emptied 0 time(s), the same/')"
    case_ cache-different red "$(printf '%s\n' "$GREEN" | sed 's/the same text after: same pixels/the same text after: DIFFERENT pixels/')"
    case_ size-not-clamped red "$(printf '%s\n' "$GREEN" | sed 's/largest: cap 45 when/largest: cap 1000 when/')"
    case_ lines-dropped red "$(printf '%s\n' "$GREEN" | sed 's/lines dropped 0$/lines dropped 3/')"
    case_ box-too-big red "$(printf '%s\n' "$GREEN" | sed 's/boxes up to 58x57/boxes up to 58x129/')"
    case_ clip-leak red "$(printf '%s\n' "$GREEN" | sed 's/cut corner unchanged/cut corner CHANGED/')"
    case_ clip-nothing-inside red "$(printf '%s\n' "$GREEN" | sed 's/unchanged, 325 pixels/unchanged, 0 pixels/')"
    case_ frame-slow red "$(printf '%s\n' "$GREEN" | sed 's/(104%)/(116%)/')"
    case_ cold-slow-pc red "$(printf '%s\n' "$GREEN" | sed 's/empty cache 84463 us/empty cache 100000 us/')"
    case_ cold-slow-arm green "$(printf '%s\n' "$GREEN" | sed 's/empty cache 84463 us/empty cache 130000 us/')" 0
    case_ switch-missing red "$(printf '%s\n' "$GREEN" | grep -v 'Sovereign face')"
    case_ stale-rows red "$(printf '%s\n' "$GREEN" | sed '0,/verify: 0 of/s/verify: 0 of/verify: 12 of/')"
    case_ verify-missing red "$(printf '%s\n' "$GREEN" | grep -v '40 partial')"
    case_ worst-missing red "$(printf '%s\n' "$GREEN" | sed 's/ (worst of 20 520 us)//')"
    case_ title-overlap red "$(printf '%s\n' "$GREEN" | sed 's/nothing over Close anyway/OVERLAP/')"
    case_ title-mark-cut red "$(printf '%s\n' "$GREEN" | sed 's/mark 45..97 whole/mark 45..97 NOT WHOLE/')"
    case_ title-too-narrow red "$(printf '%s\n' "$GREEN" | sed 's/window 259 wide (at least 259)/window 40 wide (at least 259)/')"
    case_ title-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell title:')"
    case_ titletest-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell titletest:')"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 30 ]
}

echo "== smooth text that scales: the outline face's checks and budgets, on both kernels =="
if ! selftest; then echo "DESK FONT TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK FONT SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-font.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK FONT TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5992,server,nowait \
    -qmp tcp:127.0.0.1:5993,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py 5992 5993 "$(winpath "$W/arm.png")" "${CMDS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[desk\] (font|verify)|panic|PANIC' | sed 's/^/  arm | /'
grade "$W/arm.log" arm 0 || fails=$((fails + 1))

echo "  --- PC kernel (1920x1080) ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "DESK FONT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK FONT TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1920,yres=1080 \
    -serial "tcp:127.0.0.1:5994,server,nowait" -qmp "tcp:127.0.0.1:5995,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5994 5995 "$(winpath "$W/pc.png")" "screen ui" "${CMDS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] (font|verify)|panic|PANIC' | sed 's/^/  pc  | /'
grade "$W/pc.log" pc 1 || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm.png $W/pc.png"
    echo "DESK FONT TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK FONT TEST FAILED"
exit 1
