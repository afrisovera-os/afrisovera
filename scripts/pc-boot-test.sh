#!/usr/bin/env bash
# PC BOOT check (suite 691, PC track step 1 — owner's decision 3 Nov 2026, docs/05).
#
# Builds the x86-64 PC kernel (`mansa-pc`, a UEFI application), puts it where every
# PC's firmware looks for a boot loader (EFI/BOOT/BOOTX64.EFI on a FAT partition — the
# same layout a USB stick needs on real hardware), boots it in QEMU's q35 PC with the
# EDK II (OVMF) UEFI firmware, and grades the serial log.
#
# WHAT IS GRADED (each line anchored at column 0, after stripping CR and the firmware's
# terminal escape codes — the rxsplit lesson of 2 Nov 2026: GNU grep's `$` does not
# match before a CR):
#   1. the kernel was started by UEFI, and found the screen (GOP framebuffer);
#   2. it read the memory map and EXITED boot services (the firmware is gone);
#   3. its own frame pool lies ABOVE the heap's arena (the shared heap's layout rule);
#   4. its own GDT (code selector 0x8) and IDT, proved by a breakpoint taken and returned;
#   5. local APIC timer interrupts: at least 10 taken (a live interrupt path, not a flag);
#   6. SHARED kernel code, the same files as the ARM kernel: SHA-256, SHA-512, Ed25519
#      self-tests ok; a Vec through the shared allocator; the shared heap self-test with
#      every check held (P/P, P > 0) and no [FAIL] line anywhere;
#   7. no EXCEPTION and no PANIC line; and the final MANSA-PC BOOT OK.
# NOT graded: timings, addresses, memory sizes (they are the host's and the firmware's,
# not the kernel's — a literal would recreate finding 3.365).
#
# The kernel powers itself off at the end ONLY because this rig passes
# `-fw_cfg name=opt/afrisovera/poweroff` (see kernel/src/pc/qemu.rs); a real PC keeps its
# screen up. `timeout` is the backstop for a kernel that hangs.
#
# A grader selftest runs first on canned logs, both directions (the 2.767 pattern).
#
# Needs: qemu-system-x86_64, the EDK II x86-64 firmware QEMU ships
# (share/qemu/edk2-x86_64-code.fd) or a distro OVMF_CODE.fd, and the Rust target
# x86_64-unknown-uefi. Runs from the repo root.
# Usage: scripts/pc-boot-test.sh            selftest, then build + boot
#        scripts/pc-boot-test.sh --selftest selftest only
set -uo pipefail

T="${TMPDIR:-/tmp}"

# ----------------------------------------------------------------- the grader --
# grade <serial-file> -> 0 green, 1 red; one FAIL line per defect.
grade() {
    local F="$1" fail=0 L
    _f() { echo "  FAIL: $*"; fail=1; }
    if [ ! -r "$F" ]; then
        echo "  FAIL: no serial log at $F"
        return 1
    fi
    L="$(tr -d '\r' < "$F" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g')"
    has() { printf '%s\n' "$L" | grep -aqE "$1"; }

    has '^\[pc\] Mansa x86-64 \(PC\) -- started by UEFI firmware$' ||
        _f "no start line -- the firmware never ran the kernel"
    has '^\[pc\] GOP framebuffer [0-9]+x[0-9]+ ' ||
        _f "no GOP framebuffer line -- the screen was not found"
    has '^\[pc\] UEFI memory map: [1-9][0-9]* entries' ||
        _f "no memory-map line -- the firmware's map was not read"
    has '^\[pc\] boot services exited' ||
        _f "boot services were never exited -- the kernel never took the machine over"
    has '^\[pc\] frame pool: [1-9][0-9]* frames .* at 0x[0-9a-f]+, above the kernel heap arena' ||
        _f "the frame pool is missing or NOT above the heap's arena"
    has '^\[pc\] GDT loaded \(kernel code selector 0x8\)$' ||
        _f "the kernel's own GDT was not loaded (CS is not 0x8)"
    has '^\[pc\] IDT loaded; breakpoint exception taken and returned \(1\)$' ||
        _f "the breakpoint trap did not go through the kernel's IDT exactly once"
    local TK
    TK="$(printf '%s\n' "$L" | grep -aoE '^\[pc\] local APIC .* timer: [0-9]+ interrupts taken$' | tail -1 | awk '{print $(NF-2)}')"
    if [ -z "$TK" ]; then
        _f "no local APIC timer line -- timer interrupts were never taken"
    elif [ "$TK" -lt 10 ]; then
        _f "only $TK timer interrupt(s) taken, not the 10 the kernel waits for"
    fi
    has '^\[pc\] shared code: SHA-256 ok, SHA-512 ok, Ed25519 ok' ||
        _f "a shared crypto self-test did not report ok"
    has '^\[pc\] heap: a Vec of 1000 allocated through the shared allocator, sum 500500 \(ok\)' ||
        _f "the Vec through the shared allocator did not sum correctly"
    local HP HT
    HP="$(printf '%s\n' "$L" | grep -aoE '^kernel heap: [0-9]+/[0-9]+ bump checks held' | tail -1 | sed -E 's#^kernel heap: ([0-9]+)/([0-9]+).*#\1#')"
    HT="$(printf '%s\n' "$L" | grep -aoE '^kernel heap: [0-9]+/[0-9]+ bump checks held' | tail -1 | sed -E 's#^kernel heap: ([0-9]+)/([0-9]+).*#\2#')"
    if [ -z "$HT" ]; then
        _f "the shared heap self-test never reported its checks"
    elif [ "$HT" -eq 0 ] || [ "$HP" -ne "$HT" ]; then
        _f "the shared heap self-test held $HP/$HT checks"
    fi
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
GREEN='[pc] Mansa x86-64 (PC) -- started by UEFI firmware
[pc] firmware: EDK II (revision 0x10000)
[pc] GOP framebuffer 1280x800 (stride 1280) at 0x80000000, BGR
[pc] UEFI memory map: 99 entries, 249 MiB usable after exit, largest free region 163 MiB at 0x1780000
[pc] boot services exited -- the firmware is gone, the kernel owns the machine
[pc] frame pool: 4096 frames (16 MiB) at 0xcdf0000, above the kernel heap arena ending 0xcdef9e0
[pc] GDT loaded (kernel code selector 0x8)
[pc] IDT loaded; breakpoint exception taken and returned (1)
[pc] local APIC (xAPIC at 0xfee00000) timer: 10 interrupts taken
[pc] shared code: SHA-256 ok, SHA-512 ok, Ed25519 ok (the same files as the ARM kernel)
[pc] heap: a Vec of 1000 allocated through the shared allocator, sum 500500 (ok), 8000 B used
kernel heap: bump self-test against synthetic requests
  [ok]   an aligned request at an empty arena starts at the base and costs nothing extra
kernel heap: 80/80 bump checks held (arena 2564 KiB at 0x0ccef9e0 — 1024 KiB reserved + 1540 KiB grown, 0 B used, 0 refusals)
[pc] MANSA-PC BOOT OK
[pc] powering off (requested by the test rig through fw_cfg)'

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-boot-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }
    # Arms read their log via `< <(...)`, never `... | arm`: a pipe runs arm in a
    # subshell and its pass/bad counts are lost (the first draft printed 0 ok / 0 FAIL).
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
    arm real-green-2026-11-03 green < <(printf '%s\n' "$GREEN")
    # the same log with CRLF line ends and the firmware's escape codes: still green
    arm crlf-and-escapes green < <(printf '%s\n' "$GREEN" | sed 's/$/\r/; 1s/^/\x1b[2J\x1b[01;01H/')
    arm never-exited red < <(printf '%s\n' "$GREEN" | grep -v 'boot services exited')
    arm pool-below-arena red < <(printf '%s\n' "$GREEN" | sed 's/, above the kernel/, NOT above the kernel/')
    arm too-few-ticks red < <(printf '%s\n' "$GREEN" | sed 's/timer: 10 interrupts/timer: 3 interrupts/')
    arm no-timer red < <(printf '%s\n' "$GREEN" | grep -v 'local APIC')
    arm breakpoint-missed red < <(printf '%s\n' "$GREEN" | sed 's/returned (1)/returned (0)/')
    arm crypto-failed red < <(printf '%s\n' "$GREEN" | sed 's/Ed25519 ok/Ed25519 FAILED/')
    arm heap-79-of-80 red < <(printf '%s\n' "$GREEN" | sed 's#80/80 bump#79/80 bump#')
    arm fail-line red < <(printf '%s\n' "$GREEN" | sed 's/^  \[ok\]   an aligned/  [FAIL] an aligned/')
    arm exception red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] MANSA-PC BOOT OK$/[pc] EXCEPTION vector 14 (page fault) error 0x2 at RIP 0x1 (CR2 0x0) -- halted/')
    arm no-screen red < <(printf '%s\n' "$GREEN" | grep -v 'GOP framebuffer')
    # the verdict only in prose, not at column 0: must not green
    arm verdict-in-prose red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] MANSA-PC BOOT OK$/  would print [pc] MANSA-PC BOOT OK here/')
    arm empty-log red < /dev/null
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 14 ]
}

# ------------------------------------------------------------------- driver --
echo "== PC boot (suite 691, PC track step 1) =="
if ! selftest; then
    echo "PC BOOT TEST FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "PC BOOT SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || { echo "PC BOOT TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd \
         /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/ovmf/OVMF.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC BOOT TEST FAILED (harness: no x86-64 UEFI firmware (edk2-x86_64-code.fd / OVMF_CODE.fd) found)"; exit 1; }

echo "  --- build ---"
if ! cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2; then
    :
fi
EFI="target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$EFI" ] || { echo "PC BOOT TEST FAILED (harness: $EFI was not built)"; exit 1; }

W="$(mktemp -d "$T/pc-boot.XXXXXX")"
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
echo "  qemu exit $QRC ($([ "$QRC" -eq 124 ] && echo 'timed out -- the kernel never powered off' || echo 'the kernel powered off'))"
tr -d '\r' < "$SER" 2>/dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | grep -aE '^\[pc\]|^kernel heap: [0-9]|^ *\[FAIL\]' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$SER"; then
    rm -rf "$W"
    echo "PC BOOT TEST PASSED"
    exit 0
fi
echo "  serial kept at $SER"
echo "PC BOOT TEST FAILED"
exit 1
