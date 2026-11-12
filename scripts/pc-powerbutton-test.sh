#!/usr/bin/env bash
# PC track step 3 (7 Nov 2026): the PC's POWER BUTTON switches AfriSovera off cleanly.
#
# A short press of a PC's power button is an ACPI fixed event (PWRBTN_STS in PM1_STS); the
# kernel reads where those registers are from the FADT, switches the chipset into ACPI mode
# (ACPI_ENABLE to SMI_CMD, as Linux does) and sets PWRBTN_EN; the idle loop then sees a press,
# writes the shell's files to the disk in use (as `sync` does) and asks the firmware to switch
# off (as `poweroff` does). Without this a real PC's button either did nothing or cut the power.
#
# Boots the PC kernel twice with a blank 8 MiB NVMe drive. Boot 1: the boot line says the button
# is ready, `acpiinfo` shows it armed, `claimdisk nvme`, a file written; then QMP
# `system_powerdown` (QEMU's press of the ACPI power button): the press must be seen, the files
# synced AFTER it, the firmware asked to switch off, and QEMU must exit by itself. Boot 2: the
# file is read back from the drive, and a second press switches off again.
# Runs the same in an ARM walk and a PC walk: it builds the PC kernel and puts the PC shim
# (scripts/lib/pc-arch) on PATH itself.
#   scripts/pc-powerbutton-test.sh              # grader selftest, then the two QEMU boots
#   scripts/pc-powerbutton-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"

grade() { # grade <boot1> <boot2>
    local fail=0 A B
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] && [ -r "$2" ] || { echo "  FAIL: missing log"; return 1; }
    A="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    B="$(tr -d '\r' < "$2" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$2" <<< "$1"; }
    # what came after the press (the first "pressed" line onwards)
    after() { printf '%s\n' "$1" | sed -n '/^\(afrisovera> \)\{0,1\}\[pc\] power button pressed: saving AfriSovera.s files, then switching off$/,$p'; }
    local READY='^\[pc\] power button: ready; a press saves the files and switches off \(PM1a 0x[0-9a-f]+, ACPI mode (switched on|already on)\)$'
    has "$A" "$READY" || _f "boot 1: the power button was not armed at boot"
    has "$B" "$READY" || _f "boot 2: the power button was not armed at boot"
    has "$A" '^acpi: power button on \(PM1a events 0x[0-9a-f]+, control 0x[0-9a-f]+, SMI_CMD 0x[0-9a-f]+, ACPI mode on, presses 0\)$' ||
        _f "acpiinfo does not show the button armed with ACPI mode on"
    has "$A" '^claimdisk: the blank nvme disk now holds AfriSovera.s files$' || _f "boot 1: claimdisk nvme did not take the blank drive"
    local PA PB
    PA="$(after "$A")"; PB="$(after "$B")"
    [ -n "$PA" ] || _f "boot 1: the press of the power button was never seen"
    [ -n "$PB" ] || _f "boot 2: the second press of the power button was never seen"
    has "$PA" '^\[vfs\] synced [0-9]+ files to disk$' || _f "boot 1: the files were not synced after the press"
    has "$PA" '^\[pc\] power button: files saved to the disk$' || _f "boot 1: the press did not say the files were saved"
    has "$PA" '^\[pc\] powering off \(UEFI ResetSystem: shutdown\)$' || _f "boot 1: the firmware was not asked to switch off"
    has "$PB" '^\[pc\] powering off \(UEFI ResetSystem: shutdown\)$' || _f "boot 2: the firmware was not asked to switch off"
    has "$A" '^== power-button: qemu exited on its own after [0-9]+ s ==$' || _f "boot 1: the machine did not switch off after the press"
    has "$B" '^== power-button: qemu exited on its own after [0-9]+ s ==$' || _f "boot 2: the machine did not switch off after the press"
    has "$B" '^PWRBTN-7NOV$' || _f "boot 2: notes.txt did not come back from the drive"
    local ALL; ALL="$(printf '%s\n%s\n' "$A" "$B")"
    if has "$ALL" 'KERNEL PANIC|^\[pc\] (EXCEPTION|PANIC)'; then
        _f "the kernel panicked or took an exception"
    fi
    return "$fail"
}

S1='[pc] power button: ready; a press saves the files and switches off (PM1a 0x600, ACPI mode switched on)
afrisovera> acpiinfo
acpi: 2 core(s), local-APIC ids [0, 1], I/O APIC at 0xfec00000
acpi: PM timer at I/O port 0x608
acpi: power button on (PM1a events 0x600, control 0x604, SMI_CMD 0xb2, ACPI mode on, presses 0)
afrisovera> claimdisk nvme
claimdisk: the blank nvme disk now holds AfriSovera'"'"'s files
[vfs] synced 17 files to disk
afrisovera> write notes.txt PWRBTN-7NOV
[vfs] synced 18 files to disk
afrisovera> [pc] power button pressed: saving AfriSovera'"'"'s files, then switching off
[vfs] synced 18 files to disk
[pc] power button: files saved to the disk
[pc] powering off (UEFI ResetSystem: shutdown)

== power-button: qemu exited on its own after 1 s =='
S2='[pc] power button: ready; a press saves the files and switches off (PM1a 0x600, ACPI mode switched on)
afrisovera> cat notes.txt
PWRBTN-7NOV
afrisovera> [pc] power button pressed: saving AfriSovera'"'"'s files, then switching off
[vfs] synced 18 files to disk
[pc] power button: files saved to the disk
[pc] powering off (UEFI ResetSystem: shutdown)

== power-button: qemu exited on its own after 1 s =='

selftest() {
    local Dd pass=0 bad=0
    Dd="$(mktemp -d "$T/pc-powerbutton-selftest.XXXXXX")" || return 1
    arm() { # arm <name> <expect> <b1> <b2>
        local got
        printf '%s\n' "$3" > "$Dd/1"; printf '%s\n' "$4" > "$Dd/2"
        if grade "$Dd/1" "$Dd/2" > "$Dd/log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"; sed 's/^/      /' "$Dd/log"; fi
    }
    crlf() { printf '%s\n' "$1" | sed 's/$/\r/'; }
    local NOTARMED='[pc] power button: not handled (a control-method button (needs ACPI'"'"'s AML, not read yet)); hold it 4 s to switch off'
    echo "  --- grader selftest (no QEMU) ---"
    arm sample green "$S1" "$S2"
    arm crlf green "$(crlf "$S1")" "$(crlf "$S2")"
    arm already-on green "$(printf '%s\n' "$S1" | sed 's/ACPI mode switched on)/ACPI mode already on)/')" "$S2"
    arm not-armed red "$(printf '%s\n' "$S1" | sed "1s/.*/$(printf '%s' "$NOTARMED" | sed 's/[\/&]/\\&/g')/")" "$S2"
    arm acpiinfo-off red "$(printf '%s\n' "$S1" | sed 's/ACPI mode on, presses 0/ACPI mode off, presses 0/')" "$S2"
    arm not-claimed red "$(printf '%s\n' "$S1" | grep -v '^claimdisk:')" "$S2"
    arm press-unseen red "$(printf '%s\n' "$S1" | grep -v 'power button pressed\|files saved\|powering off')" "$S2"
    arm no-sync-after-press red "$(printf '%s\n' "$S1" | sed '/power button pressed/,$ { /^\[vfs\] synced/d }')" "$S2"
    arm hard-off red "$(printf '%s\n' "$S1" | grep -v '^\[pc\] powering off')" "$S2"
    arm still-running red "$(printf '%s\n' "$S1" | sed 's/^== power-button: qemu exited.*/== power-button: qemu still running after 60 s ==/')" "$S2"
    arm boot2-still-running red "$S1" "$(printf '%s\n' "$S2" | sed 's/^== power-button: qemu exited.*/== power-button: qemu still running after 60 s ==/')"
    arm not-persisted red "$S1" "$(printf '%s\n' "$S2" | grep -v '^PWRBTN')"
    arm echoed-only red "$S1" "$(printf '%s\n' "$S2" | sed 's/^PWRBTN-7NOV$/afrisovera> cat notes.txt PWRBTN-7NOV/')"
    arm panic red "$S1" "$(printf '%s\nKERNEL PANIC at acpi.rs' "$S2")"
    arm empty red "" ""
    rm -rf "$Dd"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 15 ]
}

echo "== PC power button (ACPI fixed event, PC track step 3) =="
if ! selftest; then echo "PC POWERBUTTON TEST FAILED (grader selftest): no QEMU run spent"; exit 1; fi
[ "${1:-}" = "--selftest" ] && { echo "PC POWERBUTTON SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || for d in "/c/Program Files/qemu" /usr/local/bin /usr/bin; do
    { [ -x "$d/qemu-system-x86_64" ] || [ -x "$d/qemu-system-x86_64.exe" ]; } && { PATH="$d:$PATH"; break; }
done
command -v qemu-system-x86_64 > /dev/null || { echo "PC POWERBUTTON TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }

echo "  --- build ---"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
[ -r "$PC_EFI" ] || { echo "PC POWERBUTTON TEST FAILED (harness: $PC_EFI was not built)"; exit 1; }
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"

W="$(mktemp -d "$T/pc-powerbutton.XXXXXX")"
head -c 8388608 /dev/zero > "$W/nvme.img"   # a fresh, blank 8 MiB drive

boot() { # boot <serial port> <log> <shell commands...>
    local sp=$1 log=$2; shift 2
    qemu-system-aarch64 -M virt -m 256M -display none \
        -drive "file=$W/nvme.img,if=none,format=raw,id=nv" -device nvme,serial=afripb1,drive=nv \
        -serial "tcp:127.0.0.1:$sp,server,nowait" -qmp "tcp:127.0.0.1:$((sp+1)),server,nowait" &
    local q=$!
    python3 scripts/lib/power-button.py "$sp" "$((sp+1))" "$log" "$@" > "$log.drv" 2>&1
    kill "$q" 2> /dev/null
    wait "$q" 2> /dev/null
}
echo "  --- boot 1: acpiinfo, claimdisk nvme, write notes.txt, then the power button ---"
boot 5090 "$W/boot1.log" "acpiinfo" "claimdisk nvme" "write notes.txt PWRBTN-7NOV"
echo "  --- boot 2: the same drive, cat notes.txt, then the power button again ---"
boot 5092 "$W/boot2.log" "cat notes.txt"
cat "$W"/boot[12].log | tr -d '\r' | grep -aE '^\[pc\] power|^acpi: power|^\[pc\] powering|^claimdisk|^\[vfs\]|PWRBTN|^== power' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$W/boot1.log" "$W/boot2.log"; then
    rm -rf "$W"
    echo "PC POWERBUTTON TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC POWERBUTTON TEST FAILED"
exit 1
