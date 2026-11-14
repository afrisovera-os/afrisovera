#!/usr/bin/env bash
# The desk speaks the user's language (LANGUAGES_PLAN.md in the OS notes, stage 2), on both kernels, and keeps it
# across a reboot.
#
# Part 1, each kernel (the PC on a 1920x1080 screen): `desk lang check` (every language uses the same gaps as its
# English, no combining marks, no banned dashes), `desk lang fit` (no string wider than its place on the desk), the
# English counts, then Twi with Settings, About and Files open and `desk verify` (0 rows differ after partial frames),
# and back to English.
# Part 2, the ARM kernel on one disk image, two boots: `desk lang tw` on the first; the second reads it back from
# afrisovera.conf before the desk draws, and `config` shows the line.
#   scripts/desk-lang-test.sh              # grader selftest, then the boots
#   scripts/desk-lang-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

CMDS=(
    "gui" "desk mode pointer" "desk lang selftest" "desk lang check" "desk lang fit" "desk lang" "desk open settings"
    "desk lang tw" "desk verify" "desk open about" "desk open files" "desk verify" "desk lang"
    "desk cell open" "desk cell titletest averyveryverylongname.wasm" "desk cell resize 40 30" "desk verify"
    "desk cell titlecheck" "desk cell titletest off"
    "desk lang en" "desk verify"
    "desk open settings" "desk lang list" "desk verify" "desk key down" "desk key enter" "desk lang pick es" "desk verify"
    "desk lang en"
)

grade() { # grade <serial log> <kernel name>
    local fail=0 S n d
    _f() { echo "  FAIL ($2): $1"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL ($2): missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    hasre() { printf '%s\n' "$S" | grep -aqE "$1" || _f "$3" "$2"; }
    printf '%s\n' "$S" | grep -aqE 'PANIC|panicked' && _f "the kernel panicked or faulted" "$2"
    printf '%s\n' "$S" | grep -aqE '^\[desk\] lang selftest: FAIL' && _f "a filler or safety selftest case failed" "$2"
    hasre '^\[desk\] lang selftest: ([0-9]+)/\1 checks held$' "$2" "no lang selftest line with every check held"
    printf '%s\n' "$S" | grep -aqE '^\[desk\] cell title: .*(NOT WHOLE|OVERLAP)' && _f "in Twi the unsigned mark is cut or Close anyway is covered" "$2"
    hasre '^\[desk\] cell title: window [0-9]+ wide .*mark [0-9]+\.\.[0-9]+ whole, .*nothing over Close anyway' "$2" "no cell title check in Twi"
    printf '%s\n' "$S" | grep -aqE '^\[desk\] lang check: [a-z]+ ' && _f "lang check found a problem" "$2"
    hasre '^\[desk\] lang check: [0-9]+ language\(s\), [0-9]+ ids, 0 problem\(s\)$' "$2" "no clean lang check line"
    printf '%s\n' "$S" | grep -aqE '^\[desk\] lang fit: [a-z]+ ' && _f "a string is wider than its room" "$2"
    hasre '^\[desk\] lang fit: [0-9]+ language\(s\), 0 too wide$' "$2" "no clean lang fit line"
    n="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] lang check: [0-9]+ language\(s\), ([0-9]+) ids, .*/\1/p' | head -1)"
    [ -n "$n" ] && hasre "^\[desk\] lang: en \(English\), $n strings, 0 drafts, $n checked, 0 in English$" "$2" "English is not all its own strings"
    d="$(printf '%s\n' "$S" | sed -nE 's/^\[desk\] lang: tw \(Twi\), ([0-9]+) drafts, ([0-9]+) in English$/\1/p' | head -1)"
    if [ -z "$d" ]; then _f "no switch to Twi" "$2"; elif [ "$d" -lt 1 ]; then _f "Twi has no strings marked as drafts" "$2"; fi
    hasre '^\[desk\] lang: tw \(Twi\), [0-9]+ strings, [1-9][0-9]* drafts, 0 checked, [0-9]+ in English$' "$2" "no Twi counts line"
    hasre '^\[desk\] lang: en \(English\), 0 drafts, 0 in English$' "$2" "no switch back to English"
    [ "$(printf '%s\n' "$S" | grep -acE '^\[desk\] verify: 0 of [0-9]+ rows differ')" -ge 6 ] || _f "desk verify did not show 0 rows differ six times" "$2"
    # the language list (the designer's spec, 14 Nov): open, the keyboard walks to Twi and picks it, a pick by code
    hasre '^\[desk\] lang: list open$' "$2" "the language list did not open"
    hasre '^\[desk\] lang: list on tw$' "$2" "Down did not move the list to Twi"
    hasre '^\[desk\] lang: list closed$' "$2" "the language list did not close"
    hasre '^\[desk\] lang: es \(Español\), [1-9][0-9]* drafts, [0-9]+ in English$' "$2" "no pick of Spanish from the list"
    [ "$fail" -eq 0 ]
}

grade_reboot() { # grade_reboot <boot 1 log> <boot 2 log>
    local fail=0 A B
    _f() { echo "  FAIL (reboot): $1"; fail=1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    printf '%s\n%s\n' "$A" "$B" | grep -aqE 'PANIC|panicked' && _f "the kernel panicked or faulted"
    printf '%s\n' "$A" | grep -aqE '^\[desk\] lang: tw \(Twi\), [0-9]+ drafts' || _f "boot 1 did not switch to Twi"
    printf '%s\n' "$A" | grep -aqE '^\[desk\] lang: not saved' && _f "boot 1 could not save the language"
    printf '%s\n' "$B" | grep -aqxF '[desk] lang: tw (from afrisovera.conf)' || _f "boot 2 did not read Twi back from afrisovera.conf"
    printf '%s\n' "$B" | grep -aqE '^\[desk\] lang: tw \(Twi\), [0-9]+ strings' || _f "boot 2's desk is not in Twi"
    printf '%s\n' "$B" | grep -aqE '^ *lang=tw$' || _f "boot 2's config has no lang=tw line"
    [ "$fail" -eq 0 ]
}

GREEN="[desk] lang selftest: 30/30 checks held
[desk] lang: list open
[desk] lang: list on tw
[desk] lang: list closed
[desk] lang: es (Español), 46 drafts, 12 in English
[desk] verify: 0 of 800 rows differ after a full redraw (1 partial frame(s) drawn before it)
[desk] verify: 0 of 800 rows differ after a full redraw (1 partial frame(s) drawn before it)
[desk] cell title: window 330 wide (at least 330), mark 190..240 whole, Close anyway 250..334, buttons from 340; nothing over Close anyway; name cut
[desk] verify: 0 of 800 rows differ after a full redraw (4 partial frame(s) drawn before it)
[desk] lang check: 8 language(s), 58 ids, 0 problem(s)
[desk] lang fit: 8 language(s), 0 too wide
[desk] lang: en (English), 58 strings, 0 drafts, 58 checked, 0 in English
[desk] lang: tw (Twi), 53 drafts, 5 in English
[desk] verify: 0 of 800 rows differ after a full redraw (2 partial frame(s) drawn before it)
[desk] verify: 0 of 800 rows differ after a full redraw (3 partial frame(s) drawn before it)
[desk] lang: tw (Twi), 58 strings, 53 drafts, 0 checked, 5 in English
[desk] lang: en (English), 0 drafts, 0 in English
[desk] verify: 0 of 800 rows differ after a full redraw (1 partial frame(s) drawn before it)"
BOOT1="[desk] lang: tw (Twi), 53 drafts, 5 in English"
BOOT2="[desk] lang: tw (from afrisovera.conf)
[cfg] afrisovera.conf:
  lang=tw
[desk] lang: tw (Twi), 58 strings, 53 drafts, 0 checked, 5 in English"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/desk-lang-selftest.XXXXXX")" || return 1
    case_() { # case_ <name> <expected green|red> <log>
        local got
        printf '%s\n' "$3" > "$D/l"
        if grade "$D/l" k > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    rcase() { # rcase <name> <expected> <boot 1> <boot 2>
        local got
        printf '%s\n' "$3" > "$D/a"; printf '%s\n' "$4" > "$D/b"
        if grade_reboot "$D/a" "$D/b" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    case_ green green "$GREEN"
    case_ check-problem red "$(printf '%s\n[desk] lang check: tw SetCores: gaps differ from the English' "$GREEN")"
    case_ check-count red "$(printf '%s\n' "$GREEN" | sed 's/0 problem(s)/1 problem(s)/')"
    case_ too-wide red "$(printf '%s\n[desk] lang fit: tw AboutMotto: 340 px in 310 px' "$GREEN")"
    case_ no-drafts red "$(printf '%s\n' "$GREEN" | sed 's/53 drafts/0 drafts/g')"
    case_ english-gap red "$(printf '%s\n' "$GREEN" | sed 's/58 checked, 0 in English/57 checked, 1 in English/')"
    case_ verify-differs red "$(printf '%s\n' "$GREEN" | sed '0,/0 of 800 rows differ/s//4 of 800 rows differ/')"
    case_ panic red "$(printf '%s\nPANIC at desk.rs' "$GREEN")"
    case_ selftest-fail red "$(printf '%s\n[desk] lang selftest: FAIL a gap past the arguments draws nothing: got "5-x-"' "$GREEN")"
    case_ selftest-short red "$(printf '%s\n' "$GREEN" | sed 's#30/30 checks held#29/30 checks held#')"
    case_ title-cut red "$(printf '%s\n' "$GREEN" | sed 's/ whole, Close/ NOT WHOLE, Close/')"
    case_ list-not-open red "$(printf '%s\n' "$GREEN" | grep -v 'list open')"
    case_ no-spanish red "$(printf '%s\n' "$GREEN" | grep -v 'es (Espa')"
    case_ title-missing red "$(printf '%s\n' "$GREEN" | grep -v 'cell title:')"
    rcase reboot-green green "$BOOT1" "$BOOT2"
    rcase not-read-back red "$BOOT1" "$(printf '%s\n' "$BOOT2" | grep -v 'from afrisovera.conf')"
    rcase no-config-line red "$BOOT1" "$(printf '%s\n' "$BOOT2" | grep -v 'lang=tw')"
    rcase not-saved red "$(printf '%s\n[desk] lang: not saved: the VFS has no slot for it' "$BOOT1")" "$BOOT2"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== the desk in the user's language: catalogue, fit, switch and reboot =="
if ! selftest; then echo "DESK LANG TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "DESK LANG SELFTEST ONLY: no boot run"; exit 0; }

W="$(mktemp -d "$T/desk-lang.XXXXXX")"
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fails=0
. "$(dirname "$0")/lib/afrish-boot.sh"

echo "  --- ARM kernel ---"
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel 2>&1 | tail -1
[ -r "$BIN" ] || { echo "DESK LANG TEST FAILED (harness: $BIN was not built)"; exit 1; }
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:5996,server,nowait \
    -qmp tcp:127.0.0.1:5997,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
python3 scripts/shell-demo.py 5996 5997 "$(winpath "$W/arm.png")" "${CMDS[@]}" > "$W/arm.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/arm.log" | grep -aE '^\[desk\] (lang|verify)|panic|PANIC' | sed 's/^/  arm | /'
grade "$W/arm.log" arm || fails=$((fails + 1))

echo "  --- PC kernel (1920x1080) ---"
command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "DESK LANG TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -1
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "DESK LANG TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH" qemu-system-aarch64 -M virt -m 256M -display none \
    -vga none -device VGA,xres=1920,yres=1080 \
    -serial "tcp:127.0.0.1:5998,server,nowait" -qmp "tcp:127.0.0.1:5999,server,nowait" &
QPID=$!
AFRISH_BOOT_GRACE="${AFRISH_BOOT_GRACE:-10}" python3 scripts/shell-demo.py 5998 5999 "$(winpath "$W/pc.png")" "screen ui" "${CMDS[@]}" > "$W/pc.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/pc.log" | grep -aE '^\[desk\] (lang|verify)|panic|PANIC' | sed 's/^/  pc  | /'
grade "$W/pc.log" pc || fails=$((fails + 1))

echo "  --- ARM kernel, two boots on one disk ---"
IMG="$W/lang-disk.img"
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
boot_disk "$W/boot1.log" "$W/boot1.png" "gui" "desk lang tw" "desk lang"
boot_disk "$W/boot2.log" "$W/boot2.png" "gui" "desk lang" "config"
tr -d '\r' < "$W/boot1.log" | grep -aE '^\[desk\] lang|^\[cfg\]|panic|PANIC' | sed 's/^/  boot 1 | /'
tr -d '\r' < "$W/boot2.log" | grep -aE '^\[desk\] lang|^\[cfg\]|lang=|panic|PANIC' | sed 's/^/  boot 2 | /'
grade_reboot "$W/boot1.log" "$W/boot2.log" || fails=$((fails + 1))

if [ "$fails" -eq 0 ]; then
    echo "  screenshots: $W/arm.png $W/pc.png $W/boot2.png"
    echo "DESK LANG TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "DESK LANG TEST FAILED"
exit 1
