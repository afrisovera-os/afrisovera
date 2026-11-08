#!/usr/bin/env bash
# scripts/handle-check.sh -- READ the writer handle that 2.792 started stamping.
# Usage: ./.rat-handle-2792.sh <results.tsv> [handle]
#
# 2.792 taught the WRITER to claim its own pid in ${RESULTS}.handle, re-stamped
# after every append. It published that claim to NO CONSUMER (SESSION_PLAN's
# honest gap 3). This is the consumer, and its whole job is to be able to REFUSE.
#
# WHAT IT GRADES -- three relations, none of them a typed-in pid:
#   1. the handle's WRITER row names THIS results file (not a stale generation),
#   2. the handle's rows= agrees with `wc -l < $RESULTS` (so ONE process
#      authored both the count and the rows -- that is the 2.791 hole),
#   3. an EXIT row exists and carries the runner's rc. ABSENCE IS EVIDENCE:
#      no EXIT row means the writer never exited, which 2.791 could not tell
#      apart from a clean exit.
#
# THE ONE TOLERANCE, stated not hidden: stamp_handle runs AFTER the append, so a
# reader can catch the writer in the window between the two and see rows= one
# short. A lag of exactly 1 on a NON-EXIT event is therefore reported as LAGGING,
# not as a mismatch. On an EXIT event the count must be EXACT -- the writer is
# gone, there is no window left for it to be in.
#
# EXIT CODES (refusing is always available; fabricating a zero is not):
#   0 HANDLE-GREEN     rows agree, EXIT row present, rc=0
#   3 HANDLE-EXIT-RED  rows agree, EXIT row present, rc!=0 (message notes whether
#                      rows=0 -- a kill, per 3.664's SIGHUP measurement -- or
#                      rows=N>0 -- a completed run that reported failure)
#   5 HANDLE-INFLIGHT  no EXIT row -- writer still running, or it died un-exited
#   6 HANDLE-MISSING   no handle file at all
#   7 HANDLE-ROWS      rows= and `wc -l` disagree by more than the append window
#   8 HANDLE-FOREIGN   the handle's WRITER row names a DIFFERENT results file
#   9 HANDLE-MALFORMED present but unreadable (no WRITER row / no pid / no rows)
#  10 HANDLE-VACUOUS   EXIT row says rc=0 but the walk authored ZERO rows -- 3.663.
#                      rc=0 is not a health signal on its own (a KILLED runner
#                      stamps it too, measured at 3.660), so a green verdict now
#                      also requires that at least one row was actually written.
set -u

RESULTS="${1:-}"
[ -n "$RESULTS" ] || { echo "HANDLE-MALFORMED: no results file given"; exit 9; }
HANDLE="${2:-${RESULTS}.handle}"

[ -f "$HANDLE" ] || { echo "HANDLE-MISSING: no ${HANDLE}"; exit 6; }

wrow="$(grep -m1 '^WRITER	' "$HANDLE" || true)"
[ -n "$wrow" ] || { echo "HANDLE-MALFORMED: no WRITER row in ${HANDLE}"; exit 9; }

field() { printf '%s\n' "$1" | tr '\t' '\n' | grep -m1 "^$2=" | cut -d= -f2- || true; }

h_results="$(field "$wrow" results)"
h_pid="$(field "$wrow" pid)"
h_start="$(field "$wrow" startedZ)"
[ -n "$h_pid" ] || { echo "HANDLE-MALFORMED: WRITER row has no pid"; exit 9; }

# The handle is DERIVED from $RESULTS, so the two names must agree on basename.
# Compare basenames: the writer stores the path it was invoked with, a reader may
# hold the same file under a different prefix.
if [ "$(basename "$h_results")" != "$(basename "$RESULTS")" ]; then
    echo "HANDLE-FOREIGN: handle claims results=${h_results}, asked about ${RESULTS}"
    exit 8
fi

# 2.837: LAST, not FIRST. `grep -m1` stops at the first match, so on a handle that
# is APPENDED to (any writer that stamps per row) it graded row 1's count against
# a file holding hundreds -- HANDLE-ROWS (7) on a perfectly healthy walk. `tail -1`
# is correct for both shapes: rewritten-whole handles hold exactly one such row.
erow="$(grep -E '^EXIT	' "$HANDLE" | tail -1 || true)"
prow="$(grep -E '^(PROGRESS|START)	' "$HANDLE" | tail -1 || true)"
row="${erow:-$prow}"
[ -n "$row" ] || { echo "HANDLE-MALFORMED: no START/PROGRESS/EXIT row"; exit 9; }

h_rows="$(field "$row" rows)"
h_detail="$(field "$row" detail)"
case "$h_rows" in ''|*[!0-9]*) echo "HANDLE-MALFORMED: rows='${h_rows}'"; exit 9;; esac

if [ -f "$RESULTS" ]; then
    f_rows="$(wc -l < "$RESULTS" | tr -d ' ')"
else
    f_rows=0
fi

lag=$(( f_rows - h_rows ))
echo "handle: pid=${h_pid} startedZ=${h_start} rows=${h_rows} file_rows=${f_rows} detail=${h_detail}"

if [ -n "$erow" ]; then
    # Writer is gone: the count must be EXACT.
    if [ "$lag" -ne 0 ]; then
        echo "HANDLE-ROWS: EXIT stamped rows=${h_rows} but ${RESULTS} holds ${f_rows}"
        exit 7
    fi
    rc="$(printf '%s\n' "$h_detail" | sed -n 's/^rc=\([0-9][0-9]*\)$/\1/p')"
    [ -n "$rc" ] || { echo "HANDLE-MALFORMED: EXIT detail='${h_detail}' is not rc=N"; exit 9; }
    # 3.663: rc=0 IS NOT A HEALTH SIGNAL ON ITS OWN. Measured on a REAL SIGTERMed
    # runner (3.660): a KILLED walk stamps `EXIT rows=0 detail=rc=0 holders=self`,
    # because selfclose_handle records $? -- whatever the last command happened to
    # return -- and not a "this walk was killed" fact. Graded by the rc alone, that
    # handle came back HANDLE-GREEN, indistinguishable from walk 3600's 689-row
    # green close. Two identical kills stamping rc=0 and rc=1 (3.660 arms
    # termearly/term) is the proof the field is anti-correlated with health, not
    # merely noisy.
    #
    # THE RELATION ADDED: a GREEN close must have authored at least one row. That
    # is the cheapest fact that a kill cannot fake, because it is read from the
    # RESULTS FILE (and cross-checked against the stamped count above), not from
    # the handle's self-report. It costs the 689-suite close-out path nothing --
    # that walk holds 689 rows -- and it cannot alter any walk's BEHAVIOUR, only a
    # verdict: this is the consumer half, deliberately chosen over teaching the
    # runner to trap TERM/INT/HUP, which would make bash CATCH the signal and so
    # change what a killed walk DOES, on the one script a 689-suite walk needs.
    #
    # HONEST RESIDUAL, stated here and not left for a later reader to discover:
    # this catches every kill shape 3.660 observed (both wrote 0 rows). It does
    # NOT catch a kill at row N>0 that happens to leave $?=0. That shape is
    # reachable and is NOT closed -- it needs the producer half (queue item 1b).
    if [ "$rc" = 0 ] && [ "$f_rows" -eq 0 ]; then
        echo "HANDLE-VACUOUS: writer pid=${h_pid} stamped rc=0 having authored ZERO rows;"
        echo "  rc=0 alone carries no health signal (a killed walk stamps it too) -- refusing GREEN."
        exit 10
    fi
    if [ "$rc" = 0 ]; then
        echo "HANDLE-GREEN: writer pid=${h_pid} exited rc=0 having authored all ${f_rows} rows"
        exit 0
    fi
    # 3.664 (queue item 1c): SIGHUP kills the runner mid-suite and stamps rc=1,
    # not rc=0 -- it never enters the VACUOUS branch above, so it fell into this
    # branch indistinguishable from a healthy one-suite-failed run, which also
    # stamps rc=1. Both are graded RED (safe), but the printed line could not
    # tell a human "the runner was killed" apart from "a suite actually failed"
    # without them separately reading rows= themselves. f_rows is the same
    # relation 3.663 used for VACUOUS -- read from the results file, not the
    # handle's self-report -- so a kill cannot fake it here either.
    if [ "$f_rows" -eq 0 ]; then
        echo "HANDLE-EXIT-RED: writer pid=${h_pid} exited rc=${rc} having authored ZERO rows"
        echo "  (killed before any suite finished, not a suite failure)"
    else
        echo "HANDLE-EXIT-RED: writer pid=${h_pid} exited rc=${rc} having authored ${f_rows} rows"
        echo "  (a completed run reporting failure, not a kill)"
    fi
    exit 3
fi

# No EXIT row: in flight, or died without exiting. Either way, not gradeable.
if [ "$lag" -lt 0 ] || [ "$lag" -gt 1 ]; then
    echo "HANDLE-ROWS: handle says rows=${h_rows}, ${RESULTS} holds ${f_rows} (lag ${lag}, window is 0..1)"
    exit 7
fi
[ "$lag" -eq 1 ] && echo "(lagging by 1 -- caught inside the append/stamp window)"
echo "HANDLE-INFLIGHT: no EXIT row; writer pid=${h_pid} has not exited (or died un-exited)"
exit 5
