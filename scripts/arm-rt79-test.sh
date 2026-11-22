#!/usr/bin/env bash
# arm-rt79: call 79 clock_monotonic for EL0 processes, ARM kernel; the ARM side of pc-rt79 (CLOCK_PLAN, the
# architect's verdict of 21 Nov 2026).
#
# One boot, then `rt79test`: the EL0 program `rt79` (kernel/src/process.rs; no handles, call 79 needs none) prints,
# in order:
#   1. how many of 10,000 back-to-back calls returned less than the call before: 0 (never backwards);
#   2. how many of them were not a whole number of microseconds: 0 (the 1 us rounding, for every process);
#   3. the backwards count again after four more calls, each after a 50 ms SYS_SLEEP: 0 (across ticks);
#   4. the difference in ns of two calls around a 100 ms SYS_SLEEP: between 90 ms and 2 s (loose, for TCG);
#   5. the last value: above 0 and a whole number of microseconds.
# Then the frames check ("[rt79] 1 ended; frames ... (ok)"). The counter is CNTPCT_EL0 at CNTFRQ_EL0, read by the
# kernel; what EL0 may read itself is unchanged. The grader's selftest feeds it a good log and five bad ones first.
#   scripts/arm-rt79-test.sh              # grader selftest, then the build and the QEMU boot
#   scripts/arm-rt79-test.sh --selftest   # grader selftest only
set -uo pipefail
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"

grade() { # grade <serial log>
    local fail=0 S R
    _f() { echo "  FAIL: $*"; fail=1; }
    [ -r "$1" ] || { echo "  FAIL: missing serial log"; return 1; }
    S="$(tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//')"
    grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel stopped"
    R="$(awk '/^afrisovera> rt79test/{n=1} n' <<< "$S")"
    local v
    mapfile -t v < <(grep -aoE '\[P3 @EL0\] 0x[0-9a-f]+$' <<< "$R" | awk '{print $NF}')
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

GOOD='afrisovera> rt79test
[P3 @EL0] 0x0
[P3 @EL0] 0x0
[P3 @EL0] slept 50ms (asked 50ms)
[P3 @EL0] slept 50ms (asked 50ms)
[P3 @EL0] slept 50ms (asked 50ms)
[P3 @EL0] slept 50ms (asked 50ms)
[P3 @EL0] 0x0
[P3 @EL0] slept 100ms (asked 100ms)
[P3 @EL0] 0x5f5e3e8
[P3 @EL0] 0x2540be400
[P3 @EL0] exit 0
[rt79] 1 ended; frames in use 812 before and 812 after (ok)
afrisovera> mem
heap: 1048576 bytes'

selftest() {
    local D pass=0 bad=0
    D="$(mktemp -d "$T/arm-rt79-selftest.XXXXXX")" || return 1
    printf '%s' "$GOOD" > "$D/good"
    printf '%s' "$GOOD" | sed '0,/^\[P3 @EL0\] 0x0$/s//[P3 @EL0] 0x3/' > "$D/backwards"
    printf '%s' "$GOOD" | awk '/^\[P3 @EL0\] 0x0$/ { c++; if (c == 2) { print "[P3 @EL0] 0x2710"; next } } 1' > "$D/notus"
    printf '%s' "$GOOD" | sed 's/^\[P3 @EL0\] 0x5f5e3e8$/[P3 @EL0] 0x3b9ac8/' > "$D/short"
    printf '%s' "$GOOD" | sed 's/(ok)$/(WRONG)/; s/812 after/813 after/' > "$D/frames"
    printf '%s' "$GOOD" | sed 's/^\[P3 @EL0\] 0x2540be400$/[P3 @EL0] 0x2540be401/' > "$D/lastnotus"
    echo "  --- grader selftest (no QEMU) ---"
    if grade "$D/good" > /dev/null; then pass=$((pass + 1)); else echo "  FAIL selftest: the good log was refused"; grade "$D/good"; bad=$((bad + 1)); fi
    for f in backwards notus short frames lastnotus; do
        if grade "$D/$f" > /dev/null; then echo "  FAIL selftest: the '$f' log was accepted"; bad=$((bad + 1)); else pass=$((pass + 1)); fi
    done
    rm -rf "$D"
    echo "  grader selftest: $pass ok / $bad FAIL"
    [ $bad -eq 0 ]
}

echo "== arm-rt79: call 79 clock_monotonic for EL0 processes, ARM kernel =="
selftest || { echo "ARM RT79 TEST FAILED (the grader's selftest)"; exit 1; }
if [ "${1:-}" = "--selftest" ]; then
    echo "ARM RT79 SELFTEST ONLY; no QEMU run spent"
    exit 0
fi
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
BL="$(mktemp)"
cargo build -p mansa-kernel > "$BL" 2>&1
tail -2 "$BL"
if [ ! -r "$BIN" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90
    echo "ARM RT79 TEST FAILED (harness: $BIN was not built)"
    exit 1
fi
W="$(mktemp -d "$T/arm-rt79.XXXXXX")"
# ports 6212/6213: not 6078/6079 (arm-rt3) or any other suite's
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial "tcp:127.0.0.1:6212,server,nowait" -qmp "tcp:127.0.0.1:6213,server,nowait" -kernel "$BIN" &
QPID=$!
timeout 150 $PY -u scripts/lib/cell-keys.py 6212 6213 "$W" "rt79test" "mem" > "$W/boot.log" 2>&1
kill "$QPID" 2> /dev/null
wait "$QPID" 2> /dev/null
tr -d '\r' < "$W/boot.log" | grep -aE '^\[rt79\]' | sed 's/^/  | /'
if grade "$W/boot.log"; then
    rm -rf "$W"
    echo "ARM RT79 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "ARM RT79 TEST FAILED"
exit 1
