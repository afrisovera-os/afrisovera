#!/usr/bin/env bash
# Phase 2.771 FALSIFIER — the `timeout N qemu; grep` shape, both ways, offline.
#
# WHAT IS BEING TESTED
# --------------------
# Not the kernel.  Not `pi-test.sh`.  The SHAPE that `pi` (timeout 6), `dhcp`
# (timeout 8) and `config` (timeout 7, twice) share — kill the guest at a
# constant, then grep the corpse — against the replacement shape: run the guest
# under a CEILING, wait on the guest's own output for the strings the suite
# asserts (afrish_wait_markers), then kill and grep ONE snapshot.
#
# WHY NO QEMU AND NO CARGO
# ------------------------
# The 684-suite walk is live.  A contending guest boot could red a row for no
# kernel reason — the exact defect under audit — so the guest is replaced by
# scripts/replay-guest-2771.py, which paces this walk's OWN row-3 capture
# (.rat-logs-2770/mansa-pi.log, 25 lines, a real raspi3b boot) and then idles
# forever like the real one.  Every number below is arithmetic on arrival times,
# not a simulation of QEMU.
#
# THE ARMS.  All five must be reached, and arms 1 and 5 exist precisely so that
# a harness rigged to fail cannot pass this falsifier.
#   1  OLD + fast guest        -> PASS      (the old shape is not broken per se)
#   2  OLD + slow guest        -> FAIL, and the message is one of pi-test.sh's
#                                 OWN named assertions — a harness defect wearing
#                                 the scheduler's name.  THIS IS THE DEFECT.
#   3  NEW + slow guest        -> PASS, returning on the guest's output well
#                                 inside the ceiling (not at it)
#   4  NEW + genuinely broken  -> FAIL at the ceiling, and STILL with the suite's
#      guest (timer ticks       own named message: the fix must not swallow a
#      dropped)                 real fault and report itself instead
#   5  NEW + fast guest        -> PASS in LESS wall clock than arm 1 burned:
#                                 the refund (103 s/walk across the class)
#
# EXIT
#   0  FALSIFY-OK        all five arms reached with the expected verdicts
#   1  FALSIFY-BROKEN    an arm did not do what the fix claims
#   2  cannot run (missing capture / python / lib)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 2

LIB="scripts/lib/afrish-boot.sh"
CAP=".rat-logs-2770/mansa-pi.log"
GUEST="scripts/replay-guest-2771.py"
[ -f "$LIB" ]   || { echo "NO-RUN: $LIB missing";   exit 2; }
[ -f "$CAP" ]   || { echo "NO-RUN: $CAP missing";   exit 2; }
[ -f "$GUEST" ] || { echo "NO-RUN: $GUEST missing"; exit 2; }
command -v python >/dev/null 2>&1 || { echo "NO-RUN: no python"; exit 2; }
# shellcheck source=scripts/lib/afrish-boot.sh
. "$LIB"

WORK="${TMPDIR:-/tmp}/qwf2771"
mkdir -p "$WORK"

# pi-test.sh's six assertions, verbatim, in its order.  Kept as ONE list so the
# two shapes cannot be graded against different bars — that is the only way the
# comparison means anything.
PI_RE=(
    "Pi MMU on"
    "hardware timebase OK"
    "Pi timer IRQ armed"
    "\[pi\] timer tick 1"
    "\[pi\] thread A tick"
    "\[pi\] thread B tick"
)
PI_MSG=(
    "PI TEST FAILED (MMU/boot)"
    "PI TEST FAILED (polled timebase)"
    "PI TEST FAILED (timer IRQ not armed)"
    "PI TEST FAILED (no interrupt-driven ticks)"
    "PI TEST FAILED (scheduler thread A never ran)"
    "PI TEST FAILED (scheduler thread B never ran)"
)

# grade <snapshot-file>  -> echoes the suite's verdict line, returns 0/1
grade() {
    local out="$1" i
    for i in "${!PI_RE[@]}"; do
        grep -qE "${PI_RE[$i]}" "$out" || { echo "${PI_MSG[$i]}"; return 1; }
    done
    echo "PI TEST PASSED  (raspi3b: MMU + heap + framebuffer + preemptive scheduler under timer IRQ)"
    return 0
}

now_ms() { date +%s%3N; }

# ---- shape OLD: kill at a constant, then grep -------------------------------
run_old() { # <pace_ms> <window_s> <tag> [drop_re]
    local pace="$1" win="$2" tag="$3" drop="${4:-}"
    local out="$WORK/$tag.log" t0 rc
    : >"$out"
    t0="$(now_ms)"
    REPLAY_LOG="$CAP" REPLAY_PACE_MS="$pace" REPLAY_DROP="$drop" \
        timeout "$win" python "$GUEST" -M raspi3b -kernel target/mansa-raspi.img \
        -serial stdio -display none >"$out" 2>&1
    ELAPSED_MS=$(( $(now_ms) - t0 ))
    VERDICT="$(grade "$out")"; rc=$?
    LINES="$(wc -l <"$out" | tr -d ' ')"
    return $rc
}

# ---- shape NEW: ceiling + wait on the guest's own output --------------------
run_new() { # <pace_ms> <ceiling_s> <tag> [drop_re]
    local pace="$1" ceil="$2" tag="$3" drop="${4:-}"
    local out="$WORK/$tag.log" t0 rc qpid
    : >"$out"
    t0="$(now_ms)"
    REPLAY_LOG="$CAP" REPLAY_PACE_MS="$pace" REPLAY_DROP="$drop" \
        timeout "$ceil" python "$GUEST" -M raspi3b -kernel target/mansa-raspi.img \
        -serial stdio -display none >"$out" 2>&1 &
    qpid=$!
    WAITLINE="$(afrish_wait_markers "$out" "$ceil" "${PI_RE[@]}")"
    kill "$qpid" 2>/dev/null
    wait "$qpid" 2>/dev/null
    ELAPSED_MS=$(( $(now_ms) - t0 ))
    VERDICT="$(grade "$out")"; rc=$?
    LINES="$(wc -l <"$out" | tr -d ' ')"
    return $rc
}

FAILED=0
note() { printf '  %s\n' "$*"; }
expect() { # <what> <got-rc> <want-rc>
    if [ "$2" -eq "$3" ]; then note "EXPECTED: $1"; else
        note "UNEXPECTED: $1 (rc=$2, wanted $3)"; FAILED=1; fi
}

echo "=== 2.771 falsifier: the timeout-window shape, offline (capture: $CAP, $(wc -l <"$CAP" | tr -d ' ') lines) ==="

# ARM 1 ----------------------------------------------------------------------
echo
echo "[arm 1] OLD shape (timeout 6, kill, grep) + FAST guest (40 ms/line)"
run_old 40 6 arm1; rc=$?
note "elapsed ${ELAPSED_MS} ms, $LINES lines captured"
note "verdict: $VERDICT"
expect "arm 1 PASSES — the old shape is not broken per se" "$rc" 0
ARM1_MS=$ELAPSED_MS

# ARM 2 ----------------------------------------------------------------------
# THE PACE WAS MEASURED, NOT DERIVED — and the first derivation was wrong, in
# the flattering direction, which is why the number is written down here with
# its history instead of being quietly retuned:
#   350 ms/line was chosen by arithmetic — 19 lines x 0.35 s = 6.65 s, past a
#   6 s window, so the last asserted line (`[pi] thread B tick 1`, line 19 of
#   25) should have been lost.  It was NOT: the run captured all 19 and PASSED.
#   The reason is that the constant is not the window.  `timeout 6` around this
#   guest measured 7135 / 7192 ms wall on this host — MSYS spends ~1.1-1.2 s in
#   the fork/reap around the child, and the child keeps printing through it.  A
#   suite that says `timeout 6` is grading against something closer to 7.2 s,
#   and neither number is written anywhere in the suite.
#   500 ms/line puts the last asserted line at 9.0 s of printing, past ANY
#   plausible reading of a 6 s constant, so the arm no longer depends on knowing
#   which reading is right.
echo
echo "[arm 2] OLD shape (timeout 6, kill, grep) + SLOW guest (500 ms/line)"
run_old 500 6 arm2; rc=$?
note "elapsed ${ELAPSED_MS} ms, $LINES lines captured (of 25)"
note "verdict: $VERDICT"
expect "arm 2 FAILS — the window closed on a correct guest" "$rc" 1
case "$VERDICT" in
    "PI TEST FAILED ("*)
        note "AND the message is pi-test.sh's own: a HARNESS defect reported under the KERNEL's name" ;;
    *) note "UNEXPECTED: arm 2 failed with a message pi-test.sh would never print"; FAILED=1 ;;
esac

# ARM 3 ----------------------------------------------------------------------
echo
echo "[arm 3] NEW shape (ceiling 60, wait on the asserted set, kill, one snapshot) + the SAME slow guest"
run_new 500 60 arm3; rc=$?
note "elapsed ${ELAPSED_MS} ms, $LINES lines captured"
note "$WAITLINE"
note "verdict: $VERDICT"
expect "arm 3 PASSES — a slow-but-correct guest is a slow PASS, not a red" "$rc" 0
if [ "$ELAPSED_MS" -lt 60000 ]; then
    note "AND it returned on the guest's OUTPUT, not on the ceiling (${ELAPSED_MS} ms of a 60000 ms ceiling)"
else
    note "UNEXPECTED: arm 3 burned the whole ceiling — the ceiling is still a window"; FAILED=1
fi

# ARM 4 ----------------------------------------------------------------------
echo
echo "[arm 4] NEW shape (ceiling 4) + a GENUINELY BROKEN guest (timer ticks never printed)"
run_new 40 4 arm4 '\[pi\] timer tick'; rc=$?
note "elapsed ${ELAPSED_MS} ms, $LINES lines captured"
note "$WAITLINE"
note "verdict: $VERDICT"
expect "arm 4 FAILS — a real fault is still a fail" "$rc" 1
if [ "$VERDICT" = "PI TEST FAILED (no interrupt-driven ticks)" ]; then
    note "AND it fails under the SUITE's name, not the wait's: the ceiling did not swallow the fault"
else
    note "UNEXPECTED: arm 4's message is '$VERDICT', not the suite's own"; FAILED=1
fi

# ARM 5 ----------------------------------------------------------------------
echo
echo "[arm 5] NEW shape (ceiling 60) + FAST guest (40 ms/line) — the refund"
run_new 40 60 arm5; rc=$?
note "elapsed ${ELAPSED_MS} ms, $LINES lines captured"
note "$WAITLINE"
note "verdict: $VERDICT"
expect "arm 5 PASSES" "$rc" 0
if [ "$ELAPSED_MS" -lt "$ARM1_MS" ]; then
    note "AND it cost ${ELAPSED_MS} ms where the fixed window burned ${ARM1_MS} ms — refund $(( ARM1_MS - ELAPSED_MS )) ms on ONE suite"
else
    note "UNEXPECTED: the new shape is not cheaper on a fast host (${ELAPSED_MS} vs ${ARM1_MS} ms)"; FAILED=1
fi

echo
if [ "$FAILED" -eq 0 ]; then
    echo "FALSIFY-OK  (5/5 arms reached; the window is a defect and the ceiling+read repairs it, zero boots spent)"
    exit 0
fi
echo "FALSIFY-BROKEN  (an arm did not do what the fix claims — do NOT apply the fix)"
exit 1
