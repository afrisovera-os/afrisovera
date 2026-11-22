#!/usr/bin/env bash
# pc-rt79: call 79 clock_monotonic for ring-3 processes, PC kernel (CLOCK_PLAN, the architect's verdict of 21 Nov 2026).
#
# One boot (-cpu max), then `rt79test`: the ring-3 program `rt79` (pc/proc.rs; no handles, call 79 needs none) prints,
# in order:
#   1. how many of 10,000 back-to-back calls returned less than the call before: 0 (never backwards);
#   2. how many of them were not a whole number of microseconds: 0 (the 1 us rounding, for every process);
#   3. the backwards count again after four more calls, each after a 50 ms SYS_SLEEP: 0 (across ticks);
#   4. the difference in ns of two calls around a 100 ms SYS_SLEEP: between 90 ms and 2 s (loose, for TCG);
#   5. the last value: above 0 and a whole number of microseconds.
# Then the frames check ("[rt79] 1 ended; frames ... (ok)"). The boot's `[timer]` line must name a measured source
# and say whether the TSC is invariant, and "time source rate assumed, not measured" must be absent on QEMU.
# The grader's selftest feeds it a good log and six bad ones first.
#   scripts/pc-rt79-test.sh              # grader selftest, then the build and the QEMU boot
#   scripts/pc-rt79-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"

grade() { # grade <serial log>
    local fail=0 S R
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    grep -aqE 'KERNEL PANIC|panicked|^\[pc\] (EXCEPTION|PANIC)' <<< "$S" && _f "the kernel stopped"
    grep -aqE '^\[timer\] TSC [0-9]+ MHz, measured against (the 8254 PIT|CPUID|the ACPI PM timer)[^;]*; invariant TSC: (yes|no)' <<< "$S" ||
        _f "no [timer] line naming a measured source and the invariant TSC"
    grep -aqF 'time source rate assumed, not measured' <<< "$S" && _f "the time source rate was assumed on QEMU"
    R="$(awk '/^afrisovera> rt79test/{n=1} n' <<< "$S")"
    local v
    mapfile -t v < <(grep -aoE '^\[P[0-9]\] 0x[0-9a-f]+$' <<< "$R" | cut -d' ' -f2)
    echo "  rt79 printed: ${v[*]:-nothing}"
    if [ "${#v[@]}" -ne 5 ]; then
        _f "rt79 printed ${#v[@]} values, wanted 5"
    else
        [ "${v[0]}" = 0x0 ] || _f "$(( v[0] )) of 10,000 calls went backwards"
        [ "${v[1]}" = 0x0 ] || _f "$(( v[1] )) of 10,000 values were not whole microseconds"
        [ "${v[2]}" = 0x0 ] || _f "the clock went backwards across the 50 ms sleeps ($(( v[2] )) in all)"
        local d=$(( v[3] )) last=$(( v[4] ))
        echo "  100 ms sleep measured as $(( d / 1000 )) us; last value $last ns"
        [ "$d" -ge 90000000 ] && [ "$d" -le 2000000000 ] || _f "the 100 ms sleep measured $d ns (want 90 ms to 2 s)"
        [ "$last" -gt 0 ] && [ $(( last % 1000 )) -eq 0 ] || _f "the last value $last is not a positive whole number of microseconds"
    fi
    grep -aqE '^\[rt79\] 1 ended; frames in use ([0-9]+) before and \1 after \(ok\)$' <<< "$R" || _f "frames not all back"
    grep -aq 'heap:' <<< "$R" || _f "the shell stopped answering after rt79test"
    return $fail
}

GOOD='[timer] TSC 2995 MHz, measured against the 8254 PIT; invariant TSC: yes
[pc] MANSA-PC BOOT OK
afrisovera> rt79test
[P0] 0x0
[P0] 0x0
[P0 @EL0] slept 50ms (asked 50ms)
[P0 @EL0] slept 50ms (asked 50ms)
[P0 @EL0] slept 50ms (asked 50ms)
[P0 @EL0] slept 50ms (asked 50ms)
[P0] 0x0
[P0 @EL0] slept 100ms (asked 100ms)
[P0] 0x5f5e3e8
[P0] 0x2540be400
[P0] exit 0
[rt79] 1 ended; frames in use 812 before and 812 after (ok)
afrisovera> mem
heap: 1048576 bytes'

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/pc-rt79-selftest.XXXXXX")" || return 1
    printf '%s' "$GOOD" > "$D/good"
    printf '%s' "$GOOD" | sed '0,/^\[P0\] 0x0$/s//[P0] 0x3/' > "$D/backwards"
    printf '%s' "$GOOD" | awk '/^\[P0\] 0x0$/ { c++; if (c == 2) { print "[P0] 0x2710"; next } } 1' > "$D/notus"
    printf '%s' "$GOOD" | sed 's/^\[P0\] 0x5f5e3e8$/[P0] 0x3b9ac8/' > "$D/short"
    printf '%s' "$GOOD" | sed 's/(ok)$/(WRONG)/; s/812 after/813 after/' > "$D/frames"
    printf '%s' "$GOOD" | sed '1a [timer] time source rate assumed, not measured' > "$D/assumed"
    printf '%s' "$GOOD" | sed 's/^\[P0\] 0x2540be400$/[P0] 0x2540be401/' > "$D/lastnotus"
    echo "  --- grader selftest (no QEMU) ---"
    if grade "$D/good" > /dev/null; then pass=$((pass + 1)); else echo "  FAIL selftest: the good log was refused"; grade "$D/good"; bad=$((bad + 1)); fi
    for f in backwards notus short frames assumed lastnotus; do
        if grade "$D/$f" > /dev/null; then echo "  FAIL selftest: the '$f' log was accepted"; bad=$((bad + 1)); else pass=$((pass + 1)); fi
    done
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ $bad -eq 0 ]
}

echo "== pc-rt79: call 79 clock_monotonic for ring-3 processes, PC kernel =="
selftest || { echo "PC RT79 TEST FAILED (the grader's selftest)"; exit 1; }
if [ "${1:-}" = "--selftest" ]; then
    echo "PC RT79 SELFTEST ONLY; no QEMU run spent"
    exit 0
fi
BL="$(mktemp)"
cargo build -p mansa-kernel --bin mansa-pc --features pc --target x86_64-unknown-uefi > "$BL" 2>&1
tail -2 "$BL"
export PC_EFI="${PC_EFI:-$PWD/target/x86_64-unknown-uefi/debug/mansa-pc.efi}"
if [ ! -r "$PC_EFI" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90
    echo "PC RT79 TEST FAILED (harness: $PC_EFI was not built)"
    exit 1
fi
export AFRISH_BOOT_QUIET="${AFRISH_BOOT_QUIET:-40}"
chmod +x scripts/lib/pc-arch/qemu-system-aarch64 2> /dev/null || true
PATH="$PWD/scripts/lib/pc-arch:$PATH"
W="$(mktemp -d "$T/pc-rt79.XXXXXX")"
# ports 6210/6211: not 6050/6051 (pc-rt3) or any other suite's
PC_CPU=max qemu-system-aarch64 -M virt -m 256M -display none \
    -serial "tcp:127.0.0.1:6210,server,nowait" -qmp "tcp:127.0.0.1:6211,server,nowait" &
QPID=$!
timeout 150 $PY -u scripts/lib/cell-keys.py 6210 6211 "$W" "rt79test" "mem" > "$W/boot.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[timer\]|^\[rt79\]' | sed 's/^/  | /'
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "PC RT79 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "PC RT79 TEST FAILED"
exit 1
