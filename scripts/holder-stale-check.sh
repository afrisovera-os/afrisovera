#!/usr/bin/env bash
# holder-stale-check.sh <holder-file>
#
# THE ONE QUESTION: may arm-holders.sh move this holder file aside and re-arm?
#
# WHY THIS EXISTS (2.909, from 2.907's measured finding g1 -- the SAME defect
# one level down from 2.908). arm-holders.sh:143 refuses on `[ -e "$f" ]`:
# any holder file that exists at all is "evidence" and blocks arming. That is
# right for a holder still attached to a LIVE walk and wrong for every corpse.
# 2.907 measured it: both holders refused 34 on files left by the 03:39 walk
# that died at 04:46, so a brand-new walk could arm NEITHER exit-code witness
# until a human moved them by hand. A blanket -e gate turns one dead walk into
# a permanent disarm, exactly like 2.908's corpse handle.
#
# THE WIDENED CONDITION:   EXIT row present  OR  the pid it names is DEAD
#
# The conservative half is NOT relaxed: a holder with no EXIT row whose walk is
# still alive is LIVE, and a live walk's only two exit-code witnesses are never
# clobbered. The caller MOVES ASIDE, never deletes -- the corpse is the evidence
# that says why the previous walk ended.
#
# EXIT CODES (the caller only ever acts on 0):
#    0  STALE    -- safe to move aside; reason printed
#   10  LIVE     -- no EXIT row and the pid it names IS alive; still witnessing
#   12  NO-FILE  -- nothing to decide; arming may proceed as normal
#   13  UNKNOWN  -- the liveness probe ERRORED, or the file carries no pid=.
#                   UNKNOWN IS NOT DEAD (arm-holders.sh:37 rule); refusing is
#                   the conservative direction.
#
# LIVENESS. `kill -0 <winpid>` calls a live walk DEAD (falsified at 2.765), so
# tasklist is the only thing that knows a winpid. /FI with MSYS_NO_PATHCONV set
# for THIS command only, stderr CAPTURED not dropped (2.833's repair, whose old
# form read a tasklist ERROR as "not alive" and called a LIVE walk a corpse).
#
# WHICH pid. A holder row carries two: `pid=` is the WALK it witnesses and
# `holder=` is the witness's own process. The walk is what decides staleness --
# a holder whose own process died while its walk still runs has stopped
# witnessing, but the walk it names is still writing rows, and stealing that
# slot is not this script's call. So: `pid=`, not `holder=`.
#
# TEST SEAM, declared not hidden: $HOLDERCHECK_TASKLIST names the liveness
# binary (default `tasklist`). scripts/holdercheck-selftest-2909.sh points it at
# a missing command to reach the UNKNOWN arm, which no fixture can produce.
set -u

HF="${1:-}"
[ -n "$HF" ] || { echo "usage: holder-stale-check.sh <holder-file>" >&2; exit 2; }

[ -f "$HF" ] || { echo "NO-FILE (12): ${HF} does not exist; nothing to decide"; exit 12; }

if grep -qE '^EXIT'$'\t' "$HF" 2>/dev/null; then
    echo "STALE (0): ${HF} carries an EXIT row; that witness CLOSED OUT"
    exit 0
fi

# No EXIT row. The 2.907 case: is the walk it witnesses still alive?
# `holder=NNN` does not contain the substring "pid=", so -o cannot pick the
# witness's own pid by accident; only `pid=` (and a `winpid=`, were one ever
# written into this shape) can match.
WINPID="$(grep -o 'pid=[0-9][0-9]*' "$HF" 2>/dev/null | tail -1 | cut -d= -f2)"
[ -n "$WINPID" ] || { echo "UNKNOWN (13): ${HF} has no EXIT row and carries no pid=; cannot rule"; exit 13; }

LIVE_ERR="$(mktemp)"
LIVE_OUT="$(MSYS_NO_PATHCONV=1 "${HOLDERCHECK_TASKLIST:-tasklist}" /FI "PID eq ${WINPID}" 2>"$LIVE_ERR")"
if [ -s "$LIVE_ERR" ]; then
    echo "UNKNOWN (13): liveness probe for pid=${WINPID} errored: $(head -1 "$LIVE_ERR"); UNKNOWN is not DEAD"
    rm -f "$LIVE_ERR"
    exit 13
fi
rm -f "$LIVE_ERR"

case "$LIVE_OUT" in
    *"${WINPID}"*)
        echo "LIVE (10): ${HF} names pid=${WINPID}, which IS alive; that witness is still on duty"
        exit 10
        ;;
esac

echo "STALE (0): ${HF} has NO EXIT row but names pid=${WINPID}, which is DEAD; that walk died without its witness closing out"
exit 0
