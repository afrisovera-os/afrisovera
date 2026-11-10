#!/usr/bin/env bash
# PC IPC + SCHEDULER check (suite 694, PC track step 1: processes, capabilities, IPC and the
# timer scheduler on x86-64, 4 Nov 2026 -- the Phase 0 proof, on a PC).
#
# Same boot as suites 691-693: `mansa-pc` as EFI/BOOT/BOOTX64.EFI, QEMU q35 + EDK II (OVMF),
# graded from the serial log. This suite grades kernel/src/pc/proc.rs.
#   IPC -- P0 holds only a RECV capability on endpoint 0, P1 only a SEND capability:
#   1. P0 runs first and blocks in RECV; P1 sends 0x42; P0 prints 0x42 only AFTER P1 has
#      exited -- the message crossed address spaces through the kernel, and P0 was blocked;
#   2. every request without the right is refused BY THE KERNEL, by name: P1's RECV on 0,
#      P0's SEND on 0, P0's RECV on handle 2 (no capability there at all);
#   3. isolation: both wrote a mark at the SAME virtual address; each reads back its own
#      (P1 0x5e4d, P0 0xec7) -- separate address spaces, not one shared page;
#   4. the kernel's summary: 2 exited, 3 refused, 0 faults (ok).
#   SCHEDULER -- two workers that never yield, each waiting to be preempted 3 times:
#   5. both print a slice count >= 3 and exit; the kernel counts >= 2 timer switches
#      between them (ok) -- only preemption can let both finish.
#   ENDPOINTS (review R1-10, 8 Nov 2026) -- P0 waits on endpoint 1, P1 on endpoint 2; P2 sends 0xb2
#   on endpoint 2, P3 0xa1 on endpoint 1; P4 holds a capability past the last endpoint:
#   6. P0 prints 0xa1 and P1 prints 0xb2 (a SEND that woke any receiver gives them crossed);
#      P4's RECV is refused; the kernel's summary: 5 exited, 1 refused, 0 faults (ok).
#   7. no fault, no deadlock, no EXCEPTION/PANIC/[FAIL]; MANSA-PC BOOT OK.
# Every line is read at column 0 after stripping CR and escape codes (the rxsplit lesson).
# A grader selftest runs first on canned logs, both directions (the 2.767 pattern).
# Usage: scripts/pc-ipc-test.sh            selftest, then build + boot
#        scripts/pc-ipc-test.sh --selftest selftest only
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
    has() { printf '%s\n' "$L" | grep -aqE "$1"; }
    line() { printf '%s\n' "$L" | grep -anE "$1" | head -1 | cut -d: -f1; }

    local LX LM
    LX="$(line '^\[P1\] exit 0$')"
    LM="$(line '^\[P0\] 0x42$')"
    if [ -z "$LM" ]; then
        _f "P0 never received 0x42 from P1"
    elif [ -z "$LX" ] || [ "$LM" -lt "$LX" ]; then
        _f "P0 printed 0x42 before P1 exited -- it was never blocked waiting for the message"
    fi
    has '^\[P1\] RECV on handle 0 refused: no RECV capability$' ||
        _f "P1's RECV on its send-only handle was not refused"
    has '^\[P0\] SEND on handle 0 refused: no SEND capability$' ||
        _f "P0's SEND on its receive-only handle was not refused"
    has '^\[P0\] RECV on handle 2 refused: no RECV capability$' ||
        _f "P0's RECV on a handle it holds no capability for was not refused"
    has '^\[P1\] 0x5e4d$' || _f "P1 did not read back its own mark (0x5e4d)"
    has '^\[P0\] 0xec7$' || _f "P0 did not read back its own mark (0xec7) -- address spaces not separate"
    has '^\[pc\] IPC: 2 processes in separate address spaces, 2 exited, 3 requests refused for lack of a capability, 0 faults \(ok\)$' ||
        _f "the kernel's IPC summary is not '2 exited, 3 refused, 0 faults (ok)'"

    local SL SW n v
    SL="$(printf '%s\n' "$L" | grep -aE '^\[pc\] scheduler: 2 workers time-sliced by the APIC timer, [0-9]+ switches between them, 2 exited \(ok\)$' | tail -1)"
    if [ -z "$SL" ]; then
        _f "no scheduler line with both workers exited (ok)"
    else
        SW="$(printf '%s' "$SL" | sed -E 's/^.*timer, ([0-9]+) switches.*$/\1/')"
        [ "$SW" -ge 2 ] || _f "only $SW timer switch(es) between the workers"
    fi
    # the workers' own counts: after the IPC summary, each prints one number >= 3
    n=0
    while read -r v; do
        [ $(( v )) -ge 3 ] && n=$((n+1))
    done < <(printf '%s\n' "$L" | sed -n '/^\[pc\] IPC: /,/^\[pc\] scheduler: /p' | grep -aoE '^\[P[0-9]\] 0x[0-9a-f]+$' | awk '{print $2}')
    [ "$n" -ge 2 ] || _f "only $n worker(s) reported being preempted 3 or more times"

    # the endpoint check runs after the scheduler: its lines are read from its own stretch
    local E
    E="$(printf '%s
' "$L" | sed -n '/^\[pc\] scheduler: /,/^\[pc\] IPC endpoints: /p')"
    printf '%s
' "$E" | grep -aqE '^\[P0\] 0xa1$' ||
        _f "P0 (waiting on endpoint 1) did not get 0xa1 -- endpoints are not separate"
    printf '%s
' "$E" | grep -aqE '^\[P1\] 0xb2$' ||
        _f "P1 (waiting on endpoint 2) did not get 0xb2 -- endpoints are not separate"
    printf '%s
' "$E" | grep -aqE '^\[P4\] RECV on handle 0 refused: no RECV capability$' ||
        _f "P4's capability past the last endpoint was not refused"
    has '^\[pc\] IPC endpoints: [0-9]+ endpoints, 2 messages on 2 of them, 5 exited, 1 refused \(a capability past the last endpoint\), 0 faults \(ok\)$' ||
        _f "the kernel's endpoint summary is not '5 exited, 1 refused, 0 faults (ok)'"

    if has '^\[P[0-9]\] fault:'; then _f "a process faulted: $(printf '%s\n' "$L" | grep -aE '^\[P[0-9]\] fault:' | head -1)"; fi
    if has '^\[proc\] .*deadlock'; then _f "processes deadlocked"; fi
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
GREEN="[pc] ring 3: program 3 ran a privileged instruction (cli) -> general protection fault, program stopped (ok)
[P1] RECV on handle 0 refused: no RECV capability
[P1] 0x5e4d
[P1] exit 0
[P0] 0x42
[P0] SEND on handle 0 refused: no SEND capability
[P0] RECV on handle 2 refused: no RECV capability
[P0] 0xec7
[P0] exit 0
[pc] IPC: 2 processes in separate address spaces, 2 exited, 3 requests refused for lack of a capability, 0 faults (ok)
[P0] 0x3
[P0] exit 0
[P1] 0x3
[P1] exit 0
[pc] scheduler: 2 workers time-sliced by the APIC timer, 6 switches between them, 2 exited (ok)
[P2] exit 0
[P3] exit 0
[P4] RECV on handle 0 refused: no RECV capability
[P4] 0xffffffffffffffff
[P4] exit 0
[P0] 0xa1
[P0] exit 0
[P1] 0xb2
[P1] exit 0
[pc] IPC endpoints: 4 endpoints, 2 messages on 2 of them, 5 exited, 1 refused (a capability past the last endpoint), 0 faults (ok)
[pc] MANSA-PC BOOT OK
[pc] powering off (requested by the test rig through fw_cfg)"

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-ipc-selftest.XXXXXX")" || { echo "  SELFTEST: mktemp failed"; return 1; }
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
    arm no-message red < <(printf '%s\n' "$GREEN" | grep -v '^\[P0\] 0x42$')
    arm never-blocked red < <(printf '%s\n' "$GREEN" | awk '/^\[P0\] 0x42$/ {next} /^\[P1\] RECV on handle 0/ {print "[P0] 0x42"} {print}')
    arm send-only-could-recv red < <(printf '%s\n' "$GREEN" | grep -v '^\[P1\] RECV on handle 0 refused')
    arm recv-only-could-send red < <(printf '%s\n' "$GREEN" | grep -v '^\[P0\] SEND on handle 0 refused')
    arm no-cap-handle-allowed red < <(printf '%s\n' "$GREEN" | grep -v 'RECV on handle 2 refused')
    arm shared-page red < <(printf '%s\n' "$GREEN" | sed 's/^\[P0\] 0xec7$/[P0] 0x5e4d/')
    arm summary-wrong red < <(printf '%s\n' "$GREEN" | sed 's/3 requests refused/2 requests refused/')
    arm no-switches red < <(printf '%s\n' "$GREEN" | sed 's/, 6 switches between/, 0 switches between/')
    arm worker-not-preempted red < <(printf '%s\n' "$GREEN" | sed '/^\[pc\] IPC: /,$ s/^\[P1\] 0x3$/[P1] 0x1/')
    arm worker-missing red < <(printf '%s\n' "$GREEN" | sed 's/time-sliced by the APIC timer, 6 switches between them, 2 exited (ok)/time-sliced by the APIC timer, 6 switches between them, 1 exited (WRONG)/')
    arm process-fault red < <(printf '%s\n' "$GREEN" | sed 's/^\[P0\] exit 0$/[P0] fault: vector 14 at RIP 0x10000000004 (address 0x0) -- process stopped/')
    arm deadlock red < <(printf '%s\n' "$GREEN" | sed 's/^\[pc\] MANSA-PC BOOT OK$/[proc] 1 process(es) blocked forever on IPC -- deadlock, stopping them\n[pc] MANSA-PC BOOT OK/')
    arm verdict-in-prose red < <(printf '%s\n' "$GREEN" | sed 's/^\[P0\] 0x42$/  [P0] 0x42/')
    arm endpoints-crossed red < <(printf '%s
' "$GREEN" | sed 's/^\[P0\] 0xa1$/[P0] 0xb2/; s/^\[P1\] 0xb2$/[P1] 0xa1/')
    arm past-last-endpoint-allowed red < <(printf '%s
' "$GREEN" | grep -v '^\[P4\] RECV on handle 0 refused')
    arm endpoint-summary-wrong red < <(printf '%s
' "$GREEN" | sed 's/5 exited, 1 refused (a capability/4 exited, 1 refused (a capability/')
    arm empty-log red < /dev/null
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ "$bad" -eq 0 ] && [ "$pass" -eq 19 ]
}

# ------------------------------------------------------------------- driver --
echo "== PC IPC + scheduler (suite 694, PC track step 1) =="
if ! selftest; then
    echo "PC IPC TEST FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi
[ "${1:-}" = "--selftest" ] && { echo "PC IPC SELFTEST ONLY -- no measurement taken"; exit 0; }

command -v qemu-system-x86_64 > /dev/null || { echo "PC IPC TEST FAILED (harness: qemu-system-x86_64 not on PATH)"; exit 1; }
QDIR="$(dirname "$(command -v qemu-system-x86_64)")"
OVMF=""
for f in "$QDIR/../share/qemu/edk2-x86_64-code.fd" "$QDIR/share/edk2-x86_64-code.fd" \
         /usr/share/qemu/edk2-x86_64-code.fd /usr/share/OVMF/OVMF_CODE.fd \
         /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/ovmf/OVMF.fd; do
    if [ -r "$f" ]; then OVMF="$f"; break; fi
done
[ -n "$OVMF" ] || { echo "PC IPC TEST FAILED (harness: no x86-64 UEFI firmware (edk2-x86_64-code.fd / OVMF_CODE.fd) found)"; exit 1; }

echo "  --- build ---"
if ! cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi 2>&1 | tail -2; then
    :
fi
EFI="target/x86_64-unknown-uefi/debug/mansa-pc.efi"
[ -r "$EFI" ] || { echo "PC IPC TEST FAILED (harness: $EFI was not built)"; exit 1; }

W="$(mktemp -d "$T/pc-ipc.XXXXXX")"
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
tr -d '\r' < "$SER" 2>/dev/null | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g' | grep -aE '^\[(pc|user|proc|P[0-9])\]|^kernel heap: [0-9]|^ *\[FAIL\]' | sed 's/^/  | /'

echo "  --- grading ---"
if grade "$SER"; then
    rm -rf "$W"
    echo "PC IPC TEST PASSED"
    exit 0
fi
echo "  serial kept at $SER"
echo "PC IPC TEST FAILED"
exit 1
