#!/usr/bin/env bash
# Switching off and restarting the PC, and PS/2 key repeat (14 Nov 2026, the owner's HP: `poweroff` came out
# as "poweroffff" and he could not switch the laptop off).
#
# Two boots of the PC kernel with only the PS/2 keyboard; scripts/lib/usb-steps.py presses the keys:
#   1. the firmware's way: "echo " then a burst of 4 make codes for f in 120 ms (a doubled key) and 25 for g
#      over a second (a held key), Enter: the shell must print ONE f and a repeated g only from 0.5 s on. Then
#      `lsps2` counts the repeats held back, `help shutdown` shows the PC's wording, `restart` restarts the PC
#      (the firmware's ResetSystem: a second boot) and `shutdown` switches it off (QEMU ends by itself).
#   2. ACPI's way (fw_cfg opt/afrisovera/power-acpi: the firmware is skipped): `acpiinfo` names \_S5 and the
#      FADT reset register, `restart` restarts through the reset register, `shutdown` switches off by S5.
#   scripts/pc-power-test.sh              # grader selftest, then the two boots
#   scripts/pc-power-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade1() { # grade1 <serial capture> <qemu ended by itself: yes|no>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^afrisovera> echo fg+$' || _f "the echo line did not reach the shell as 'echo f' and g's"
    local line
    line="$(grep -aE '^fg+$' <<< "$S" | head -1)"
    if [ -z "$line" ]; then
        _f "the shell did not print one f and the g's (a doubled make code was typed twice?)"
    else
        local g=$(( ${#line} - 1 ))
        { [ "$g" -ge 6 ] && [ "$g" -le 17 ]; } || _f "a held g gave $g g's (want about 13: one, then from 0.5 s on at most 30 a second)"
    fi
    local held
    held="$(grep -aoE '^ps2: key repeat: [0-9]+ repeats held back, [0-9]+ taken' <<< "$S" | tail -1 | awk '{print $4}')"
    { [ -n "$held" ] && [ "$held" -ge 10 ]; } || _f "lsps2 does not count the repeats held back (got '${held:-none}')"
    has '^  poweroff, shutdown  save the files, then switch the PC off \(the firmware, then ACPI S5\)$' ||
        _f "help shutdown does not show the PC's wording"
    has 'PSCI' && _f "help still names PSCI on the PC"
    has '^\[pc\] restart: no disk in use, nothing to save' || _f "restart did not save (or say why not) first"
    has "^\[pc\] restarting \(the firmware's ResetSystem\)$" || _f "restart did not go through the firmware"
    [ "$(grep -ac 'MANSA-PC BOOT OK' <<< "$S")" -ge 2 ] || _f "the PC did not boot a second time after restart"
    has "^\[pc\] switching off \(the firmware's ResetSystem\)$" || _f "shutdown did not go through the firmware"
    [ "$2" = yes ] || _f "QEMU did not end by itself after shutdown"
    has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' && _f "the kernel faulted"
    return "$fail"
}

grade2() { # grade2 <serial capture> <qemu ended by itself: yes|no>
    local fail=0 S
    _f() { echo "  FAIL: $*"; fail=1; }
    S="$(tr -d '\r' < "$1" 2> /dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$S"; }
    has '^acpi: switching off: S5 by PM1 control, SLP_TYP a=[0-7] b=[0-7]; restarting: the FADT reset register \(I/O\)$' ||
        _f "acpiinfo does not name \\_S5 and the FADT reset register"
    has '^\[pc\] restarting \(the ACPI reset register\)$' || _f "restart did not use the ACPI reset register"
    has "ResetSystem" && _f "the firmware was used although the rig asked for ACPI"
    [ "$(grep -ac 'MANSA-PC BOOT OK' <<< "$S")" -ge 2 ] || _f "the PC did not boot a second time after the ACPI restart"
    has '^\[pc\] switching off \(ACPI S5\)$' || _f "shutdown did not use ACPI S5"
    has 'ACPI could not switch off' && _f "ACPI S5 did not switch the PC off"
    [ "$2" = yes ] || _f "QEMU did not end by itself after the S5 shutdown"
    has 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)' && _f "the kernel faulted"
    return "$fail"
}

G1="[pc] MANSA-PC BOOT OK
afrisovera> echo fggggggggggggg
fggggggggggggg
afrisovera> lsps2
ps2: i8042 keyboard, polled; 80 scancodes read, 30 keys typed
ps2: key repeat: 15 repeats held back, 12 taken (a held key repeats after 0.5 s, at most 30 a second)
afrisovera> help shutdown
commands:
  poweroff, shutdown  save the files, then switch the PC off (the firmware, then ACPI S5)
afrisovera> restart
[pc] restart: no disk in use, nothing to save (files were in memory only)
[pc] restarting (the firmware's ResetSystem)
[pc] MANSA-PC BOOT OK
afrisovera> shutdown
[pc] shutdown: no disk in use, nothing to save (files were in memory only)
[pc] switching off (the firmware's ResetSystem)"
G2="[pc] MANSA-PC BOOT OK
afrisovera> acpiinfo
acpi: switching off: S5 by PM1 control, SLP_TYP a=0 b=0; restarting: the FADT reset register (I/O)
afrisovera> restart
[pc] restart: no disk in use, nothing to save (files were in memory only)
[pc] restarting (the ACPI reset register)
[pc] MANSA-PC BOOT OK
afrisovera> shutdown
[pc] shutdown: no disk in use, nothing to save (files were in memory only)
[pc] switching off (ACPI S5)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-power-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <grade fn> <serial text> <ended>
        local got
        printf '%s\n' "$4" > "$D/s"
        if "$3" "$D/s" "$5" > "$D/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$D/log"; fi
    }
    echo "  --- grader selftest (no QEMU) ---"
    arm green1 green grade1 "$G1" yes
    arm doubled-f red grade1 "$(printf '%s\n' "$G1" | sed 's/fggg/ffffggg/')" yes
    arm repeat-unfiltered red grade1 "$(printf '%s\n' "$G1" | sed 's/^fg*$/fggggggggggggggggggggggggg/')" yes
    arm no-repeat-at-all red grade1 "$(printf '%s\n' "$G1" | sed 's/^fg*$/fg/')" yes
    arm no-count red grade1 "$(printf '%s\n' "$G1" | grep -v 'key repeat')" yes
    arm psci-help red grade1 "$(printf '%s\n%s' "$G1" '  poweroff        shut down the machine (PSCI SYSTEM_OFF)')" yes
    arm no-second-boot red grade1 "$(printf '%s\n' "$G1" | awk '/BOOT OK/ && n++ {next} {print}')" yes
    arm no-save red grade1 "$(printf '%s\n' "$G1" | grep -v 'restart: no disk')" yes
    arm still-on red grade1 "$G1" no
    arm crlf green grade1 "$(printf '%s\n' "$G1" | sed 's/$/\r/')" yes
    arm green2 green grade2 "$G2" yes
    arm firmware-used red grade2 "$(printf '%s\n' "$G2" | sed "s/the ACPI reset register/the firmware's ResetSystem/")" yes
    arm no-s5 red grade2 "$(printf '%s\n' "$G2" | sed 's/S5 by PM1 control, SLP_TYP a=0 b=0/no \\_S5 found/')" yes
    arm s5-failed red grade2 "$(printf '%s\n%s' "$G2" '[pc] ACPI could not switch off (x): everything is stopped')" no
    arm panic red grade2 "$(printf '%s\n[pc] PANIC: x' "$G2")" yes
    arm empty red grade1 "" no
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 16 ]
}

echo "== PC switching off, restarting, PS/2 key repeat =="
if ! selftest; then echo "PC POWER TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC POWER SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC POWER TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC POWER TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-power.XXXXXX")"
fail=0
# run <name> <grade fn> <extra qemu args...> -- <steps...>
run() {
    local name="$1" fn="$2"
    shift 2
    local extra=()
    while [ "$1" != "--" ]; do extra+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -m 256M -display none "${extra[@]}" \
        -serial "tcp:127.0.0.1:4944,server,nowait" -qmp "tcp:127.0.0.1:4945,server,nowait" 2> "$W/$name.qemu" &
    local q=$! ended=no
    python3 scripts/lib/usb-steps.py 4944 4945 "$W/$name.log" "$W/$name.png" "$@" 2> "$W/$name.steps"
    for _ in $(seq 1 20); do kill -0 "$q" 2> /dev/null || { ended=yes; break; }; sleep 1; done
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
    tr -d '\r' < "$W/$name.log" | grep -aE '^afrisovera> (echo|restart|shutdown)|^fg|^ps2: key|^  poweroff|^acpi: switching|^\[pc\] (restart|shutdown|switching|restarting|ACPI|the firmware|MANSA)' | sed "s/^/  $name| /"
    echo "  $name: QEMU ended by itself: $ended"
    echo "  --- grading ($name) ---"
    "$fn" "$W/$name.log" "$ended" || fail=1
}

run firmware grade1 -- keys:e keys:c keys:h keys:o keys:spc burst:f,4,30 burst:g,25,40 keys:ret wait:2 \
    type:lsps2 "type:help shutdown" type:restart wait:45 type:shutdown off
run acpi grade2 -fw_cfg name=opt/afrisovera/power-acpi,string=1 -- \
    type:acpiinfo type:restart wait:45 type:shutdown off

if [ "$fail" -eq 0 ]; then
    rm -rf "$W"
    echo "PC POWER TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC POWER TEST FAILED"
exit 1
