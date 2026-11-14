#!/usr/bin/env bash
# The desk's Terminal scrolls back, a pager and output to a file (afriui.md section 28, 14 Nov 2026: the owner's
# `hwscan` on his HP ran 72 devices past a 48-line Terminal with no way back), on both kernels.
#
# Each kernel shows the desk with the Terminal open and runs `help` (about 600 lines) into it, then:
#   ARM (serial only): `desk term up` twice and `desk term wheel -2`, each followed by `desk term status`;
#   PC: Shift+PgUp and three wheel notches through QEMU's PS/2 keyboard and mouse (QMP), as the HP has, then Enter
#       on that keyboard, which must bring the view back to the bottom;
#   `more help` with Space then q, `help | more` with q, `help > helpout` and `ls -l`;
#   the architect's refusals (14 Nov): `more appget ...` (a command that asks for a key), `help > afrisovera.conf` (the
#   system's file), `help > helpout` again (an existing file), `more more help` (one at a time); helpout unchanged;
#   `desk verify` (0 rows differ) while scrolled back and at the end;
#   then the text console: `clear`, a word typed with a mistake and Backspace (a screenshot, by eye: the console
#   drew Backspace as a box before this change).
# Graded from each `[desk] term:` status line (R = the rows the Terminal shows):
#   the view at the bottom is 0;
#   ARM: the first page up is R/2, the second R/2 + 2 more (the status line and the next command's echo also come in
#        while scrolled back, and the view keeps its text in place), the wheel 6 + 2 more;
#   PC: Shift+PgUp gives R/2 + 1 (the next command's echo), three notches 9 + 2 more, Enter 0;
#   the pager stops after 2(R - 1) lines (two pages) and after R - 1 (one); the file holds 16384 bytes, cut.
#   scripts/term-scroll-test.sh              # grader selftest, then the two boots
#   scripts/term-scroll-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"

HEAD=("gui" "desk mode pointer" "desk open terminal" "help" "desk term status")
TAIL=("desk term bottom" "&more help" "@More:" "~1" "!more" ":20" "~2" ":71" "=" "&help | more" "@More:" ":71" "="
      "help > helpout" "more appget 10.0.2.2 80 x.wasm 00" "help > afrisovera.conf" "help > helpout" "more more help"
      "ls -l" "desk verify" "clear" ":6563686f206162637878" ":7f" ":79" "~1" "!console-backspace" ":0d" "~1")
ARM_MID=("desk term up" "desk term up" "desk term wheel -2" "desk verify" "!scrolled")
PC_MID=("%shift-pgup" "desk term status" "^-3" "desk term status" "desk verify" "!scrolled" "%ret" "desk term status")

grade() { # grade <serial log> <kernel name> <arm|pc>
    local fail=0 S v r
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    # the views, in order, and the rows shown
    v=($(printf '%s\n' "$S" | sed -nE 's/^\[desk\] term: [0-9]+ lines kept, [0-9]+ rows shown, view ([0-9]+) line\(s\) back; top row: .*$/\1/p'))
    r="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] term: [0-9]+ lines kept, ([0-9]+) rows shown, view .*$/\1/p' | head -1)"
    if [ -z "$r" ]; then _f "no desk term status line" "$2"; return 1; fi
    local h=$((r / 2)) p=$((r - 1))
    if [ "$3" = arm ]; then
        [ "${#v[@]}" -ge 5 ] || { _f "${#v[@]} status lines, not 5" "$2"; return 1; }
        [ "${v[0]}" -eq 0 ] || _f "the view at the bottom is ${v[0]}, not 0" "$2"
        [ "${v[1]}" -eq "$h" ] || _f "one page up gave ${v[1]}, not $h" "$2"
        [ "${v[2]}" -eq $((v[1] + h + 2)) ] || _f "a second page up gave ${v[2]}, not $((v[1] + h + 2))" "$2"
        [ "${v[3]}" -eq $((v[2] + 6 + 2)) ] || _f "two wheel notches gave ${v[3]}, not $((v[2] + 8))" "$2"
        [ "${v[4]}" -eq 0 ] || _f "desk term bottom gave ${v[4]}, not 0" "$2"
    else
        [ "${#v[@]}" -ge 4 ] || { _f "${#v[@]} status lines, not 4" "$2"; return 1; }
        [ "${v[0]}" -eq 0 ] || _f "the view at the bottom is ${v[0]}, not 0" "$2"
        [ "${v[1]}" -eq $((h + 1)) ] || _f "Shift+PgUp on the PS/2 keyboard gave ${v[1]}, not $((h + 1))" "$2"
        [ "${v[2]}" -eq $((v[1] + 9 + 2)) ] || _f "three wheel notches on the PS/2 mouse gave ${v[2]}, not $((v[1] + 11))" "$2"
        [ "${v[3]}" -eq 0 ] || _f "Enter on the PS/2 keyboard left the view at ${v[3]}, not 0" "$2"
    fi
    grep -aqF "[more] stopped after $((2 * p)) line(s)" <<< "$S" || _f "more help did not stop after two pages ($((2 * p)) lines)" "$2"
    grep -aqF "[more] stopped after $p line(s)" <<< "$S" || _f "help | more did not stop after one page ($p lines)" "$2"
    grep -aqF "[shell] 16384 bytes of 'help' saved to helpout (cut: the output was longer)" <<< "$S" || _f "help > helpout did not save 16384 bytes" "$2"
    grep -aqE '^ *16384 .*helpout$' <<< "$S" || _f "ls -l does not list helpout at 16384 bytes" "$2"
    grep -aqF "more: 'appget' asks for a key, so it cannot run inside more or >" <<< "$S" || _f "more appget was not refused" "$2"
    grep -aqF "[shell] afrisovera.conf is the system's file; not replaced" <<< "$S" || _f "> afrisovera.conf was not refused" "$2"
    grep -aqF "[shell] helpout exists; not replaced (choose another name)" <<< "$S" || _f "> an existing file was not refused" "$2"
    grep -aqF "more: one more or > at a time" <<< "$S" || _f "more more help did not say one at a time" "$2"
    [ "$(printf '%s\n' "$S" | grep -acF "saved to helpout")" -eq 1 ] || _f "helpout was saved more than once" "$2"
    grep -aqF "saved to afrisovera.conf" <<< "$S" && _f "the system's file was written" "$2"
    local n
    n="$(printf '%s\n' "$S" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ after a full redraw')"
    [ "$n" -ge 2 ] || _f "$n desk verify runs found 0 rows differ (2 wanted: scrolled back and at the end)" "$2"
    grep -aqE '^\[desk\] verify: [1-9][0-9]* of ' <<< "$S" && _f "a desk verify found rows that differ" "$2"
    return "$fail"
}

ST="[desk] term: 712 lines kept, 16 rows shown, view %d line(s) back; top row:   ping            ICMP echo"
ARM_GREEN="$(printf "$ST\n" 0 8 18 26)
[desk] verify: 0 of 480 rows differ after a full redraw (7 partial frame(s) drawn before it)
$(printf "$ST\n" 0)
More: Space next page, Enter next line, q quit
[more] stopped after 30 line(s)
More: Space next page, Enter next line, q quit
[more] stopped after 15 line(s)
afrisovera> help > helpout
[shell] 16384 bytes of 'help' saved to helpout (cut: the output was longer)
more: 'appget' asks for a key, so it cannot run inside more or >
[shell] afrisovera.conf is the system's file; not replaced
[shell] helpout exists; not replaced (choose another name)
more: one more or > at a time
 16384  2026-11-14 01:03:59  helpout
[desk] verify: 0 of 480 rows differ after a full redraw (18 partial frame(s) drawn before it)"
PC_GREEN="$(printf "$ST\n" 0 9 20)
[desk] verify: 0 of 800 rows differ after a full redraw (7 partial frame(s) drawn before it)
$(printf "$ST\n" 0)
More: Space next page, Enter next line, q quit
[more] stopped after 30 line(s)
[more] stopped after 15 line(s)
[shell] 16384 bytes of 'help' saved to helpout (cut: the output was longer)
more: 'appget' asks for a key, so it cannot run inside more or >
[shell] afrisovera.conf is the system's file; not replaced
[shell] helpout exists; not replaced (choose another name)
more: one more or > at a time
 16384  2026-11-14 01:04:39  helpout
[desk] verify: 0 of 800 rows differ after a full redraw (22 partial frame(s) drawn before it)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/term-scroll-selftest.XXXXXX")" || return 1
    case_() { # case_ <name> <expected green|red> <arm|pc> <log>
        local got
        printf '%s\n' "$4" > "$D/l"
        if grade "$D/l" k "$3" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ arm-green green arm "$ARM_GREEN"
    case_ pc-green green pc "$PC_GREEN"
    case_ crlf green pc "$(printf '%s\n' "$PC_GREEN" | sed 's/$/\r/')"
    case_ empty red arm ""
    case_ panic red pc "$(printf '%s\nKERNEL PANIC at desk.rs\n' "$PC_GREEN")"
    case_ arm-page-wrong red arm "$(printf '%s\n' "$ARM_GREEN" | sed '0,/view 8 line/s/view 8 line/view 7 line/')"
    case_ arm-second-page red arm "$(printf '%s\n' "$ARM_GREEN" | sed 's/view 18 line/view 16 line/')"
    case_ arm-wheel red arm "$(printf '%s\n' "$ARM_GREEN" | sed 's/view 26 line/view 18 line/')"
    case_ arm-no-bottom red arm "$(printf '%s\n' "$ARM_GREEN" | sed '$!{/view 0 line/{x;/./{x;s/view 0 line/view 3 line/;b};x;h}}')"
    case_ pc-ps2-page red pc "$(printf '%s\n' "$PC_GREEN" | sed 's/view 9 line/view 1 line/')"
    case_ pc-ps2-wheel red pc "$(printf '%s\n' "$PC_GREEN" | sed 's/view 20 line/view 9 line/')"
    case_ pc-enter-stays red pc "$(printf '%s\n' "$PC_GREEN" | awk '/view 0 line/{n++; if(n==2){sub(/view 0 line/,"view 20 line")}} {print}')"
    case_ pc-status-missing red pc "$(printf '%s\n' "$PC_GREEN" | grep -v 'view 20 line')"
    case_ more-runs-on red arm "$(printf '%s\n' "$ARM_GREEN" | sed 's/stopped after 30 line/stopped after 45 line/')"
    case_ pipe-more-runs-on red pc "$(printf '%s\n' "$PC_GREEN" | sed 's/stopped after 15 line/stopped after 600 line/')"
    case_ file-missing red arm "$(printf '%s\n' "$ARM_GREEN" | grep -v 'saved to helpout')"
    case_ file-not-listed red pc "$(printf '%s\n' "$PC_GREEN" | grep -v '  helpout$')"
    case_ stale-rows red pc "$(printf '%s\n' "$PC_GREEN" | sed '0,/verify: 0 of/s/verify: 0 of/verify: 14 of/')"
    case_ verify-missing red arm "$(printf '%s\n' "$ARM_GREEN" | grep -v 'verify:')"
    case_ asker-ran red arm "$(printf '%s\n' "$ARM_GREEN" | grep -v "asks for a key")"
    case_ conf-written red pc "$(printf '%s\n' "$PC_GREEN" | sed "s/afrisovera.conf is the system's file; not replaced/16384 bytes of 'help' saved to afrisovera.conf/")"
    case_ existing-replaced red pc "$(printf '%s\n' "$PC_GREEN" | sed 's/helpout exists; not replaced (choose another name)/16384 bytes of '"'"'help'"'"' saved to helpout/')"
    case_ nested-runs red arm "$(printf '%s\n' "$ARM_GREEN" | grep -v 'one more or > at a time')"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 23 ]
}

echo "== the Terminal scrolls back, a pager and output to a file, on both kernels =="
if ! selftest; then echo "TERM SCROLL TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "TERM SCROLL SELFTEST ONLY: no measurement taken"; exit 0; }

W="$(mktemp -d "$T/term-scroll.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0
show() { tr -d '\r' < "$1" | grep -aE '^\[desk\] (term|verify)|^\[more\]|^\[shell\]|panic|PANIC' | sed "s/^/  $2 | /"; }

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "TERM SCROLL TEST FAILED (harness: $BIN was not built)"; exit 1; }
mkdir -p "$W/arm"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:6106,server,nowait \
    -qmp tcp:127.0.0.1:6107,server,nowait -kernel "$BIN" &
QPID=$!
$PY scripts/lib/cell-keys.py 6106 6107 "$(winpath "$W/arm")" "${HEAD[@]}" "${ARM_MID[@]}" "${TAIL[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/arm.log" arm
grade "$W/arm.log" arm arm || fails=$((fails + 1))

echo "  --- PC kernel (PS/2 keyboard and mouse, as the HP) ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "TERM SCROLL TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
mkdir -p "$W/pc"
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1280,yres=800 \
    -serial "tcp:127.0.0.1:6108,server,nowait" -qmp "tcp:127.0.0.1:6109,server,nowait" &
QPID=$!
$PY scripts/lib/cell-keys.py 6108 6109 "$(winpath "$W/pc")" "screen ui" "${HEAD[@]}" "${PC_MID[@]}" "${TAIL[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
show "$W/pc.log" "pc "
grade "$W/pc.log" pc pc || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm/scrolled.png $W/arm/more.png $W/pc/scrolled.png $W/pc/more.png $W/pc/console-backspace.png"
    echo "TERM SCROLL TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "TERM SCROLL TEST FAILED"
exit 1
