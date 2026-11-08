#!/usr/bin/env bash
# scripts/writer-guard.sh -- REFUSE TO BE THE SECOND WRITER (2.859).
#
# Usage: bash scripts/writer-guard.sh <results.tsv>
# Exit:  0  PROCEED  (nothing live is writing that file)
#        88 REFUSE   (a live writer holds it, or its liveness is UNKNOWN)
#
# THE GAP THIS CLOSES. 2.858 measured what prose could not prevent: EIGHTEEN live
# run-all-tests.sh, with eight different argument shapes, all appending to ONE
# .rat-results.2948.tsv -- 488 rows / 260 distinct / 40 suite names recording BOTH
# a PASS and a FAIL. Six consecutive dispatches each armed a runner believing it
# was the only one, and the gate armed on that file had no term that could see a
# second writer: it would have graded interleaved rows as confidently as clean
# ones. The handle has carried everything needed to stop this since 2.792
# (results=, pid=, winpid=, startedZ=). NOTHING read it at startup. This does.
#
# WHAT IT GRADES -- three relations, no typed-in pid:
#   1. a handle exists beside THIS results file and its WRITER row names it
#      (a handle about another file is not ours to read -> PROCEED);
#   2. it carries NO EXIT row (run-all-tests.sh's 2.858 trap stamps one, so a
#      closed handle belongs to a walk that has ENDED -> PROCEED);
#   3. its winpid is LIVE, asked of `arm-holders.sh --live`, which is the tasklist
#      form scripts/liveprobe-falsify-2831.sh falsified. 2.765 rule 5: CALL the
#      probe that already exists, never re-type it.
#
# UNKNOWN IS NOT DEAD, and here it is not GO either: --live exit 37 (the probe
# itself errored) refuses too. The flattering direction -- "could not tell, so
# start anyway" -- is the exact one that cost six dispatches.
#
# WHY A SCRIPT AND NOT A FUNCTION. It was written inline in run-all-tests.sh
# first. Grading it there meant a 12 s leash on a runner that spends ~60 s forking
# `basename` over 684 suites BEFORE the guard is reached -- so every arm of the
# first selftest was vacuous, PASSes included, and read as green. A guard with no
# callable boundary cannot be falsified cheaply, and a check that is expensive to
# falsify is one that stops being falsified.
set -u

RESULTS="${1:-}"
[ -n "$RESULTS" ] || { echo "GUARD-PROCEED: no results file given, nothing to guard"; exit 0; }

if [ "${RAT_ALLOW_CONCURRENT:-0}" = 1 ]; then
    echo "GUARD-PROCEED: RAT_ALLOW_CONCURRENT=1, single-writer guard skipped by typed opt-in"
    exit 0
fi

HANDLE="${RESULTS}.handle"
[ -f "$HANDLE" ] || { echo "GUARD-PROCEED: no ${HANDLE}; nothing claims this file"; exit 0; }
[ -f scripts/arm-holders.sh ] || { echo "GUARD-PROCEED: no scripts/arm-holders.sh to probe with"; exit 0; }

wrow="$(grep -E '^WRITER'$'\t' "$HANDLE" 2>/dev/null | tr -d '\r' | tail -1)"
[ -n "${wrow:-}" ] || { echo "GUARD-PROCEED: ${HANDLE} has no WRITER row"; exit 0; }

hbase="$(printf '%s\n' "$wrow" | tr '\t' '\n' | grep -m1 '^results=' | cut -d= -f2-)"
if [ "$(basename "${hbase:-}")" != "$(basename "$RESULTS")" ]; then
    echo "GUARD-PROCEED: ${HANDLE} claims results=${hbase:-}, not $(basename "$RESULTS"); not ours to read"
    exit 0
fi

if grep -qE '^EXIT'$'\t' "$HANDLE" 2>/dev/null; then
    echo "GUARD-PROCEED: ${HANDLE} carries an EXIT row; that walk has ended"
    exit 0
fi

out="$(bash scripts/arm-holders.sh --live "$RESULTS" 2>&1)"; rc=$?
case "$rc" in
    0)
        rows="$( [ -f "$RESULTS" ] && wc -l < "$RESULTS" | tr -d ' ' || echo 0 )"
        echo "GUARD-REFUSE: ${HANDLE} names a LIVE writer and has no EXIT row."
        echo "  handle: ${wrow}"
        echo "  probe : $(printf '%s\n' "$out" | tail -1)"
        echo "  ${RESULTS} holds ${rows} rows, all of them that walk's."
        echo "  Wait for it, or use --results <other.tsv>, or RAT_ALLOW_CONCURRENT=1 if you mean it."
        exit 88
        ;;
    37)
        echo "GUARD-REFUSE: liveness of ${HANDLE}'s writer is UNKNOWN, and UNKNOWN is not DEAD."
        echo "  probe : $(printf '%s\n' "$out" | tail -1)"
        exit 88
        ;;
    *)
        echo "GUARD-PROCEED: ${HANDLE} names no live writer (probe rc=${rc}): $(printf '%s\n' "$out" | tail -1)"
        exit 0
        ;;
esac
