#!/usr/bin/env bash
# Each language's numbers, plural forms and clock words, from Unicode CLDR (LANGUAGES_PLAN.md stage 5; the
# architect's OK of 14 Nov), on both kernels.
#
# `desk locale probe` prints, per language, the plural form of 0 1 2 3 5 11 21 101 1000000, two grouped numbers,
# three memory sizes, am and pm and the usual clock. Every line must equal tools/lang/locale_expected.txt, which
# tools/lang/cldr_src.py works out in Python from the same pinned CLDR files. The kernel also checks its plural rules
# against every one of CLDR's own @integer samples in the table. Then `desk lang selftest`, `check` (every plural
# form a language uses is there) and `fit` (every form fits its place), and the desk in French (ARM) and German (PC)
# with Settings and About open, `desk verify` each time. The screenshots are kept.
# Part 2 (stage 5b): two ARM boots on one disk. The clock follows the language (English, CLDR's en-001, 24-hour; Twi
# 12-hour) until the person chooses with `desk clock 24|12`; the choice is kept as clock24= and beats every language.
#   scripts/desk-locale-test.sh              # grader selftest, then the boots
#   scripts/desk-locale-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
EXPECTED="tools/lang/locale_expected.txt"

cmds() { # cmds <language to end in>
    printf '%s\n' "gui" "desk mode pointer" "desk locale probe" "desk lang selftest" "desk lang check" "desk lang fit" \
        "desk lang $1" "desk open settings" "desk verify" "desk open about" "desk verify" "desk open files" "desk verify"
}

grade() { # grade <serial log> <kernel name> <expected file>
    local fail=0 S line
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    [ -s "$3" ] || { echo "  FAIL ($2): no expected lines ($3)"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasre() { grep -aqE "$1" <<< "$S" || _f "$3" "$2"; }
    grep -aqE 'PANIC|panicked' <<< "$S" && _f "the kernel panicked or faulted" "$2"
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        grep -aqxF -e "$line" <<< "$S" || _f "not as CLDR says: ${line%%:*}" "$2"
    done < "$3"
    [ "$(printf '%s\n' "$S" | grep -ac '^\[desk\] locale [a-z]*: plural')" -eq "$(grep -c . "$3")" ] \
        || _f "the probe printed a different number of languages than the expected file" "$2"
    grep -aqE '^\[desk\] locale samples: FAIL' <<< "$S" && _f "a CLDR sample got the wrong plural form" "$2"
    hasre '^\[desk\] locale samples: ([0-9]+)/\1 held; table order ok; plural stand-ins 0$' "$2" \
        "not every CLDR sample held, the table is out of order, or a plural form was missing"
    grep -aqE '^\[desk\] locale samples: 0/' <<< "$S" && _f "no CLDR samples in the table" "$2"
    grep -aqE '^\[desk\] lang selftest: FAIL' <<< "$S" && _f "a filler selftest case failed" "$2"
    hasre '^\[desk\] lang selftest: ([0-9]+)/\1 checks held$' "$2" "no lang selftest line with every check held"
    grep -aqE '^\[desk\] lang check: [a-z]+ ' <<< "$S" && _f "lang check found a problem" "$2"
    hasre '^\[desk\] lang check: [0-9]+ language\(s\), [0-9]+ ids, 0 problem\(s\)$' "$2" "no clean lang check line"
    grep -aqE '^\[desk\] lang fit: [a-z]+ ' <<< "$S" && _f "a string is wider than its room" "$2"
    hasre '^\[desk\] lang fit: [0-9]+ language\(s\), 0 too wide$' "$2" "no clean lang fit line"
    [ "$(printf '%s\n' "$S" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ')" -ge 3 ] \
        || _f "desk verify did not show 0 rows differ three times" "$2"
    [ "$fail" -eq 0 ]
}

# Part 2 (stage 5b), the ARM kernel on one disk, two boots: the clock follows the language until the person chooses
# one, and the choice is kept in afrisovera.conf and beats every language after a reboot.
BOOT1_CMDS=("gui" "desk clock" "desk lang tw" "desk clock" "desk clock 24")
BOOT2_CMDS=("gui" "desk clock" "desk lang fr" "desk clock" "desk clock 12" "config")
grade_clock() { # grade_clock <boot 1 log> <boot 2 log>
    local fail=0 A B
    _f() { echo "  FAIL (clock): $1"; fail=1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    grep -aqE 'PANIC|panicked' <<< "$A"$'\n'"$B" && _f "the kernel panicked or faulted"
    grep -aqxF "[desk] clock: 24-hour, the language's default (24-hour)" <<< "$A" \
        || _f "boot 1: English (en-001) did not start on its 24-hour default"
    grep -aqxF "[desk] clock: 12-hour, the language's default (12-hour)" <<< "$A" \
        || _f "boot 1: Twi did not bring its 12-hour default"
    grep -aqxF "[desk] clock: 24-hour, the person's choice" <<< "$A" || _f "boot 1: the choice was not taken"
    grep -aqE '^\[desk\] clock: not saved' <<< "$A"$'\n'"$B" && _f "the clock could not be saved"
    [ "$(printf '%s\n' "$B" | grep -acxF "[desk] clock: 24-hour, the person's choice")" -ge 2 ] \
        || _f "boot 2: the saved 24-hour choice did not hold, before and after switching to French"
    grep -aqxF "[desk] clock: 12-hour, the person's choice" <<< "$B" || _f "boot 2: the new choice was not taken"
    grep -aqE '^ *clock24=0$' <<< "$B" || _f "boot 2: the config has no clock24=0 line after choosing 12-hour"
    [ "$fail" -eq 0 ]
}

selftest() {
    local D pass=0 bad=0 GREEN
    D="$(mktemp -d "$T/desk-locale-selftest.XXXXXX")" || return 1
    printf '%s\n' \
        "[desk] locale en: plural 0=other 1=one 2=other; group 1000=1,000 1234567=1,234,567; am=AM pm=PM; clock 12" \
        "[desk] locale fr: plural 0=one 1=one 1000000=many; group 1000=1$(printf '\302\240')000; am=AM pm=PM; clock 24" \
        > "$D/exp"
    GREEN="$(cat "$D/exp")
[desk] locale samples: 312/312 held; table order ok; plural stand-ins 0
[desk] lang selftest: 40/40 checks held
[desk] lang check: 8 language(s), 55 ids, 0 problem(s)
[desk] lang fit: 8 language(s), 0 too wide
[desk] lang: fr (Français), 41 drafts, 14 in English
[desk] verify: 0 of 800 rows differ after a full redraw (1 partial frame(s) drawn before it)
[desk] verify: 0 of 800 rows differ after a full redraw (1 partial frame(s) drawn before it)
[desk] verify: 0 of 800 rows differ after a full redraw (2 partial frame(s) drawn before it)"
    case_() { # case_ <name> <expected green|red> <log>
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k "$D/exp" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ wrong-plural red "$(printf '%s\n' "$GREEN" | sed '0,/1000000=many/s//1000000=other/')"
    case_ wrong-grouping red "$(printf '%s\n' "$GREEN" | sed '0,/1234567=1,234,567/s//1234567=1234567/')"
    case_ plain-space-for-nbsp red "$(printf '%s\n' "$GREEN" | sed "s/1$(printf '\302\240')000/1 000/")"
    case_ language-missing red "$(printf '%s\n' "$GREEN" | grep -v 'locale fr:')"
    case_ extra-language red "$(printf '%s\n[desk] locale xx: plural 0=other' "$GREEN")"
    case_ sample-fail red "$(printf '%s\n[desk] locale samples: FAIL fr 0: CLDR says one, the desk says other' "$GREEN")"
    case_ samples-short red "$(printf '%s\n' "$GREEN" | sed 's#312/312 held#311/312 held#')"
    case_ no-samples red "$(printf '%s\n' "$GREEN" | sed 's#312/312 held#0/0 held#')"
    case_ order-wrong red "$(printf '%s\n' "$GREEN" | sed 's/table order ok/table order WRONG/')"
    case_ stand-ins red "$(printf '%s\n' "$GREEN" | sed 's/plural stand-ins 0/plural stand-ins 2/')"
    case_ check-problem red "$(printf '%s\n[desk] lang check: fr SetCores: a plural form the language uses is missing' "$GREEN")"
    case_ too-wide red "$(printf '%s\n[desk] lang fit: de SetCores: 360 px in 352 px' "$GREEN")"
    case_ selftest-short red "$(printf '%s\n' "$GREEN" | sed 's#40/40 checks held#39/40 checks held#')"
    case_ verify-differs red "$(printf '%s\n' "$GREEN" | sed '0,/0 of 800 rows differ/s//4 of 800 rows differ/')"
    case_ panic red "$(printf '%s\nPANIC at locale.rs' "$GREEN")"
    local C1 C2
    C1="[desk] clock: 24-hour, the language's default (24-hour)
[desk] lang: tw (Twi), 53 drafts, 9 in English
[desk] clock: 12-hour, the language's default (12-hour)
[desk] clock: 24-hour, the person's choice"
    C2="[desk] lang: tw (from afrisovera.conf)
[desk] clock: 24-hour, the person's choice
[desk] lang: fr (Français), 41 drafts, 21 in English
[desk] clock: 24-hour, the person's choice
[desk] clock: 12-hour, the person's choice
[cfg] afrisovera.conf:
  lang=fr
  clock24=0"
    ccase() { # ccase <name> <expected> <boot 1> <boot 2>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade_clock "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    ccase clock-green green "$C1" "$C2"
    ccase english-12 red "$(printf '%s\n' "$C1" | sed '1s/24-hour, the language/12-hour, the language/')" "$C2"
    ccase choice-lost red "$C1" "$(printf '%s\n' "$C2" | sed '2s/24-hour, the person/12-hour, the language/')"
    ccase language-beats-choice red "$C1" "$(printf '%s\n' "$C2" | sed "4s/.*/[desk] clock: 24-hour, the language's default (24-hour)/")"
    ccase no-config-line red "$C1" "$(printf '%s\n' "$C2" | grep -v 'clock24=0')"
    ccase not-saved red "$(printf '%s\n[desk] clock: not saved: the VFS has no slot for it' "$C1")" "$C2"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 22 ]
}

echo "== each language's numbers, plural forms and clock, from Unicode CLDR =="
if ! selftest; then echo "DESK LOCALE TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK LOCALE SELFTEST ONLY: no boot run"; exit 0; }
[ -s "$EXPECTED" ] || { echo "DESK LOCALE TEST FAILED (harness: $EXPECTED missing)"; exit 1; }

W="$(mktemp -d "$T/desk-locale.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0
. "$(dirname "$0")/lib/afrish-boot.sh"
mapfile -t ARM_CMDS < <(cmds fr)
mapfile -t PC_CMDS < <(cmds de)

echo "  --- ARM kernel, ending in French ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK LOCALE TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5996,server,nowait \
    -qmp tcp:127.0.0.1:5997,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 5996 5997 "$(winpath "$W/arm-fr.png")" "${ARM_CMDS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[desk\] (locale|lang (check|fit|selftest)|verify)|panic|PANIC' | sed 's/^/  arm | /'
grade "$W/arm.log" arm "$EXPECTED" || fails=$((fails + 1))

echo "  --- PC kernel (1920x1080), ending in German ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "DESK LOCALE TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK LOCALE TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1920,yres=1080 \
    -serial "tcp:127.0.0.1:5998,server,nowait" -qmp "tcp:127.0.0.1:5999,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5998 5999 "$(winpath "$W/pc-de.png")" "screen ui" "${PC_CMDS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] (locale|lang (check|fit|selftest)|verify)|panic|PANIC' | sed 's/^/  pc  | /'
grade "$W/pc.log" pc "$EXPECTED" || fails=$((fails + 1))

echo "  --- ARM kernel, two boots on one disk: the clock's setting ---"
IMG="$W/clock-disk.img"
head -c 4194304 /dev/zero > "$IMG"
boot_disk() { # boot_disk <log> <png> <command...>
    local log="$1" png="$2"; shift 2
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none \
        -drive "file=$IMG,if=none,format=raw,id=d0" -device virtio-blk-device,drive=d0 \
        -serial tcp:127.0.0.1:5996,server,nowait \
        -qmp tcp:127.0.0.1:5997,server,nowait -kernel "$BIN" &
    local qpid=$!
    afrish_wait_boot
    python3 scripts/shell-demo.py 5996 5997 "$(winpath "$png")" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
boot_disk "$W/clock1.log" "$W/clock1.png" "${BOOT1_CMDS[@]}"
boot_disk "$W/clock2.log" "$W/clock2.png" "${BOOT2_CMDS[@]}"
tr -d '\r' < "$W/clock1.log" | grep -aE '^\[desk\] (clock|lang: tw)|panic|PANIC' | sed 's/^/  boot 1 | /'
tr -d '\r' < "$W/clock2.log" | grep -aE '^\[desk\] (clock|lang)|clock24=|panic|PANIC' | sed 's/^/  boot 2 | /'
grade_clock "$W/clock1.log" "$W/clock2.log" || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm-fr.png $W/pc-de.png"
    echo "DESK LOCALE TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK LOCALE TEST FAILED"
exit 1
