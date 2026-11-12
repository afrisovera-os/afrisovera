#!/usr/bin/env bash
# PC PAGING check (suite 692, PC track step 1: the kernel's own page tables, 4 Nov 2026).
#
# Same boot as suite 691 (pc-boot): the x86-64 kernel `mansa-pc` as EFI/BOOT/BOOTX64.EFI
# on a FAT disk, QEMU q35 + EDK II (OVMF), graded from the serial log. 691 grades the boot;
# this suite grades what kernel/src/pc/mmu.rs adds:
#   1. the kernel built its OWN page tables and loaded them: the CR3 it reports differs from
#      the firmware's, at least 4 GiB is mapped 1:1 (RAM, framebuffer, APIC), in >= 1 frame;
#   2. demand paging: N >= 1 pages nobody mapped were touched, each one page fault served by
#      the kernel's #PF handler (faults == N), and all N read back through BOTH the virtual
#      address and the frame's own 1:1 address: "N of N ... (ok)";
#   3. ORDER: the page-table line comes BEFORE the local-APIC timer line, so the timer's
#      MMIO and its interrupts ran under the kernel's tables, not the firmware's;
#   4. nothing broke after the switch: the timer, the shared crypto and BOOT OK, with no
#      [FAIL], no EXCEPTION and no PANIC line.
# Every line is read at column 0 after stripping CR and escape codes (the rxsplit lesson).
# NOT graded: addresses and sizes beyond the floors above (host- and firmware-dependent).
# A grader selftest runs first on canned logs, both directions (the 2.767 pattern).
# Usage: scripts/pc-paging-test.sh            selftest, then build + boot
#        scripts/pc-paging-test.sh --selftest selftest only
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

    local PT CR FW GB TF
    PT="$(printf '%s\n' "$L" | grep -aE '^\[pc\] own page tables: CR3 0x[0-9a-f]+ \(firmware.s was 0x[0-9a-f]+\), [0-9]+ GiB mapped 1:1 with 2 MiB pages in [0-9]+ table frames$' | tail -1)"
    if [ -z "$PT" ]; then
        _f "no own-page-tables line; the kernel never switched to its own tables"
    else
        CR="$(printf '%s' "$PT" | sed -E 's/^.*CR3 (0x[0-9a-f]+) .*$/\1/')"
        FW="$(printf '%s' "$PT" | sed -E 's/^.*was (0x[0-9a-f]+)\).*$/\1/')"
        GB="$(printf '%s' "$PT" | sed -E 's/^.*\), ([0-9]+) GiB.*$/\1/')"
        TF="$(printf '%s' "$PT" | sed -E 's/^.* in ([0-9]+) table frames$/\1/')"
        [ "$CR" != "$FW" ] || _f "CR3 $CR is still the firmware's tables"
        [ "$GB" -ge 4 ] || _f "only $GB GiB mapped; the 4 GiB floor (framebuffer, APIC) is not covered"
        [ "$TF" -ge 1 ] || _f "the tables used $TF frames"
    fi
    local DP NF RB RN
    DP="$(printf '%s\n' "$L" | grep -aE '^\[pc\] demand paging: [0-9]+ page faults served at 0x[0-9a-f]+, [0-9]+ of [0-9]+ pages read back through both addresses \(ok\)$' | tail -1)"
    if [ -z "$DP" ]; then
        _f "no demand-paging line ending (ok); a fault was not served or a page did not read back"
    else
        NF="$(printf '%s' "$DP" | sed -E 's/^\[pc\] demand paging: ([0-9]+) page faults.*$/\1/')"
        RB="$(printf '%s' "$DP" | sed -E 's/^.*, ([0-9]+) of ([0-9]+) pages.*$/\1/')"
        RN="$(printf '%s' "$DP" | sed -E 's/^.*, ([0-9]+) of ([0-9]+) pages.*$/\2/')"
        [ "$RN" -ge 1 ] || _f "no pages were touched"
        [ "$RB" -eq "$RN" ] || _f "only $RB of $RN pages read back"
        [ "$NF" -eq "$RN" ] || _f "$NF page faults for $RN pages; each touch must fault exactly once"
    fi
    local LP LA
    LP="$(printf '%s\n' "$L" | grep -anE '^\[pc\] own page tables: CR3' | head -1 | cut -d: -f1)"
    LA="$(printf '%s\n' "$L" | grep -anE '^\[pc\] local APIC .* timer: [0-9]+ interrupts taken$' | head -1 | cut -d: -f1)"
    if [ -z "$LA" ]; then
        _f "no local APIC timer line after the switch"
    elif [ -n "$LP" ] && [ "$LA" -lt "$LP" ]; then
        _f "the timer ran BEFORE the switch; it proves nothing about the kernel's tables"
    fi
    has '^\[pc\] shared code: SHA-256 ok, SHA-512 ok, Ed25519 ok' ||
        _f "a shared crypto self-test did not report ok after the switch"
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
[pc] boot services exited; the firmware is gone, the kernel owns the machine
[pc] IDT loaded; breakpoint exception taken and returned (1)
[pc] own page tables: CR3 0xcdee000 (firmware's was 0xf801000), 4 GiB mapped 1:1 with 2 MiB pages in 6 table frames
[pc] demand paging: 3 page faults served at 0x8000000000, 3 of 3 pages read back through both addresses (ok)
[pc] local APIC (xAPIC at 0xfee00000) timer: 10 interrupts taken
[pc] shared code: SHA-256 ok, SHA-512 ok, Ed25519 ok (the same files as the ARM kernel)
kernel heap: 80/80 bump checks held (arena 2564 KiB at 0x0cced9e0)
[pc] MANSA-PC BOOT OK
[pc] powering off (requested by the test rig through fw_cfg)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-paging-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }
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
    arm never-switched red < <(printf '%s\n' "$GREEN" | grep -v 'own page tables')
    arm still-firmware-cr3 red < <(printf '%s\n' "$GREEN" | sed 's/CR3 0xcdee000/CR3 0xf801000/')
    arm under-4gib red < <(printf '%s\n' "$GREEN" | sed 's/, 4 GiB mapped/, 2 GiB mapped/')
    arm no-demand-line red < <(printf '%s\n' "$GREEN" | grep -v 'demand paging')
    arm readback-mismatch red < <(printf '%s\n' "$GREEN" | sed 's/3 of 3 pages read back through both addresses (ok)/0 of 3 pages read back through both addresses (MISMATCH)/')
    arm faults-not-per-page red < <(printf '%s\n' "$GREEN" | sed 's/: 3 page faults/: 1 page faults/')
    arm zero-pages red < <(printf '%s\n' "$GREEN" | sed 's/: 3 page faults/: 0 page faults/; s/3 of 3 pages/0 of 0 pages/')
    arm timer-before-switch red < <(printf '%s\n' "$GREEN" | awk '/local APIC/ {next} /own page tables/ {print "[pc] local APIC (xAPIC at 0xfee00000) timer: 10 interrupts taken"} {print}')
    arm page-fault-exception red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] MANSA-PC BOOT OK$/[pc] EXCEPTION vector 14 (page fault) error 0x2 at RIP 0x1 (CR2 0x8000001000); halted/')
    arm verdict-in-prose red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] demand paging/  [pc] demand paging/')
    arm empty-log red < /dev/null
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 13 ]
}

# ------------------------------------------------------------------- driver --
echo "== PC paging (suite 692, PC track step 1) =="
if ! selftest; then
    echo "PC PAGING TEST FAILED (grader selftest): no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "PC PAGING SELFTEST ONLY: no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || { echo "PC PAGING TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd \
         /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/ovmf/OVMF.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC PAGING TEST FAILED (harness: no x86-64 UEFI firmware (edk2-x86_64-code.fd / OVMF_CODE.fd) found)"; exit 1; }

echo "  --- build ---"
if ! cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2; then
    :
fi
EFI="target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$EFI" ] || { echo "PC PAGING TEST FAILED (harness: $EFI was not built)"; exit 1; }

W="$(mktemp -d "$T/pc-paging.XXXXXX")"
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
tr -d '\r' < "$SER" 2>/dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | grep -aE '^\[pc\]|^kernel heap: [0-9]|^ *\[FAIL\]' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$SER"; then
    rm -rf "$W"
    echo "PC PAGING TEST PASSED"
    exit 0
fi
echo "  serial kept at $SER"
echo "PC PAGING TEST FAILED"
exit 1
