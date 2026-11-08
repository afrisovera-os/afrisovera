#!/usr/bin/env bash
# handle-stale-check.sh <results-file>
#
# THE ONE QUESTION: may arm_own_handle move ${results}.handle aside?
#
# WHY THIS EXISTS (2.908, from 2.907's measured finding). arm_own_handle's
# move-aside was gated on "EXIT row present AND 0 rows". That covers a walk
# that CLOSED OUT. The case that actually happens is the other one: a walk that
# DIES without closing out leaves an OPEN handle naming a DEAD winpid, and that
# corpse handle then disarms every later walk on that results file -- forever.
# 2.907 launch #1 measured exactly that: handle "already exists; not re-stamping",
# keeper WRITER-GONE (65), holders ARM-REFUSED (33) on winpid=31936, dead since
# 04:46:30Z. All three halves of a fresh walk dead, on a file holding 0 rows.
#
# THE WIDENED CONDITION:   zero rows  AND  (EXIT row present  OR  winpid dead)
#
# The zero-rows half is NOT relaxed. A handle over a file that HOLDS ROWS names
# a walk whose data is on disk; moving its handle aside is never this script's
# call. Only the "that walk has ended" half is widened.
#
# EXIT CODES (the caller only ever acts on 0):
#    0  STALE      -- safe to move aside; reason printed
#   10  LIVE       -- the handle names a winpid that IS alive
#   11  HAS-ROWS   -- the results file holds rows; not this script's call
#   12  NO-HANDLE  -- nothing to decide
#   13  UNKNOWN    -- the liveness probe ERRORED, or the handle carries no
#                     winpid=. UNKNOWN IS NOT DEAD (arm-holders.sh:37 rule).
#                     Refusing to move aside is the conservative direction.
#
# LIVENESS. `kill -0 <winpid>` calls a live walk DEAD (falsified at 2.765), so
# tasklist is the only thing that knows a winpid. The switch is /FI with
# MSYS_NO_PATHCONV set for THIS command only, and stderr is CAPTURED, not
# dropped -- 2.833's repair, whose old form read a tasklist ERROR as "not alive"
# and called a LIVE walk a corpse, the family's flattering direction.
#
# TEST SEAM, declared not hidden: $STALECHECK_TASKLIST names the liveness
# binary (default `tasklist`). scripts/stalecheck-selftest-2908.sh points it at
# a missing command to reach the UNKNOWN arm, which no fixture can otherwise
# produce. Nothing else reads it.
set -u

RESULTS="${1:-}"
[ -n "$RESULTS" ] || { echo "usage: handle-stale-check.sh <results-file>" >&2; exit 2; }
HANDLE="${RESULTS}.handle"

[ -f "$HANDLE" ] || { echo "NO-HANDLE (12): ${HANDLE} does not exist; nothing to decide"; exit 12; }

# rows first: it is the cheap half and it is the half that must never be relaxed
ROWS=0
[ -f "$RESULTS" ] && ROWS="$(wc -l < "$RESULTS" | tr -d ' ')"
if [ "$ROWS" != 0 ]; then
    echo "HAS-ROWS (11): ${RESULTS} holds ${ROWS} row(s); a handle over live data is not moved aside"
    exit 11
fi

if grep -qE '^EXIT'$'\t' "$HANDLE" 2>/dev/null; then
    echo "STALE (0): ${HANDLE} carries an EXIT row and ${RESULTS} holds 0 rows; that walk CLOSED OUT"
    exit 0
fi

# No EXIT row. The 2.907 case: is the writer it names still alive?
WINPID="$(grep -o 'winpid=[0-9][0-9]*' "$HANDLE" 2>/dev/null | tail -1 | cut -d= -f2)"
[ -n "$WINPID" ] || { echo "UNKNOWN (13): ${HANDLE} has no EXIT row and carries no winpid=; cannot rule"; exit 13; }

LIVE_ERR="$(mktemp)"
LIVE_OUT="$(MSYS_NO_PATHCONV=1 "${STALECHECK_TASKLIST:-tasklist}" /FI "PID eq ${WINPID}" 2>"$LIVE_ERR")"
if [ -s "$LIVE_ERR" ]; then
    echo "UNKNOWN (13): liveness probe for winpid=${WINPID} errored: $(head -1 "$LIVE_ERR"); UNKNOWN is not DEAD"
    rm -f "$LIVE_ERR"
    exit 13
fi
rm -f "$LIVE_ERR"

case "$LIVE_OUT" in
    *"${WINPID}"*)
        echo "LIVE (10): ${HANDLE} names winpid=${WINPID}, which IS alive; that walk is still writing"
        exit 10
        ;;
esac

echo "STALE (0): ${HANDLE} has NO EXIT row but names winpid=${WINPID}, which is DEAD, and ${RESULTS} holds 0 rows; that walk died without closing out"
exit 0
