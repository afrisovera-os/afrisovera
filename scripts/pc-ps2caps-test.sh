#!/usr/bin/env bash
# PC track step 3 (13 Nov 2026): the PS/2 keyboard's Caps Lock light, and one Caps Lock state
# for the PS/2 keyboard and a USB one.
#
# A laptop's own keyboard is PS/2 (i8042). Its Caps Lock key turned letters to capitals but its
# light stayed dark, and a USB keyboard kept a Caps Lock of its own. Now the kernel sends the
# keyboard 0xED and the LED byte (each answered 0xFA) from the idle loop, and Caps Lock is one
# state: either keyboard's key turns it and both lights follow.
# Boot 1, no USB keyboard (QMP send-key goes to the PS/2 one): "echo ab", Caps Lock, "cd1E",
# Caps Lock, "fg" and Enter, then lsps2. Graded: the shell got "echo abCD1efg", and lsps2 says
# Caps Lock is off again, the light set twice (on, then off) and nothing refused.
# Boot 2, a USB keyboard too (send-key now goes to it): Caps Lock on the USB keyboard, then
# "LSPS2" and "LSUSB" typed with Shift, which with Caps Lock on must give small letters. Graded:
# both commands ran, and both lsps2 and lsusb say Caps Lock is on, each light set once, nothing
# refused: the USB key turned the PS/2 keyboard's light too.
#   scripts/pc-ps2caps-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-ps2caps-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
WORD="abCD1efg"

clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g'; }
bad_line() { grep -aE 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' <<< "$1" | head -1; }

grade1() { # grade1 <serial capture of boot 1, PS/2 only>
    local fail=0 S l
    _f() { echo "  FAIL (PS/2 alone): $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture of boot 1"; return 1; }
    S="$(clean "$1")"
    grep -aqE '^\[ps2\] i8042 keyboard controller \(config 0x[0-9a-f]{2} -> 0x[0-9a-f]{2}\), scancode set 1, polled$' <<< "$S" ||
        _f "the i8042 keyboard controller was not found"
    grep -aqE "^afrisovera> echo $WORD$" <<< "$S" || _f "the line typed with Caps Lock did not reach the shell as echo $WORD"
    grep -aqE "^$WORD$" <<< "$S" || _f "the shell did not print $WORD"
    l="$(grep -aoE '^ps2: keyboard caps lock (on|off), its light set [0-9]+ times, [0-9]+ refused$' <<< "$S" | tail -1)"
    if [ -z "$l" ]; then
        _f "lsps2 did not report Caps Lock"
    else
        grep -q 'caps lock off' <<< "$l" || _f "Caps Lock was left on: $l"
        [ "$(awk '{print $9}' <<< "$l")" -eq 2 ] || _f "the light was not set twice (on, then off): $l"
        [ "$(awk '{print $11}' <<< "$l")" -eq 0 ] || _f "the keyboard refused light commands: $l"
    fi
    l="$(grep -aoE '^ps2: light commands: longest [0-9]+ us with interrupts off \(bound 20 ms\)$' <<< "$S" | tail -1)"
    if [ -z "$l" ]; then _f "lsps2 did not report the light commands' longest time with interrupts off"
    else [ "$(awk '{print $5}' <<< "$l")" -le 20000 ] || _f "a light command kept interrupts off past the 20 ms bound: $l"; fi
    [ -z "$(bad_line "$S")" ] || _f "$(bad_line "$S")"
    return "$fail"
}

grade3() { # grade3 <serial capture of boot 3: the test rig's silent keyboard (fw_cfg opt/afrisovera/ps2-led-mute)>
    local fail=0 S l a b
    _f() { echo "  FAIL (silent keyboard): $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture of boot 3"; return 1; }
    S="$(clean "$1")"
    grep -aqE "^$WORD$" <<< "$S" || _f "Caps Lock did not work with a keyboard that ignores light commands (no $WORD)"
    [ "$(grep -acE '^\[ps2\] keyboard does not answer light commands; Caps Lock still works, its light is left alone$' <<< "$S")" -eq 1 ] ||
        _f "the give-up line was not printed exactly once"
    grep -aqE '^ps2: keyboard caps lock off, its light set 0 times, 2 refused$' <<< "$S" || _f "lsps2 does not show 2 refused light commands and none set"
    l="$(grep -aoE '^ps2: light commands: longest [0-9]+ us with interrupts off \(bound 20 ms\), given up for this boot$' <<< "$S" | tail -1)"
    if [ -z "$l" ]; then _f "lsps2 does not show the light given up for the boot"
    else [ "$(awk '{print $5}' <<< "$l")" -le 25000 ] || _f "interrupts were off past the bound waiting for the silent keyboard: $l"; fi
    a="$(grep -aoE '^[0-9]+ ticks  \(~' <<< "$S" | sed -n 1p | cut -d' ' -f1)"
    b="$(grep -aoE '^[0-9]+ ticks  \(~' <<< "$S" | sed -n 2p | cut -d' ' -f1)"
    { [ -n "$a" ] && [ -n "$b" ] && [ "$b" -gt "$a" ]; } || _f "the timer did not tick between the two uptimes ('$a', '$b')"
    [ -z "$(bad_line "$S")" ] || _f "$(bad_line "$S")"
    return "$fail"
}

grade2() { # grade2 <serial capture of boot 2, PS/2 and USB>
    local fail=0 S l
    _f() { echo "  FAIL (with USB): $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial capture of boot 2"; return 1; }
    S="$(clean "$1")"
    grep -aqE '^\[xhci\] port [0-9]+: USB keyboard [0-9a-f]{4}:[0-9a-f]{4} \((full|low|high) speed\), slot [0-9]+' <<< "$S" ||
        _f "no USB keyboard was enumerated"
    grep -aqE '^afrisovera> lsps2$' <<< "$S" || _f "Shift with Caps Lock on did not give small letters (no lsps2 line)"
    grep -aqE '^afrisovera> lsusb$' <<< "$S" || _f "Shift with Caps Lock on did not give small letters (no lsusb line)"
    l="$(grep -aoE '^ps2: keyboard caps lock (on|off), its light set [0-9]+ times, [0-9]+ refused$' <<< "$S" | tail -1)"
    if [ -z "$l" ]; then
        _f "lsps2 did not report Caps Lock"
    else
        grep -q 'caps lock on' <<< "$l" || _f "the USB Caps Lock key did not turn the shared state on: $l"
        [ "$(awk '{print $9}' <<< "$l")" -eq 1 ] || _f "the PS/2 light did not follow the USB key once: $l"
        [ "$(awk '{print $11}' <<< "$l")" -eq 0 ] || _f "the PS/2 keyboard refused light commands: $l"
    fi
    l="$(grep -aoE '^  keyboard: caps lock (on|off), its light set [0-9]+ times, [0-9]+ refused$' <<< "$S" | tail -1)"
    if [ -z "$l" ]; then
        _f "lsusb did not report Caps Lock"
    else
        grep -q 'caps lock on' <<< "$l" || _f "lsusb does not see the shared Caps Lock: $l"
        [ "$(awk '{print $8}' <<< "$l")" -eq 1 ] || _f "the USB light was not set once: $l"
        [ "$(awk '{print $10}' <<< "$l")" -eq 0 ] || _f "the USB keyboard refused light requests: $l"
    fi
    [ -z "$(bad_line "$S")" ] || _f "$(bad_line "$S")"
    grep -aqE '^\[xhci\] .*(failed|timed out|did not)' <<< "$S" &&
        _f "$(grep -aE '^\[xhci\] .*(failed|timed out|did not)' <<< "$S" | head -1)"
    return "$fail"
}

G1="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
AfriSh ready. Type 'help' for commands.
afrisovera> echo $WORD
$WORD
afrisovera> lsps2
ps2: i8042 keyboard, polled; 61 scancodes read, 20 keys typed
ps2: keyboard caps lock off, its light set 2 times, 0 refused
ps2: light commands: longest 431 us with interrupts off (bound 20 ms)
ps2: no PS/2 mouse or touchpad
afrisovera> "
G2="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
[xhci] port 1: USB keyboard 0627:0001 (full speed), slot 1, interrupt-IN endpoint 1 every 8000 us
AfriSh ready. Type 'help' for commands.
afrisovera> lsps2
ps2: i8042 keyboard, polled; 0 scancodes read, 0 keys typed
ps2: keyboard caps lock on, its light set 1 times, 0 refused
afrisovera> lsusb
usb: xHCI controller 00:03.0, 8 ports, 1 connected
  keyboard: slot 1, 30 reports, 12 keys typed
  keyboard: caps lock on, its light set 1 times, 0 refused
afrisovera> "

G3="[ps2] i8042 keyboard controller (config 0x67 -> 0x64), scancode set 1, polled
AfriSh ready. Type 'help' for commands.
[ps2] keyboard does not answer light commands; Caps Lock still works, its light is left alone
afrisovera> echo $WORD
$WORD
afrisovera> uptime
512 ticks  (~5120 ms, ~5 s)
afrisovera> uptime
547 ticks  (~5470 ms, ~5 s)
afrisovera> lsps2
ps2: i8042 keyboard, polled; 61 scancodes read, 20 keys typed
ps2: keyboard caps lock off, its light set 0 times, 2 refused
ps2: light commands: longest 20011 us with interrupts off (bound 20 ms), given up for this boot
afrisovera> "

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ps2caps-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <boot 1 text> <boot 2 text>
        local got
        printf '%s\n' "$3" > "$D/s1"
        printf '%s\n' "$4" > "$D/s2"
        if grade1 "$D/s1" > "$D/log" 2>&1 && grade2 "$D/s2" >> "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green green "$G1" "$G2"
    arm crlf green "$(printf '%s\n' "$G1" | sed 's/$/\r/')" "$(printf '%s\n' "$G2" | sed 's/$/\r/')"
    arm before-2026-11-13 red "$(printf '%s\n' "$G1" | grep -v 'caps lock')" "$(printf '%s\n' "$G2" | grep -v '^ps2: keyboard caps')"
    arm caps-ignored red "$(printf '%s\n' "$G1" | sed 's/abCD1efg/abcd1Efg/')" "$G2"
    arm shift-not-inverted red "$(printf '%s\n' "$G1" | sed 's/abCD1efg/abCD1Efg/')" "$G2"
    arm left-on red "$(printf '%s\n' "$G1" | sed 's/caps lock off/caps lock on/')" "$G2"
    arm light-once red "$(printf '%s\n' "$G1" | sed 's/set 2 times/set 1 times/')" "$G2"
    arm ps2-refused red "$(printf '%s\n' "$G1" | sed 's/, 0 refused/, 2 refused/')" "$G2"
    arm no-controller red "$(printf '%s\n' "$G1" | grep -v '^\[ps2\]')" "$G2"
    arm not-shared red "$G1" "$(printf '%s\n' "$G2" | sed 's/^ps2: keyboard caps lock on, its light set 1/ps2: keyboard caps lock off, its light set 0/')"
    arm ps2-light-not-followed red "$G1" "$(printf '%s\n' "$G2" | sed 's/^ps2: keyboard caps lock on, its light set 1/ps2: keyboard caps lock on, its light set 0/')"
    arm usb-light-missed red "$G1" "$(printf '%s\n' "$G2" | sed 's/^  keyboard: caps lock on, its light set 1/  keyboard: caps lock on, its light set 0/')"
    arm usb-shift-not-inverted red "$G1" "$(printf '%s\n' "$G2" | sed 's/^afrisovera> lsps2$/afrisovera> LSPS2/')"
    arm no-usb-keyboard red "$G1" "$(printf '%s\n' "$G2" | grep -v 'USB keyboard')"
    arm panic red "$G1" "$(printf '%s\n[pc] PANIC: x' "$G2")"
    arm empty red "" ""
    arm light-unbounded red "$(printf '%s\n' "$G1" | sed 's/longest 431 us/longest 3400000 us/')" "$G2"
    arm no-bound-line red "$(printf '%s\n' "$G1" | grep -v 'light commands')" "$G2"
    arm3() { # arm3 <name> <expect> <boot 3 text>
        local got
        printf '%s\n' "$3" > "$D/s3"
        if grade3 "$D/s3" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    arm3 silent-green green "$G3"
    arm3 silent-held-off red "$(printf '%s\n' "$G3" | sed 's/longest 20011 us/longest 3400000 us/')"
    arm3 silent-no-giveup red "$(printf '%s\n' "$G3" | grep -v 'does not answer light')"
    arm3 silent-kept-trying red "$(printf '%s\n' "$G3" | sed 's/0 times, 2 refused/0 times, 5 refused/')"
    arm3 silent-timer-stopped red "$(printf '%s\n' "$G3" | sed 's/^547 ticks/512 ticks/')"
    arm3 silent-caps-broken red "$(printf '%s\n' "$G3" | sed 's/^abCD1efg$/abcd1Efg/')"
    arm3 silent-line-glued red "$(printf '%s\n' "$G3" | sed '/^\[ps2\] keyboard does not answer/d; s/^afrisovera> echo abCD1efg$/afrisovera> echo abCD1e[ps2] keyboard does not answer light commands; Caps Lock still works, its light is left alone\nfg/')"
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 25 ]
}

echo "== PC PS/2 Caps Lock light, shared with USB (PC track step 3) =="
if ! selftest; then echo "PC PS2CAPS TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC PS2CAPS SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC PS2CAPS TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC PS2CAPS TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-ps2caps.XXXXXX")"
echo "  --- boot 1: PS/2 keyboard alone ---"
qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:5172,server,nowait" -qmp "tcp:127.0.0.1:5173,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5172 5173 "$W/serial1.log" \
    "@raw echo ab" "@key caps_lock" "@raw cd1E" "@key caps_lock" "fg" "lsps2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
clean "$W/serial1.log" | grep -aE '^\[ps2\]|^afrisovera> (echo|lsps2)|^ps2:|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- boot 2: PS/2 and a USB keyboard ---"
qemu-system-aarch64 -M virt -m 256M -display none \
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0 \
    -serial "tcp:127.0.0.1:5174,server,nowait" -qmp "tcp:127.0.0.1:5175,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5174 5175 "$W/serial2.log" "@key caps_lock" "LSPS2" "LSUSB"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
clean "$W/serial2.log" | grep -aE '^\[ps2\]|^\[xhci\]|^afrisovera> |^ps2:|^usb:|^  keyboard' | sed 's/^/  | /'

echo "  --- boot 3: a keyboard that does not answer light commands (the test rig's switch) ---"
qemu-system-aarch64 -M virt -m 256M -display none -fw_cfg name=opt/afrisovera/ps2-led-mute,string=1 \
    -serial "tcp:127.0.0.1:5176,server,nowait" -qmp "tcp:127.0.0.1:5177,server,nowait" &
QPID=$!
python3 scripts/lib/usb-type.py 5176 5177 "$W/serial3.log" \
    "@raw echo ab" "@key caps_lock" "@raw cd1E" "@key caps_lock" "fg" "uptime" "uptime" "lsps2"
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
clean "$W/serial3.log" | grep -aE '^\[ps2\]|^afrisovera> |^ps2:|ticks  \(|^'"$WORD"'$' | sed 's/^/  | /'

echo "  --- grading ---"
r=0
grade1 "$W/serial1.log" || r=1
grade2 "$W/serial2.log" || r=1
grade3 "$W/serial3.log" || r=1
if [ "$r" -eq 0 ]; then
    rm -rf "$W"
    echo "PC PS2CAPS TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC PS2CAPS TEST FAILED"
exit 1
