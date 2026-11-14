#!/usr/bin/env bash
# Arabic joined and right to left on the desk (SHAPING_PLAN.md phase C1), on both kernels.
#
# Boots each kernel (the PC on a 1920x1080 screen, the desk at 2x) and runs:
#   desk shape probe: every oracle string's hash (graded against tools/face/shape_expected.txt, HarfBuzz's answers on
#     the same pinned font, made by tools/face/shape_src.py in the cloud); the sample line's coverage hash at cap 9
#     and 18 (the same on ARM and PC); widths drawn as measured; the limits (300 characters cut to 256, explicit
#     controls dropped); the fast test (Latin and Yoruba stay on the section 26 path); the speed of a 40-character
#     Arabic line against the 40-character Latin title;
#   desk shape hex for five lines whose levels and visual order are worked out by hand from UAX 9 below;
#   the architect's title case: an app named in Arabic with Latin and digits, as an unsigned cell window, graded with
#     titlecheck at its normal width and at its narrowest (the mark whole, nothing over Close anyway);
#   the sample notice and the bell's list showing it (the screenshot), and desk verify (0 rows differ) three times.
#   scripts/desk-shape-test.sh              # grader selftest, then the two boots
#   scripts/desk-shape-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
EXPECTED="${SHAPE_EXPECTED:-tools/face/shape_expected.txt}"
SHAPE_HASH_9="${SHAPE_HASH_9-}"    # filled in from the first the cloud runner run; the ARM and PC values must agree anyway
SHAPE_HASH_18="${SHAPE_HASH_18-}"

# The five bidi lines (hex of the UTF-8) and what UAX 9 gives for them, worked by hand:
#  A  مرحبا: five AL, paragraph 1, all level 1, read right to left.
#  B  "AB بك 12": paragraph 0 (A is first); W2 makes 12 Arabic numbers after an AL, so level 2; the space before
#     them sits between R and AN (R), level 1; the first space between L and R takes the paragraph's L.
#  C  "(بك) x": paragraph 1; N0 gives the bracket pair R (R inside), x is L at an odd level, so 2.
#  D  "ab" U+202E "بك": the override is dropped and counted; paragraph 0, the Arabic at level 1.
#  E  "١,٢ بك": paragraph 1 (AN is not strong for P2); W4 joins ١,٢ as AN, level 2 inside the R line.
HEX_A=d985d8b1d8add8a8d8a7
HEX_B=414220d8a8d983203132
HEX_C=28d8a8d983292078
HEX_D=6162e280aed8a8d983
HEX_E=d9a12cd9a220d8a8d983
WANT_A="5 chars, 0 dropped, paragraph 1, levels 1 1 1 1 1, order 4 3 2 1 0,"
WANT_B="8 chars, 0 dropped, paragraph 0, levels 0 0 0 1 1 1 2 2, order 0 1 2 6 7 5 4 3,"
WANT_C="6 chars, 0 dropped, paragraph 1, levels 1 1 1 1 1 2, order 5 4 3 2 1 0,"
WANT_D="4 chars, 1 dropped, paragraph 0, levels 0 0 1 1, order 0 1 3 2,"
WANT_E="6 chars, 0 dropped, paragraph 1, levels 2 2 2 1 1 1, order 5 4 3 0 1 2,"
# the title: تطبيق app 2 (16 bytes)
TITLE_HEX=d8aad8b7d8a8d98ad982206170702032

CMDS=(
    "gui" "desk mode pointer" "desk shape probe"
    "desk shape hex $HEX_A" "desk shape hex $HEX_B" "desk shape hex $HEX_C" "desk shape hex $HEX_D" "desk shape hex $HEX_E"
    "desk cell open" "desk cell titlehex $TITLE_HEX" "desk verify" "desk cell titlecheck"
    "desk cell resize 40 30" "desk cell titlecheck" "desk cell titletest off" "desk cell close"
    "desk shape sample" "desk verify" "desk notes" "desk verify"
)

grade() { # grade <serial log> <kernel name>
    local fail=0 S got k want n
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqxF "$1" <<< "$S" || _f "no line: $1" "$2"; }
    hasre() { grep -aqE "$1" <<< "$S" || _f "$3" "$2"; }
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    # the oracle: every expected line, by index
    [ -r "$EXPECTED" ] || { _f "no expected file $EXPECTED" "$2"; return 1; }
    n=0
    while IFS=$'\t' read -r k want _; do
        case "$k" in ''|\#*) continue ;; esac
        n=$((n + 1))
        got="$(sed -nE "s/^\[desk\] shape oracle $k ([0-9a-f]{16})$/\1/p" <<< "$S" | head -1)"
        [ "$got" = "$want" ] || _f "oracle string $k hashes ${got:-nothing}, HarfBuzz $want" "$2"
    done < "$EXPECTED"
    [ "$n" -gt 0 ] || _f "the expected file has no lines" "$2"
    got="$(sed -nE 's/^\[desk\] shape: coverage hash cap 9 ([0-9a-f]{16}), cap 18 ([0-9a-f]{16})$/\1 \2/p' <<< "$S" | head -1)"
    if [ -z "$got" ]; then _f "no coverage hash line" "$2"
    else
        [ -z "$SHAPE_HASH_9" ] || [ "${got% *}" = "$SHAPE_HASH_9" ] || _f "the cap 9 coverage hashes ${got% *}, not $SHAPE_HASH_9" "$2"
        [ -z "$SHAPE_HASH_18" ] || [ "${got#* }" = "$SHAPE_HASH_18" ] || _f "the cap 18 coverage hashes ${got#* }, not $SHAPE_HASH_18" "$2"
    fi
    has "[desk] shape: widths: 8 of 8 drawn as measured" "$2"
    hasre '^\[desk\] shape: limits: 300 characters gave 44 cut and width [1-9][0-9]*, controls dropped 2$' "$2" "the long line was not cut to 256 with 44 counted, or the two controls not dropped"
    has "[desk] shape: needed: latin no, yoruba no, arabic yes, hebrew yes, control yes" "$2"
    hasre '^\[desk\] shape: arabic 40 characters: cap 9 cold [0-9]+ us, warm [0-9]+ us \(worst of 20 [0-9]+ us\); cap 18 ' "$2" "no Arabic speed line"
    hasre '^\[desk\] shape: latin 40 characters: cap 9 cold [0-9]+ us, warm [0-9]+ us \(worst of 20 [0-9]+ us\); cap 18 ' "$2" "no Latin speed line"
    for want in "$WANT_A" "$WANT_B" "$WANT_C" "$WANT_D" "$WANT_E"; do
        grep -aqF "[desk] shape: $want width " <<< "$S" || _f "no bidi line: $want" "$2"
    done
    [ "$(grep -acE '^\[desk\] shape: oracle [0-9a-f]{16}$' <<< "$S")" -eq 5 ] || _f "not five oracle lines after desk shape hex" "$2"
    has "[desk] cell titlehex: 16 bytes (unsigned, Close anyway offered)" "$2"
    got="$(grep -acE '^\[desk\] cell title: window [0-9]+ wide \(at least [0-9]+\), mark [0-9]+\.\.[0-9]+ whole, Close anyway [0-9]+\.\.[0-9]+, buttons from [0-9]+; nothing over Close anyway; name (cut|whole)$' <<< "$S")"
    [ "$got" -eq 2 ] || _f "$got of 2 Arabic titles kept the mark whole and nothing over Close anyway" "$2"
    grep -aqE '^\[desk\] cell title: window ([0-9]+) wide \(at least \1\), .*name cut$' <<< "$S" || _f "the narrowest Arabic title is not at its minimum width with the name cut" "$2"
    hasre '^\[desk\] shape sample: a notice with 28 characters in 43 bytes$' "$2" "no sample notice"
    got="$(grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw' <<< "$S")"
    [ "$got" -eq 3 ] || _f "$got of 3 desk verify runs found 0 rows differ" "$2"
    return "$fail"
}

selftest() {
    local D pass=0 bad=0 GREEN
    local SHAPE_HASH_9=0123456789abcdef SHAPE_HASH_18=1123456789abcdef
    D="$(mktemp -d "$T/desk-shape-selftest.XXXXXX")" || return 1
    printf '# test\n0\t00000000000000aa\td8a7\t1:2\n1\t00000000000000bb\td8a8\t3:4\n' > "$D/exp"
    local EXPECTED="$D/exp"
    GREEN="afrisovera> desk shape probe
[desk] shape oracle 0 00000000000000aa
[desk] shape oracle 1 00000000000000bb
[desk] shape: coverage hash cap 9 0123456789abcdef, cap 18 1123456789abcdef
[desk] shape: widths: 8 of 8 drawn as measured
[desk] shape: limits: 300 characters gave 44 cut and width 1290, controls dropped 2
[desk] shape: needed: latin no, yoruba no, arabic yes, hebrew yes, control yes
[desk] shape: arabic 40 characters: cap 9 cold 5000 us, warm 600 us (worst of 20 800 us); cap 18 cold 9000 us, warm 1100 us (worst of 20 1300 us)
[desk] shape: latin 40 characters: cap 9 cold 4000 us, warm 450 us (worst of 20 700 us); cap 18 cold 8000 us, warm 1000 us (worst of 20 1200 us)
[desk] shape: lines shaped 400, controls dropped 3, characters cut 44
[desk] shape: $WANT_A width 30, needed yes
[desk] shape: oracle 1111111111111111
[desk] shape: $WANT_B width 40, needed yes
[desk] shape: oracle 2222222222222222
[desk] shape: $WANT_C width 30, needed yes
[desk] shape: oracle 3333333333333333
[desk] shape: $WANT_D width 25, needed yes
[desk] shape: oracle 4444444444444444
[desk] shape: $WANT_E width 28, needed yes
[desk] shape: oracle 5555555555555555
[desk] cell titlehex: 16 bytes (unsigned, Close anyway offered)
[desk] verify: 0 of 540 rows differ after a full redraw (3 partial frame(s) drawn before it)
[desk] cell title: window 300 wide (at least 259), mark 60..112 whole, Close anyway 120..204, buttons from 222; nothing over Close anyway; name whole
[desk] cell: 37x0 grid after resize
[desk] cell title: window 259 wide (at least 259), mark 45..97 whole, Close anyway 105..189, buttons from 207; nothing over Close anyway; name cut
[desk] shape sample: a notice with 28 characters in 43 bytes
[desk] verify: 0 of 540 rows differ after a full redraw (5 partial frame(s) drawn before it)
[desk] notifications open
[desk] verify: 0 of 540 rows differ after a full redraw (6 partial frame(s) drawn before it)"
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
    case_ panic red "$(printf '%s\nKERNEL PANIC at shape.rs\n' "$GREEN")"
    case_ oracle-differs red "$(printf '%s\n' "$GREEN" | sed 's/oracle 1 00000000000000bb/oracle 1 00000000000000bc/')"
    case_ oracle-missing red "$(printf '%s\n' "$GREEN" | grep -v 'oracle 0 ')"
    case_ hash9-bad red "$(printf '%s\n' "$GREEN" | sed 's/cap 9 0123456789abcdef/cap 9 0123456789abcdee/')"
    case_ hash18-bad red "$(printf '%s\n' "$GREEN" | sed 's/cap 18 1123456789abcdef/cap 18 1123456789abcdee/')"
    case_ width-differs red "$(printf '%s\n' "$GREEN" | sed 's/8 of 8 drawn/7 of 8 drawn/')"
    case_ not-cut red "$(printf '%s\n' "$GREEN" | sed 's/gave 44 cut/gave 0 cut/')"
    case_ control-kept red "$(printf '%s\n' "$GREEN" | sed 's/controls dropped 2$/controls dropped 0/')"
    case_ latin-shaped red "$(printf '%s\n' "$GREEN" | sed 's/latin no, yoruba no/latin yes, yoruba no/')"
    case_ yoruba-shaped red "$(printf '%s\n' "$GREEN" | sed 's/yoruba no/yoruba yes/')"
    case_ speed-missing red "$(printf '%s\n' "$GREEN" | grep -v 'arabic 40')"
    case_ order-wrong red "$(printf '%s\n' "$GREEN" | sed 's/order 0 1 2 6 7 5 4 3/order 0 1 2 7 6 5 4 3/')"
    case_ levels-wrong red "$(printf '%s\n' "$GREEN" | sed 's/levels 2 2 2 1 1 1/levels 2 2 2 2 1 1/')"
    case_ dropped-missing red "$(printf '%s\n' "$GREEN" | sed 's/4 chars, 1 dropped/5 chars, 0 dropped/')"
    case_ title-overlap red "$(printf '%s\n' "$GREEN" | sed '0,/nothing over Close anyway/s/nothing over Close anyway/OVERLAP/')"
    case_ title-mark-cut red "$(printf '%s\n' "$GREEN" | sed 's/mark 45..97 whole/mark 45..97 NOT WHOLE/')"
    case_ title-not-narrowest red "$(printf '%s\n' "$GREEN" | sed 's/window 259 wide (at least 259)/window 270 wide (at least 259)/')"
    case_ titlehex-missing red "$(printf '%s\n' "$GREEN" | grep -v 'titlehex')"
    case_ sample-missing red "$(printf '%s\n' "$GREEN" | grep -v 'shape sample')"
    case_ stale-rows red "$(printf '%s\n' "$GREEN" | sed '0,/verify: 0 of/s/verify: 0 of/verify: 12 of/')"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 23 ]
}

echo "== Arabic joined and right to left (SHAPING_PLAN C1), on both kernels =="
if ! selftest; then echo "DESK SHAPE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK SHAPE SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/desk-shape.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK SHAPE TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5996,server,nowait \
    -qmp tcp:127.0.0.1:5997,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
python3 scripts/shell-demo.py 5996 5997 "$(winpath "$W/arm.png")" "${CMDS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[desk\] (shape|verify|cell title)|panic|PANIC' | grep -av 'shape oracle' | sed 's/^/  arm | /'
grade "$W/arm.log" arm || fails=$((fails + 1))

echo "  --- PC kernel (1920x1080) ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "DESK SHAPE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK SHAPE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1920,yres=1080 \
    -serial "tcp:127.0.0.1:5998,server,nowait" -qmp "tcp:127.0.0.1:5999,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5998 5999 "$(winpath "$W/pc.png")" "screen ui" "${CMDS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] (shape|verify|cell title)|panic|PANIC' | grep -av 'shape oracle' | sed 's/^/  pc  | /'
grade "$W/pc.log" pc || fails=$((fails + 1))

# the coverage hash must be the same bytes on both kernels
a="$(tr -d '\r' < "$W/arm.log" | sed -nE 's/^\[desk\] shape: coverage hash (.*)$/\1/p' | head -1)"
b="$(tr -d '\r' < "$W/pc.log" | sed -nE 's/^\[desk\] shape: coverage hash (.*)$/\1/p' | head -1)"
if [ -z "$a" ] || [ "$a" != "$b" ]; then echo "  FAIL: the coverage hash differs between ARM ($a) and PC ($b)"; fails=$((fails + 1)); fi

if [ -n "${SHAPE_OUT:-}" ]; then mkdir -p "$SHAPE_OUT" && cp "$W"/*.png "$W"/*.log "$SHAPE_OUT"/ 2> /dev/null; fi
if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm.png $W/pc.png"
    echo "DESK SHAPE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK SHAPE TEST FAILED"
