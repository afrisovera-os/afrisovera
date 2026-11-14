#!/usr/bin/env bash
# The cell window on the desk (afriui.md section 24, stage 1, 13 Nov 2026), on both kernels.
#
# Boots each kernel, shows the desk in pointer mode and drives the cell window with the shell's
# `desk cell` verbs, the same calls an app will make in stage 2:
#   desk cell open                  -> a 65x24 grid (the window's home size, 410x360)
#   desk cell demo                  -> an editor-like page in all 16 roles; its grid hash
#   desk verify                     -> 0 rows differ from a full redraw
#   desk cell put 2 12 03 ...       -> 11 cells, and only that row's band (13 rows) redrawn
#   desk cell cursor 5 12 1         -> the cursor moves; only its two cells redrawn
#   desk cell put 60 5 ...          -> clipped at the grid's edge: 5 of 8 cells
#   desk cell put <usize::MAX> ...  -> 0 cells, in x and in y (the clip saturates)
#   desk cell put 0 13 0f \x80\x01\xC3\x7FA, then `desk cell same`
#                                   -> 0x80, 0x01, 0xC3 and 0x7F draw the same box; "A" differs
#   desk cell size                  -> the grid hash after the puts
#   desk verify                     -> 0 rows differ, after partial frames
#   desk cell resize 300 200        -> 47x12, then a put and desk verify (the architect's case)
#   desk cell resize 20 20          -> the smallest window: 3x0, a put writes 0 cells, desk verify
# Graded per kernel: every line exactly; the two grid hashes against CELL_DEMO_HASH and
# CELL_PUT_HASH (empty = printed, not graded); `desk cell roles`: every text role at 4.5:1 or more on the body, the
# cursor line and the selection, and the title's unsigned mark and Close anyway on their own grounds (section 32).
#   scripts/desk-cell-test.sh              # grader selftest, then the two boots
#   scripts/desk-cell-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
CELL_DEMO_HASH="${CELL_DEMO_HASH-1c6f918fc2716b05}"  # the cloud runner 13 Nov on 3e5a00fd, the same on both kernels
CELL_PUT_HASH="${CELL_PUT_HASH-7a8d0bf5c19f47b4}"
MAXU=18446744073709551615

# the commands, in order (the same on both kernels)
CMDS=(
    "gui" "desk mode pointer" "desk cell open" "desk cell demo" "desk verify"
    "desk cell put 2 12 03 hello world" "desk cell cursor 5 12 1"
    'desk cell put 60 5 0f \x80\x80\x80\x80\x80\x80\x80\x80'
    "desk cell put $MAXU 0 0f x" "desk cell put 0 $MAXU 0f x"
    'desk cell put 0 13 0f \x80\x01\xC3\x7FA'
    "desk cell same 0 13 1 13" "desk cell same 0 13 2 13" "desk cell same 0 13 3 13" "desk cell same 0 13 4 13"
    "desk cell size" "desk verify"
    "desk cell resize 300 200" "desk cell put 1 1 02 after" "desk verify"
    "desk cell resize 20 20" "desk cell put 0 0 0f x" "desk verify" "desk cell roles"
)
CONTRAST_MIN=4.5 # every text role on the body, the cursor line and the selection; the title's marks (section 32)
# The apps' bars sit on the cursor line (12) with text roles 10, 11, 13 and 15 on it (Andika, Hati, Ngoma, the native
# apps engineer, 14 Nov): those pairs are in this grade through "on the cursor line"; keep them there.

# The WCAG contrast failures in a `desk cell roles` line, one per line ("role 6 on the selection: 3.0"); nothing when
# every text role (all but 9 selection and 12 cursor line) reads at CONTRAST_MIN or more on all three grounds.
contrast_fails() {
    awk -v line="$1" -v min="$CONTRAST_MIN" '
    function hx(t,   i, n) { n = 0; t = toupper(t); for (i = 1; i <= length(t); i++) n = n * 16 + index("0123456789ABCDEF", substr(t, i, 1)) - 1; return n }
    function ch(v) { v /= 255; return v <= 0.03928 ? v / 12.92 : exp(2.4 * log((v + 0.055) / 1.055)) }
    function lum(h) { return 0.2126 * ch(hx(substr(h, 1, 2))) + 0.7152 * ch(hx(substr(h, 3, 2))) + 0.0722 * ch(hx(substr(h, 5, 2))) }
    function ratio(a, b,   x, y) { x = lum(a); y = lum(b); return x > y ? (x + 0.05) / (y + 0.05) : (y + 0.05) / (x + 0.05) }
    function need(what, a, b,   r) { r = ratio(a, b); if (r < min) printf "%s: %.1f\n", what, r }
    BEGIN {
        if (!match(line, /body [0-9A-F]{6};/)) { print "no body colour"; exit }
        body = substr(line, RSTART + 5, 6)
        n = 0
        rest = line
        while (match(rest, / [0-9]+=[0-9A-F]{6}/)) {
            t = substr(rest, RSTART + 1, RLENGTH - 1); split(t, kv, "="); role[kv[1]] = kv[2]; n++
            rest = substr(rest, RSTART + RLENGTH)
        }
        if (n != 16) { print "not 16 roles"; exit }
        for (i = 0; i < 16; i++) {
            if (i == 9 || i == 12) continue
            need("role " i " on the body", role[i], body)
            need("role " i " on the cursor line", role[i], role[12])
            need("role " i " on the selection", role[i], role[9])
        }
        if (match(line, /unsigned [0-9A-F]{6} on [0-9A-F]{6}/)) need("the unsigned mark", substr(line, RSTART + 9, 6), substr(line, RSTART + 19, 6)); else print "no unsigned mark"
        if (match(line, /close anyway [0-9A-F]{6} on [0-9A-F]{6}/)) need("Close anyway", substr(line, RSTART + 13, 6), substr(line, RSTART + 23, 6)); else print "no Close anyway"
    }'
}

grade() { # grade <serial log> <kernel name>
    local fail=0 S got h
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxF "$1" <<< "$S" || _f "no line: $1" "$2"; }
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    has "[desk] cell: 65x24 grid" "$2"
    h="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] cell demo: 65x24 grid, hash ([0-9a-f]{16})$/\1/p' | head -1)"
    if [ -z "$h" ]; then _f "no demo line for a 65x24 grid" "$2"
    elif [ -n "$CELL_DEMO_HASH" ] && [ "$h" != "$CELL_DEMO_HASH" ]; then _f "the demo grid hashes $h, not $CELL_DEMO_HASH" "$2"
    elif [ -z "$CELL_DEMO_HASH" ]; then echo "  note ($2): demo grid hash $h (no expected hash set yet, not graded)"; fi
    has "[desk] cell put: 11 cell(s) at 2,12; 13 row(s) redrawn" "$2"
    got="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] cell cursor: 5,12 shape 1; ([0-9]+) row\(s\) redrawn$/\1/p' | head -1)"
    if [ -z "$got" ]; then _f "no cursor line" "$2"
    elif [ "$got" -lt 13 ] || [ "$got" -gt 26 ]; then _f "the cursor move redrew $got rows, not its two cells' bands (13 to 26)" "$2"; fi
    grep -aqE '^\[desk\] cell put: 5 cell\(s\) at 60,5; [0-9]+ row\(s\) redrawn$' <<< "$S" ||
        _f "a put past the grid's right edge did not stop at 5 cells" "$2"
    grep -aqE "^\[desk\] cell put: 0 cell\(s\) at $MAXU,0; [0-9]+ row\(s\) redrawn$" <<< "$S" ||
        _f "a put at column usize::MAX did not write 0 cells" "$2"
    grep -aqE "^\[desk\] cell put: 0 cell\(s\) at 0,$MAXU; [0-9]+ row\(s\) redrawn$" <<< "$S" ||
        _f "a put at row usize::MAX did not write 0 cells" "$2"
    has "[desk] cell put: 5 cell(s) at 0,13; 13 row(s) redrawn" "$2"
    has "[desk] cell same: 0,13 and 1,13 draw the same" "$2"
    has "[desk] cell same: 0,13 and 2,13 draw the same" "$2"
    has "[desk] cell same: 0,13 and 3,13 draw the same" "$2"
    has "[desk] cell same: 0,13 and 4,13 differ" "$2"
    h="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] cell: 65x24 grid, cursor 5,12 shape 1, hash ([0-9a-f]{16})$/\1/p' | head -1)"
    if [ -z "$h" ]; then _f "no size line for the grid after the puts" "$2"
    elif [ -n "$CELL_PUT_HASH" ] && [ "$h" != "$CELL_PUT_HASH" ]; then _f "the grid after the puts hashes $h, not $CELL_PUT_HASH" "$2"
    elif [ -z "$CELL_PUT_HASH" ]; then echo "  note ($2): grid hash after the puts $h (no expected hash set yet, not graded)"; fi
    has "[desk] cell: 47x12 grid after resize" "$2"
    has "[desk] cell put: 5 cell(s) at 1,1; 13 row(s) redrawn" "$2"
    has "[desk] cell: 3x0 grid after resize" "$2"
    grep -aqE '^\[desk\] cell put: 0 cell\(s\) at 0,0; [0-9]+ row\(s\) redrawn$' <<< "$S" ||
        _f "a put in a 3x0 grid did not write 0 cells" "$2"
    got="$(printf '%s\n' "$S" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw')"
    [ "$got" -eq 4 ] || _f "$got of 4 desk verify runs found 0 rows differ" "$2"
    grep -aqE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw \([1-9][0-9]* partial frame' <<< "$S" ||
        _f "no desk verify came after partial frames" "$2"
    got="$(grep -a '^\[desk\] cell roles: ' <<< "$S" | head -1)"
    if [ -z "$got" ]; then _f "no cell roles line" "$2"
    else
        while read -r v; do [ -n "$v" ] && _f "under $CONTRAST_MIN:1: $v" "$2"; done < <(contrast_fails "$got")
    fi
    return "$fail"
}

ROLES="[desk] cell roles: body 0E0B09; 0=D8CEBE 1=A3988B 2=F2C14E 3=33AE75 4=B0A391 5=D08A2E 6=DC815D 7=A59889 8=C8963A 9=3A2E22 10=F2EADC 11=B0A391 12=1C1612 13=E8796F 14=4FC08A 15=7A9CE0; unsigned 0E0B09 on D08A2E, close anyway F2EADC on C1121F"
GREEN="afrisovera> desk cell open
[desk] open cell
[desk] cell: 65x24 grid
afrisovera> desk cell demo
[desk] cell demo: 65x24 grid, hash 0123456789abcdef
afrisovera> desk verify
[desk] verify: 0 of 480 rows differ after a full redraw (0 partial frame(s) drawn before it)
[desk] cell put: 11 cell(s) at 2,12; 13 row(s) redrawn
[desk] cell cursor: 5,12 shape 1; 26 row(s) redrawn
[desk] cell put: 5 cell(s) at 60,5; 13 row(s) redrawn
[desk] cell put: 0 cell(s) at $MAXU,0; 0 row(s) redrawn
[desk] cell put: 0 cell(s) at 0,$MAXU; 0 row(s) redrawn
[desk] cell put: 5 cell(s) at 0,13; 13 row(s) redrawn
[desk] cell same: 0,13 and 1,13 draw the same
[desk] cell same: 0,13 and 2,13 draw the same
[desk] cell same: 0,13 and 3,13 draw the same
[desk] cell same: 0,13 and 4,13 differ
[desk] cell: 65x24 grid, cursor 5,12 shape 1, hash 1123456789abcdef
[desk] verify: 0 of 480 rows differ after a full redraw (5 partial frame(s) drawn before it)
[desk] cell: 47x12 grid after resize
[desk] cell put: 5 cell(s) at 1,1; 13 row(s) redrawn
[desk] verify: 0 of 480 rows differ after a full redraw (6 partial frame(s) drawn before it)
[desk] cell: 3x0 grid after resize
[desk] cell put: 0 cell(s) at 0,0; 0 row(s) redrawn
[desk] verify: 0 of 480 rows differ after a full redraw (6 partial frame(s) drawn before it)
$ROLES"

selftest() {
    local D pass=0 bad=0
    local CELL_DEMO_HASH=0123456789abcdef CELL_PUT_HASH=1123456789abcdef # the sample's hashes
    D="$(mktemp -d "$T/desk-cell-selftest.XXXXXX")" || return 1
    case_() { # case_ <name> <expected green|red> <log>
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    case_ empty red ""
    case_ panic red "$(printf '%s\nKERNEL PANIC at desk.rs\n' "$GREEN")"
    case_ grid-size-bad red "$(printf '%s\n' "$GREEN" | sed 's/^\[desk\] cell: 65x24 grid$/[desk] cell: 64x24 grid/')"
    case_ demo-hash-bad red "$(printf '%s\n' "$GREEN" | sed 's/hash 0123456789abcdef/hash 0123456789abcdee/')"
    case_ demo-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell demo:')"
    case_ put-copies-more red "$(printf '%s\n' "$GREEN" | sed 's/at 2,12; 13 row/at 2,12; 480 row/')"
    case_ cursor-copies-more red "$(printf '%s\n' "$GREEN" | sed 's/shape 1; 26 row/shape 1; 480 row/')"
    case_ cursor-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell cursor:')"
    case_ edge-not-clipped red "$(printf '%s\n' "$GREEN" | sed 's/5 cell(s) at 60,5/8 cell(s) at 60,5/')"
    case_ max-col-wrote red "$(printf '%s\n' "$GREEN" | sed "s/0 cell(s) at $MAXU,0/1 cell(s) at $MAXU,0/")"
    case_ max-row-missing red "$(printf '%s\n' "$GREEN" | grep -v "at 0,$MAXU")"
    case_ box-differs red "$(printf '%s\n' "$GREEN" | sed 's/0,13 and 2,13 draw the same/0,13 and 2,13 differ/')"
    case_ letter-same red "$(printf '%s\n' "$GREEN" | sed 's/0,13 and 4,13 differ/0,13 and 4,13 draw the same/')"
    case_ put-hash-bad red "$(printf '%s\n' "$GREEN" | sed 's/hash 1123456789abcdef/hash 1123456789abcdee/')"
    case_ stale-rows red "$(printf '%s\n' "$GREEN" | sed '0,/verify: 0 of/s/verify: 0 of/verify: 13 of/')"
    case_ resize-verify-missing red "$(printf '%s\n' "$GREEN" | sed '$d')"
    case_ resize-bad red "$(printf '%s\n' "$GREEN" | sed 's/47x12 grid after resize/48x12 grid after resize/')"
    case_ tiny-underflow red "$(printf '%s\n' "$GREEN" | sed 's/3x0 grid after resize/3x4294967295 grid after resize/')"
    case_ no-partial-before-verify red "$(printf '%s\n' "$GREEN" | sed 's/([1-9][0-9]* partial/(0 partial/g')"
    case_ roles-missing red "$(grep -v 'cell roles:' <<< "$GREEN")"
    case_ punct-old red "$(sed 's/ 6=DC815D / 6=C4562B /' <<< "$GREEN")"
    case_ gutter-old red "$(sed 's/ 7=A59889 / 7=6E6254 /' <<< "$GREEN")"
    case_ selection-light red "$(sed 's/ 9=3A2E22 / 9=5A4E42 /' <<< "$GREEN")"
    case_ cursor-line-light red "$(sed 's/ 12=1C1612 / 12=4A4038 /' <<< "$GREEN")"
    case_ role-missing red "$(sed 's/ 15=7A9CE0//' <<< "$GREEN")"
    case_ anyway-weak red "$(sed 's/close anyway F2EADC on C1121F/close anyway F2EADC on E8A090/' <<< "$GREEN")"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 28 ]
}

echo "== the cell window on the desk: put, cursor, clip, box, resize, verify, on both kernels =="
if ! selftest; then echo "DESK CELL TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK CELL SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-cell.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK CELL TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5984,server,nowait \
    -qmp tcp:127.0.0.1:5985,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py 5984 5985 "$(winpath "$W/arm.png")" "${CMDS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[desk\] (cell|verify)|panic|PANIC' | sed 's/^/  arm | /'
grade "$W/arm.log" arm || fails=$((fails + 1))

echo "  --- PC kernel ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "DESK CELL TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK CELL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:5986,server,nowait" -qmp "tcp:127.0.0.1:5987,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5986 5987 "$(winpath "$W/pc.png")" "${CMDS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] (cell|verify)|panic|PANIC' | sed 's/^/  pc  | /'
grade "$W/pc.log" pc || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm.png $W/pc.png"
    echo "DESK CELL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK CELL TEST FAILED"
exit 1
