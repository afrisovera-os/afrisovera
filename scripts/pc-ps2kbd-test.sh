#!/usr/bin/env bash
# PC step 4 groundwork (5 Nov 2026): the PS/2 (i8042) keyboard: how most laptops' built-in
# keyboard reaches the CPU, and QEMU q35's default keyboard.
#
# Boots the PC kernel with no USB keyboard, so QEMU's QMP send-key goes to the PS/2 one.
# Nothing is typed on the serial line: scripts/lib/usb-steps.py presses the keys, and the
# shell's answers are read from COM1. Graded: the i8042 is found, a command typed on the
# PS/2 keyboard runs (with shifted characters in it), and `lsps2`, typed the same way,
# reports the scancode and key counters. Since 9 Nov 2026 (HP ENVY field item F5: the arrow
# keys did nothing at the shell), scripts/lib/usb-steps.py then presses Up, Up, Up (already at
# the oldest line: nothing), Down, Up and Enter, which must run `echo` again from the history,
# and `history` must list the four lines with four recalls. Since 9 Nov 2026 (field item F7: no
# scrollback), with the boot log and those lines on the console, Shift+PgUp, Shift+PgUp,
# Shift+PgDn, Shift+PgDn must move the console view 24, 48, 24 lines up and back to the live
# console (notes on COM1 only, the same top line both times at 24 up), and `scrollback` must
# count 4 moves and 1 return.
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
    has() { grep -aqE "$1" <<< "$S"; }
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
    [ "$(printf '%s\n' "$S" | grep -acxE "$WORD")" -ge 2 ] || _f "Up/Down did not bring back and run the echo line"
    has "^    3  echo $WORD$" || _f "history does not list the recalled echo as line 3"
    has '^history: 4 line\(s\) typed, the last 4 kept; 4 recalled with Up/Down$' ||
        _f "history does not count 4 lines and 4 recalls"
    local ups
    ups="$(printf '%s\n' "$S" | grep -aoE '^\[con\] scrollback: [0-9]+ lines up' | awk '{print $3}' | tr '\n' ' ')"
    [ "$ups" = "24 48 24 " ] || _f "Shift+PgUp/PgDn did not move the console view 24, 48, 24 lines up (got: $ups)"
    has '^\[con\] scrollback: back to the live console$' || _f "Shift+PgDn did not bring back the live console"
    [ "$(printf '%s\n' "$S" | grep -aE '^\[con\] scrollback: 24 lines up' | sed 's/.*; top line: //' | sort -u | wc -l)" -eq 1 ] ||
        _f "the view 24 lines up did not show the same top line both times"
    has '^scrollback: [0-9]+ console line\(s\) written, the last [0-9]+ kept; Shift\+PgUp/PgDn moved the view 4 time\(s\), 1 back to live$' ||
        _f "scrollback does not count 4 moves and 1 return to live"
    if has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then _f "the kernel panicked"; fi
    return "$fail"
}

GREEN="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsps2
ps2: i8042 keyboard, polled; 53 scancodes read, 22 keys typed
afrisovera> lsps2$(printf '\b \b%.0s' 1 2 3 4 5)echo $WORD
$WORD
afrisovera> history
    1  echo $WORD
    2  lsps2
    3  echo $WORD
    4  history
history: 4 line(s) typed, the last 4 kept; 4 recalled with Up/Down
afrisovera>
[con] scrollback: 24 lines up of 183 kept; top line: [pc] PS/2 keyboard: i8042 found
[con] scrollback: 48 lines up of 183 kept; top line: [pc] SMP: 1 core online
[con] scrollback: 24 lines up of 183 kept; top line: [pc] PS/2 keyboard: i8042 found
[con] scrollback: back to the live console
scrollback
scrollback: 210 console line(s) written, the last 210 kept; Shift+PgUp/PgDn moved the view 4 time(s), 1 back to live
afrisovera> "
OLD="$(printf '%s\n' "$GREEN" | sed -n '1,/^ps2:/p')"

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
    arm green-2026-11-09 green "$GREEN"
    arm before-history-2026-11-05 red "$OLD"
    arm no-rerun red "$(printf '%s\n' "$GREEN" | awk -v w="$WORD" '$0==w && n++ {next} {print}')"
    arm no-listing red "$(printf '%s\n' "$GREEN" | grep -v '^    3  ')"
    arm wrong-recalls red "$(printf '%s\n' "$GREEN" | sed 's/4 recalled/3 recalled/')"
    arm no-scrollback-2026-11-08 red "$(printf '%s\n' "$GREEN" | grep -v '^\[con\]\|^scrollback')"
    arm no-return red "$(printf '%s\n' "$GREEN" | grep -v 'back to the live')"
    arm top-moved red "$(printf '%s\n' "$GREEN" | awk '/24 lines up/ && n++ {sub(/PS.2/, "USB")} {print}')"
    arm wrong-moves red "$(printf '%s\n' "$GREEN" | sed 's/view 4 time/view 2 time/')"
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
    [ "$bad" -eq 0 ] && [ "$pass" -eq 18 ]
}

echo "== PC PS/2 keyboard (PC step 4 groundwork) =="
if ! selftest; then echo "PC PS2KBD TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC PS2KBD SELFTEST ONLY: no measurement taken"; exit 0; }

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
python3 scripts/lib/usb-steps.py 4934 4935 "$W/serial.log" "$W/shot.png" "type:echo $WORD" "type:lsps2" \
    keys:up keys:up keys:up keys:down keys:up keys:ret wait:3 "type:history" \
    wait:2 keys:shift+pgup wait:1 keys:shift+pgup wait:1 keys:shift+pgdn wait:1 keys:shift+pgdn wait:1 \
    "type:scrollback"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/serial.log" | grep -aE '^\[ps2\]|^\[con\]|^afrisovera> (echo|lsps2|history)|^ps2:|^history:|^scrollback|^    [0-9]  |^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/serial.log"; then
    rm -rf "$W"
    echo "PC PS2KBD TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC PS2KBD TEST FAILED"
exit 1
