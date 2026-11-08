#!/usr/bin/env bash
# scripts/close-out.sh -- THE NEXT-GENERATION CLOSE-OUT GUARD.
# Usage: ./scripts/close-out.sh <results.tsv> [verdict-cmd [args...]]
#
# 2.793 built scripts/handle-check.sh -- a consumer whose whole job is to be able
# to REFUSE -- and left it called by nobody (SESSION_PLAN 2.793 honest gap 2).
# This is the caller. It stands BEFORE the verdict, not beside it: the gate is
# never reached while the writer handle says the walk is still in flight, or that
# the row count and the file disagree.
#
# WHY A GUARD AND NOT A REWRITE (2.765 rule 5): every grading arm below belongs
# to the verdict command it is handed -- .rat-final-2766.sh and its chain,
# falsified 5/5. This file adds exactly ONE thing, the refusal, and then execs
# the verdict UNEDITED. It re-implements no gate.
#
# WHY IT LIVES IN scripts/ (2.793): commit 0762464 gitignores the whole .rat-*
# namespace, so a guard written as .rat-closeout-guard.sh would be untracked and
# invisible to the next agent. The thing that refuses must be the thing that ships.
#
# THE RELATION IT ENFORCES, stated plainly: a walk may only be graded by a
# process that can prove the writer is GONE and authored every row now in the
# file. Anything less -- no handle, a handle for another file, a live writer, a
# count that drifted -- is a refusal, never a pass-through.
#
# NO OVERRIDE EXISTS ON PURPOSE. An env var that skips the guard is the same
# flattering hole as 2.765's `gate | tail -25`: it turns a refusal into a zero
# for whoever is in a hurry. If the handle is wrong, fix the handle.
#
# EXIT CODES -- the 1x band is THIS guard refusing, and never overlaps the
# verdict's own 0/2/4/5/6/7/8/9/10, which are propagated verbatim:
#   0      the guard passed AND the verdict returned 0 (or, with no verdict
#          command given, the handle alone graded GREEN)
#   13     CLOSEOUT-REFUSED  handle-check HANDLE-EXIT-RED (writer exited rc!=0)
#   14     CLOSEOUT-REFUSED  HANDLE-VACUOUS (rc=0 but ZERO rows -- 3.663; a killed
#          runner stamps rc=0 too, so that rc alone is not a health signal)
#   15     CLOSEOUT-REFUSED  HANDLE-INFLIGHT (no EXIT row: live, or died un-exited)
#   16     CLOSEOUT-REFUSED  HANDLE-MISSING (no handle -- a pre-2.792 walk)
#   17     CLOSEOUT-REFUSED  HANDLE-ROWS (stamped count != wc -l: the 2.791 hole)
#   18     CLOSEOUT-REFUSED  HANDLE-FOREIGN (handle names a different results file)
#   19     CLOSEOUT-REFUSED  HANDLE-MALFORMED (present but unreadable)
#   20     CLOSEOUT-NO-RESULTS  the results file itself is absent
#   21     CLOSEOUT-NO-VERDICT  the verdict command is not executable
#   other  the verdict command's own exit code, untouched
set -u

HC="${CLOSEOUT_HANDLE_CHECK:-$(dirname "$0")/handle-check.sh}"

RESULTS="${1:-}"
[ -n "$RESULTS" ] || { echo "CLOSEOUT-NO-RESULTS: no results file given"; exit 20; }
shift

[ -f "$RESULTS" ] || { echo "CLOSEOUT-NO-RESULTS: no ${RESULTS}"; exit 20; }
[ -x "$HC" ] || [ -f "$HC" ] || { echo "CLOSEOUT-REFUSED: no handle checker at ${HC}"; exit 19; }

echo "== close-out guard: handle first, verdict second =="
bash "$HC" "$RESULTS"
hrc=$?

case "$hrc" in
    0) echo "GUARD PASSED: the writer is gone and authored every row in ${RESULTS}" ;;
    3)  echo "CLOSEOUT-REFUSED (13): the runner itself exited non-zero; nothing here is gradeable"; exit 13 ;;
    10) echo "CLOSEOUT-REFUSED (14): the writer stamped rc=0 having written NOTHING; a killed walk stamps that too"; exit 14 ;;
    5)  echo "CLOSEOUT-REFUSED (15): the walk has not exited -- grade it when it has"; exit 15 ;;
    6)  echo "CLOSEOUT-REFUSED (16): no writer handle; this walk cannot prove who wrote it"; exit 16 ;;
    7)  echo "CLOSEOUT-REFUSED (17): stamped row count and the file disagree (the 2.791 hole)"; exit 17 ;;
    8)  echo "CLOSEOUT-REFUSED (18): that handle belongs to a different results file"; exit 18 ;;
    9)  echo "CLOSEOUT-REFUSED (19): the handle is present but unreadable"; exit 19 ;;
    *)  echo "CLOSEOUT-REFUSED (19): handle-check returned an unknown rc=${hrc}"; exit 19 ;;
esac

if [ "$#" -eq 0 ]; then
    echo "CLOSEOUT-HANDLE-ONLY: no verdict command given; the handle alone is GREEN"
    exit 0
fi

VERDICT="$1"; shift
command -v "$VERDICT" >/dev/null 2>&1 || [ -f "$VERDICT" ] || {
    echo "CLOSEOUT-NO-VERDICT: ${VERDICT} is not runnable"; exit 21; }

echo "== verdict: ${VERDICT} $* (run UNEDITED, its exit code propagated) =="
# 2.818: the runnable CHECK above and the dispatch below must agree on what
# "runnable" means. They did not. `command -v python` succeeds, so an
# interpreter-shaped verdict (`close-out.sh <tsv> python scripts/rollup-report.py ...`
# -- the exact command pre-registered in docs/predict-2.810b.md) passed the
# check and was then handed to `bash`, which searches PATH for a SCRIPT named
# `python`, finds the interpreter binary, and dies `cannot execute binary file`
# rc=126 -- an undocumented code, outside both the guard's 1x band and the
# verdict's own set, produced WITHOUT the gate ever running. A close-out that
# cannot tell "the gate said no" from "the gate never ran" is worse than no
# close-out. Anything `command -v` resolves is now EXECUTED, not interpreted;
# only a name that resolves to nothing falls through to bash (a non-executable
# script path, the pre-2.818 convenience, which is preserved deliberately).
if [ -x "$VERDICT" ] || command -v "$VERDICT" >/dev/null 2>&1; then
    "$VERDICT" "$@"
else
    bash "$VERDICT" "$@"
fi
vrc=$?
echo "close-out: verdict rc=${vrc}"
exit "$vrc"
