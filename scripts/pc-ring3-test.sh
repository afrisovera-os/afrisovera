#!/usr/bin/env bash
# PC RING 3 check (suite 693, PC track step 1: programs at the CPU's lowest privilege, 4 Nov 2026).
#
# Same boot as suites 691/692: the x86-64 kernel `mansa-pc` as EFI/BOOT/BOOTX64.EFI on a FAT
# disk, QEMU q35 + EDK II (OVMF), graded from the serial log. This suite grades
# kernel/src/pc/user.rs: three tiny programs entered with iretq at privilege level 3.
#   1. program 1 talks to the kernel only through system calls (int 0x80): it prints 42, asks
#      which privilege level the CPU saw it at and prints that (the kernel must report 3)
#      and exits with code 7: "[user] print 42", "[user] print 3", "made 4 system calls and
#      exited with code 7 (ok)";
#   2. program 2 reads a kernel address: the read must page-fault AT THAT ADDRESS with an
#      error code saying present + user-mode (bits 0 and 2), the program is stopped and the
#      kernel continues "(ok)";
#   3. program 3 runs `cli`: a general-protection fault, the program is stopped "(ok)";
#   4. the kernel reaches MANSA-PC BOOT OK afterwards, with no [FAIL], EXCEPTION or PANIC line
#      (a ring-3 fault reported as a kernel EXCEPTION means containment failed).
# Every line is read at column 0 after stripping CR and escape codes (the rxsplit lesson).
# NOT graded: the addresses themselves (host- and build-dependent), only that they agree.
# A grader selftest runs first on canned logs, both directions (the 2.767 pattern).
# Usage: scripts/pc-ring3-test.sh            selftest, then build + boot
#        scripts/pc-ring3-test.sh --selftest selftest only
set -uo pipefail

T="${TMPDIR:-/tmp}"

# ----------------------------------------------------------------- the grader --
grade() {
    local F="$1" fail=0 L
    _f() { echo "  FAIL: $*"; fail=1; }
    if [ ! -r "$F" ]; then
        echo "  FAIL: no serial log at $F"
        return 1
    fi
    L="$(tr -d '\r' < "$F" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { grep -aqE "$1" <<< "$L"; }

    has '^\[user\] print 42$' ||
        _f "program 1's first system call (print 42) never reached the kernel"
    has '^\[user\] print 3$' ||
        _f "the kernel did not see program 1 running at privilege level 3"
    if has '^\[user\] print [0-2]$'; then
        _f "program 1 ran at privilege level $(printf '%s\n' "$L" | grep -aoE '^\[user\] print [0-2]$' | head -1 | awk '{print $3}'); not in ring 3"
    fi
    has '^\[pc\] ring 3: program 1 made 4 system calls and exited with code 7 \(ok\)$' ||
        _f "program 1 did not make its 4 system calls and exit with code 7"

    local P2 RA FA ER
    P2="$(printf '%s\n' "$L" | grep -aE '^\[pc\] ring 3: program 2 read kernel memory at 0x[0-9a-f]+ -> page fault \(error 0x[0-9a-f]+\) at 0x[0-9a-f]+, program stopped, kernel continues \(ok\)$' | tail -1)"
    if [ -z "$P2" ]; then
        _f "program 2's read of kernel memory was not stopped by a page fault"
    else
        RA="$(printf '%s' "$P2" | sed -E 's/^.*memory at (0x[0-9a-f]+) .*$/\1/')"
        FA="$(printf '%s' "$P2" | sed -E 's/^.*\) at (0x[0-9a-f]+), .*$/\1/')"
        ER="$(printf '%s' "$P2" | sed -E 's/^.*\(error (0x[0-9a-f]+)\).*$/\1/')"
        [ "$RA" = "$FA" ] || _f "the fault was at $FA, not at the kernel address $RA the program read"
        [ $(( ER & 5 )) -eq 5 ] || _f "fault error $ER does not say present + user-mode (bits 0 and 2)"
    fi
    has '^\[pc\] ring 3: program 3 ran a privileged instruction \(cli\) -> general protection fault, program stopped \(ok\)$' ||
        _f "program 3's privileged instruction was not stopped"

    if has '^ *\[FAIL\]'; then
        _f "a [FAIL] line: $(printf '%s\n' "$L" | grep -aE '^ *\[FAIL\]' | head -1)"
    fi
    if has '^\[pc\] (EXCEPTION|PANIC)'; then
        _f "$(printf '%s\n' "$L" | grep -aE '^\[pc\] (EXCEPTION|PANIC)' | head -1)"
    fi
    has '^\[pc\] MANSA-PC BOOT OK$' ||
        _f "no 'MANSA-PC BOOT OK' line"
    return "$fail"
}

# --------------------------------------------------------------- the selftest --
GREEN="[pc] Mansa x86-64 (PC), started by UEFI firmware
[pc] own page tables: CR3 0xcded000 (firmware's was 0xf801000), 4 GiB mapped 1:1 with 2 MiB pages in 6 table frames
[user] print 42
[user] print 3
[pc] ring 3: program 1 made 4 system calls and exited with code 7 (ok)
[pc] ring 3: program 2 read kernel memory at 0xccd69d8 -> page fault (error 0x5) at 0xccd69d8, program stopped, kernel continues (ok)
[pc] ring 3: program 3 ran a privileged instruction (cli) -> general protection fault, program stopped (ok)
[pc] MANSA-PC BOOT OK
[pc] powering off (requested by the test rig through fw_cfg)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ring3-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }
    # arms read via `< <(...)`, never a pipe: a piped arm loses its counts (the pc-boot lesson)
    arm() { # arm <name> <expect green|red>  < serial
        local got
        cat > "$D/$1.serial"
        if grade "$D/$1.serial" > "$D/$1.log" 2>&1; then got=green; else got=red; fi
        if [ "$got" = "$2" ]; then
            pass=$((pass+1)); echo "    arm $1: $got (expected $2) OK"
        else
            bad=$((bad+1)); echo "    arm $1: $got but expected $2  <-- GRADER BUG"
            sed 's/^/      /' "$D/$1.log"
        fi
    }
    echo "  --- grader selftest (no QEMU; both directions must be reachable) ---"
    arm real-green-2026-11-04 green < <(printf '%s\n' "$GREEN")
    arm crlf-and-escapes green < <(printf '%s\n' "$GREEN" | sed 's/$/\r/; 1s/^/\x1b[2J\x1b[01;01H/')
    arm no-syscall-print red < <(printf '%s\n' "$GREEN" | grep -v 'print 42')
    arm ran-in-ring-0 red < <(printf '%s\n' "$GREEN" | sed 's/^\[user\] print 3$/[user] print 0/')
    arm wrong-exit-code red < <(printf '%s\n' "$GREEN" | sed 's/exited with code 7 (ok)/exited with code 214870320 (WRONG)/')
    arm too-few-calls red < <(printf '%s\n' "$GREEN" | sed 's/made 4 system calls/made 2 system calls/')
    arm snoop-not-stopped red < <(printf '%s\n' "$GREEN" | sed 's/kernel continues (ok)/kernel continues (NOT STOPPED)/')
    arm fault-elsewhere red < <(printf '%s\n' "$GREEN" | sed 's/) at 0xccd69d8,/) at 0xccea930,/')
    arm fault-not-user red < <(printf '%s\n' "$GREEN" | sed 's/(error 0x5)/(error 0x1)/')
    arm cli-not-stopped red < <(printf '%s\n' "$GREEN" | grep -v 'program 3')
    arm kernel-exception red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] MANSA-PC BOOT OK$/[pc] EXCEPTION vector 13 (general protection) error 0x0 at RIP 0x10000000000 (CR2 0x0); halted/')
    arm verdict-in-prose red < <(printf '%s\n' "$GREEN" | sed 's/^\[user\] print 42$/  [user] print 42/')
    arm empty-log red < /dev/null
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

# ------------------------------------------------------------------- driver --
echo "== PC ring 3 (suite 693, PC track step 1) =="
if ! selftest; then
    echo "PC RING3 TEST FAILED (grader selftest): no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "PC RING3 SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || { echo "PC RING3 TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd \
         /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/ovmf/OVMF.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC RING3 TEST FAILED (harness: no x86-64 UEFI firmware (edk2-x86_64-code.fd / OVMF_CODE.fd) found)"; exit 1; }

echo "  --- build ---"
if ! cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2; then
    :
fi
EFI="target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$EFI" ] || { echo "PC RING3 TEST FAILED (harness: $EFI was not built)"; exit 1; }

W="$(mktemp -d "$T/pc-ring3.XXXXXX")"
mkdir -p "$W/esp/EFI/BOOT"
cp "$EFI" "$W/esp/EFI/BOOT/BOOTX64.EFI"
SER="$W/pc.serial"
# native Windows QEMU wants Windows paths; elsewhere these are the identity
winpath() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else printf '%s' "$1"; fi; }

echo "  --- boot: q35 + $(basename "$OVMF") ---"
timeout 240 qemu-system-x86_64 -M q35 -m 256M \
    -drive "if=pflash,format=raw,readonly=on,file=$(winpath "$OVMF")" \
    -drive "format=raw,file=fat:rw:$(winpath "$W/esp")" \
    -serial "file:$(winpath "$SER")" -display none -no-reboot -net none \
    -fw_cfg name=opt/afrisovera/poweroff,string=1
QRC=$?
echo "  qemu exit $QRC ($([ "$QRC" -eq 124 ] && echo 'timed out; the kernel never powered off' || echo 'the kernel powered off'))"
tr -d '\r' < "$SER" 2>/dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | grep -aE '^\[(pc|user)\]|^kernel heap: [0-9]|^ *\[FAIL\]' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$SER"; then
    rm -rf "$W"
    echo "PC RING3 TEST PASSED"
    exit 0
fi
echo "  serial kept at $SER"
echo "PC RING3 TEST FAILED"
exit 1
