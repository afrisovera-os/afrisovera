#!/usr/bin/env bash
# PC step 4 groundwork (5 Nov 2026): the PS/2 (i8042) keyboard -- how most laptops' built-in
# keyboard reaches the CPU, and QEMU q35's default keyboard.
#
# Boots the PC kernel with no USB keyboard, so QEMU's QMP send-key goes to the PS/2 one.
# Nothing is typed on the serial line: scripts/lib/usb-type.py presses the keys, and the
# shell's answers are read from COM1. Graded: the i8042 is found, a command typed on the
# PS/2 keyboard runs (with shifted characters in it), and `lsps2`, typed the same way,
# reports the scancode and key counters.
#   scripts/pc-ps2kbd-test.sh              # grader selftest, then the QEMU boot
#   scripts/pc-ps2kbd-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
WORD="ps2kbd-OK_5nov"

grade() { # grade <serial capture>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$S" | grep -aqE "$1"; }
    has '^\[ps2\] i8042 keyboard controller \(config 0x[0-9a-f]{2} -> 0x[0-9a-f]{2}\), scancode set 1, polled$' ||
        _f "the i8042 keyboard controller was not found"
    printf '%s\n' "$S" | grep -aqE "^afrisovera> echo $WORD$" || _f "the command typed on the PS/2 keyboard did not reach the shell"
    printf '%s\n' "$S" | grep -aqE "^$WORD$" || _f "the shell did not run the command typed on the PS/2 keyboard"
    local k
    k="$(printf '%s\n' "$S" | grep -aoE '^ps2: i8042 keyboard, polled; [0-9]+ scancodes read, [0-9]+ keys typed$' | tail -1 | awk '{print $8}')"
    if [ -z "$k" ]; then
        _f "lsps2 (typed on the PS/2 keyboard) did not report"
    elif [ "$k" -lt $(( ${#WORD} + 6 )) ]; then
        _f "the keyboard counted only $k keys"
    fi
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsps2
ps2: i8042 keyboard, polled; 53 scancodes read, 22 keys typed
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ps2kbd-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <serial text>
        local got
        printf '%s\n' "$3" > "$D/s"
        if grade "$D/s" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm real-green-2026-11-05 green "$GREEN"
    arm crlf green "$(printf '%s\n' "$GREEN" | sed 's/$/\r/')"
    arm no-controller red "$(printf '%s\n' "$GREEN" | grep -v '^\[ps2\]')"
    arm not-typed red "$(printf '%s\n' "$GREEN" | grep -v "^afrisovera> echo")"
    arm shift-lost red "$(printf '%s\n' "$GREEN" | sed 's/ps2kbd-OK_5nov/ps2kbd-ok-5nov/')"
    arm not-run red "$(printf '%s\n' "$GREEN" | grep -vx "$WORD")"
    arm no-lsps2 red "$(printf '%s\n' "$GREEN" | grep -v '^ps2:')"
    arm too-few-keys red "$(printf '%s\n' "$GREEN" | sed 's/22 keys typed/3 keys typed/')"
    arm panic red "$(printf '%s\n[pc] PANIC: x' "$GREEN")"
    arm empty red ""
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 10 ]
}

echo "== PC PS/2 keyboard (PC step 4 groundwork) =="
if ! selftest; then echo "PC PS2KBD TEST FAILED (grader selftest) -- no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC PS2KBD SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC PS2KBD TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC PS2KBD TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-ps2kbd.XXXXXX")"
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:4934,server,nowait" -qmp "tcp:127.0.0.1:4935,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 4934 4935 "$W/serial.log" "echo $WORD" "lsps2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[ps2\]|^afrisovera> (echo|lsps2)|^ps2:|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC PS2KBD TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC PS2KBD TEST FAILED"
exit 1
