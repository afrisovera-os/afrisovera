#!/usr/bin/env bash
# arm-cell3: the cell window for EL0 processes on ARM (AFR-0001 7.6, calls 64 to 68; Dira M3 S5b), the ARM side of
# pc-cell3. On ARM every cell call is served by the idle context in mainline (interrupts on): the call blocks the
# process, idle runs the desk call and hands the result back. The tick only hands the CPU to idle when the owner's
# 10 ms desk pass is due; nothing draws in an exception.
#
# Boot 1 has the test rig's fw_cfg flag (opt/afrisovera/test-grants), the desk shown. `celltest3` runs the EL0
# program `cell` (a CELL handle titled "celltest3 (test)", preemptible) and `nocell` (an endpoint only). Graded:
#   run A: the title; cell_open 0; a second open E_WOULDBLOCK (one owner); the size; a put of 4 cells (4); a put from
#     0x1000 (E_FAULT); 241 bytes (E_INVAL); a put on row 100 (0, clipped); the cursor (0); keys a and b as key
#     events; Ctrl+\ as a close request, and the program exits 7; the window closes; nocell's cell_open is E_PERM;
#   run B: the key s makes the program spin with no calls; Ctrl+\, then 3.5 s later Ctrl+\ again ("Close anyway"):
#     it stops within 100 ms, the window closes;
#   run C: the key w makes it ignore close requests while it waits; Close anyway then reaches it as kind 4 (the window
#     gone) and it ends; a CPU-bound third process runs throughout, and keys still arrive within two desk passes;
#   desk verify: 0 rows differ after the runs.
# Boot 2 has no flag: `celltest3` is refused.
set -u
cd "$(dirname "$0")/.."
T="${TMPDIR:-/tmp}"
PY="${PY:-python3}"
echo "== arm-cell3: the cell window for EL0 processes (calls 64 to 68), ARM kernel =="
BIN="target/aarch64-unknown-none/debug/mansa-kernel"
BL="$(mktemp)"
cargo build -p mansa-kernel > "$BL" 2>&1
tail -2 "$BL"
if [ ! -r "$BIN" ] || grep -q "^error" "$BL"; then
    grep -A14 "^error" "$BL" | head -90
    echo "ARM CELL3 TEST FAILED (harness: $BIN was not built)"
    exit 1
fi
W="$(mktemp -d "$T/arm-cell3.XXXXXX")"

boot() { # boot <log> <extra qemu args...> -- <steps...>
    local log="$1"; shift
    local q=()
    while [ "$1" != "--" ]; do q+=("$1"); shift; done
    shift
    qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
        -global virtio-mmio.force-legacy=false -device ramfb -display none "${q[@]}" \
        -serial "tcp:127.0.0.1:5992,server,nowait" -qmp "tcp:127.0.0.1:5993,server,nowait" -kernel "$BIN" &
    local qpid=$!
    timeout "$LIMIT" $PY -u scripts/lib/cell-keys.py 5992 5993 "$W" "$@" > "$log" 2>&1
    kill "$qpid" 2> /dev/null
    wait "$qpid" 2> /dev/null
}
LIMIT=250 boot "$W/rig.log" -fw_cfg name=opt/afrisovera/test-grants,string=1 -- \
    "gui" "desk mode pointer" \
    "&celltest3" "@opened by celltest3 (test)" "~2" "!cell" ":61" "~0.6" ":62" "~0.6" ":03" "~0.6" ":1c" "=" \
    "&celltest3" "@opened by celltest3 (test)" "~1" ":73" "~2" ":1c" "~3.5" ":1c" "=" \
    "&celltest3" "@opened by celltest3 (test)" "~1" ":77" "~1" ":1c" "~3.5" ":1c" "=" \
    "desk verify" "mem"
LIMIT=40 boot "$W/plain.log" -- "celltest3" "mem"

fail=0
_f() { echo "  FAIL: $1"; fail=1; }
clean() { tr -d '\r' < "$1" | sed 's/\x1b\[[0-9;?]*[A-Za-z]//g; s/^[[:space:]]*//'; }
S="$(clean "$W/rig.log")"
grep -aE '@EL0\]|^\[P[34]\]|^\[cell3\]|^\[desk\] (cell|verify)|EL0 fault' <<< "$S" | sed 's/^/  | /' | head -60
grep -aqE 'KERNEL PANIC|panicked' <<< "$S" && _f "the kernel panicked"
A="$(awk '/^afrisovera> celltest3/{n++} n==1' <<< "$S")"
B="$(awk '/^afrisovera> celltest3/{n++} n==2' <<< "$S")"
seq() { grep -aoE '\[P3 @EL0\] 0x[0-9a-f]+$' <<< "$1" | awk '{print $NF}' | tr '\n' ' ' | sed 's/ $//'; }
C="$(awk '/^afrisovera> celltest3/{n++} n==3' <<< "$S")"
SA="$(seq "$A")"; SB="$(seq "$B")"; SC="$(seq "$C")"
# the head ends with the deadline's 0 (a 300 ms cell_event with nothing typed). The ways out of BlockedCell
# (architect's S5b point 4): events (a, b), serial Ctrl+C as a key (3), the close request (kind 3), the deadline (0),
# Close anyway on a computing owner (run B, ended as a fault) and on a waiting one (run C, kind 4: the window gone)
HEAD_RE='^0x0 0xfffffffffffffff9 0x[1-9a-f][0-9a-f]* 0x4 0xfffffffffffffffc 0xfffffffffffffffb 0x0 0x0 0x0'
grep -qE "$HEAD_RE 0x10000061 0x10000062 0x10000003 0x30000000\$" <<< "$SA" || _f "run A printed [$SA]"
grep -qE "$HEAD_RE 0x10000073\$" <<< "$SB" || _f "run B printed [$SB]"
grep -qE "$HEAD_RE 0x10000077 0x30000000 0x40000000\$" <<< "$SC" || _f "run C printed [$SC] (Close anyway must reach a waiting owner as kind 4)"
grep -aqE '\[P3 @EL0\] exit' <<< "$C" || _f "run C: the owner did not end after kind 4"
grep -aqE '^\[P3\] stopped: Close anyway' <<< "$C" && _f "run C: a waiting owner was ended as a fault instead of getting kind 4"
GAP="$(sed -n 's/.*the longest gap between desk passes \([0-9]*\) ms.*/\1/p' <<< "$A" | head -1)"
MASK="$(sed -n 's/.*longest masked hand-over \([0-9]*\) us$/\1/p' <<< "$A" | head -1)"
IDLE="$(sed -n 's/^\[cell\] idle took \([0-9]*\)% of the CPU.*/\1/p' <<< "$A" | head -1)"
echo "  run A, with a CPU-bound second process: idle ${IDLE:-?}% of the CPU; longest gap between desk passes ${GAP:-?} ms; tick masked ${MASK:-?} us"
# a key waits at most one gap: two passes (20 ms) and a little
[ -n "$GAP" ] && [ "$GAP" -le 25 ] || _f "run A: a gap of ${GAP:-?} ms between desk passes (keys must arrive within two passes)"
[ -n "$MASK" ] && [ "$MASK" -lt 1000 ] || _f "run A: the tick held interrupts masked ${MASK:-?} us (bookkeeping only, under 1 ms)"
grep -aqF "[desk] cell: opened by celltest3 (test) (signed)" <<< "$A" || _f "run A: the window was not opened with the kernel's title"
grep -aqE '\[P3 @EL0\] exit' <<< "$A" || _f "run A: the close request did not end the program"
grep -aqE '^\[desk\] cell window closed: the app ended' <<< "$A" || _f "run A: the window did not close"
grep -aqE '\[P4 @EL0\] 0xffffffffffffffff$' <<< "$A" || _f "run A: nocell's cell_open was not refused (E_PERM)"
grep -aqE '^\[P3\] stopped: Close anyway' <<< "$B" || _f "run B: Close anyway did not stop the spinning program"
MS="$(sed -n 's/^\[desk\] cell: the app stopped \([0-9]*\) ms after Close anyway$/\1/p' <<< "$B" | head -1)"
echo "  run B: stopped ${MS:-?} ms after Close anyway"
[ -n "$MS" ] && [ "$MS" -lt 100 ] || _f "run B: stopped ${MS:-never} ms after Close anyway (the limit is 100 ms)"
grep -aqE '^\[cell3\] ended; .* map area 0\)$' <<< "$A" || _f "run A: the map area held frames after the run"
grep -aqE '^\[desk\] verify: 0 of [0-9]+ rows differ' <<< "$S" || _f "desk verify found rows that differ"
grep -aq "heap:" <<< "$S" || _f "the shell stopped answering (boot 1)"
P="$(clean "$W/plain.log")"
grep -aqF "[cell3] refused: only the test rig" <<< "$P" || _f "boot 2: celltest3 ran without the test rig's flag"
grep -aq "heap:" <<< "$P" || _f "the shell stopped answering (boot 2)"
[ -s "$W/cell.png" ] || _f "no screenshot of the window"

if [ "$fail" -eq 0 ]; then
    echo "  screenshot: $W/cell.png"
    echo "ARM CELL3 TEST PASSED"
    exit 0
fi
echo "  logs kept in $W"
echo "ARM CELL3 TEST FAILED"
exit 1
