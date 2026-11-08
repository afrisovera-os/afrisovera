#!/usr/bin/env bash
# Phase 2.886 - the KILL half of the marker-keyed reaper.
#
# 2.881 built the SELECTION half (qemu-select-2881.sh: marker -> PID list, never
# kills). 2.885 built the MINT half (run-all-tests.sh writes a PATH shim that
# execs qemu with -name afrish-walk-$AFRISH_WALK_ID). This is the third piece and
# the only one that takes a life: it kills BY PID, and only PIDs the selector
# printed as MATCH.
#
# It is a SEPARATE SCRIPT on purpose. run-all-tests.sh:400 kill_qemu still runs
# the machine-wide `taskkill //F //IM qemu-system-aarch64.exe`, and that form is
# deleted LAST - after this reaper has been measured against a real control guest.
# Until then the //IM sweep is the safety net; replacing it before measuring would
# swap a known-too-wide reaper for an unmeasured one.
#
# Usage:  qemu-reap-2886.sh <marker>
# Exit:   0  = reaped every MATCH pid (or there were none - a no-op is success)
#         2  = REFUSED (selector refused the marker shape; nothing selected)
#         4  = at least one MATCH pid survived the kill
#         3  = no kill mechanism on this host
set -u

MARKER="${1:-}"
SELECT=scripts/qemu-select-2881.sh
[ -f "$SELECT" ] || { echo "REAP: missing $SELECT"; exit 3; }

OUT="$(bash "$SELECT" "$MARKER" 2>&1)"; SRC=$?
printf '%s\n' "$OUT" | sed 's/^/  select| /'

if [ "$SRC" = 2 ]; then
  echo "REAP REFUSED: the selector would not stand behind this marker. Killing nothing."
  exit 2
fi

MATCH="$(printf '%s\n' "$OUT" | sed -n 's/^MATCH \([0-9][0-9]*\)$/\1/p')"
SKIP="$(printf '%s\n' "$OUT" | sed -n 's/^SKIP \([0-9][0-9]*\)$/\1/p')"
NSKIP="$(printf '%s\n' "$SKIP" | grep -c '[0-9]')"

if [ -z "$MATCH" ]; then
  echo "REAP NO-OP: 0 pids carry '$MARKER'; $NSKIP unmarked qemu left alone."
  exit 0
fi

command -v taskkill >/dev/null 2>&1 || { echo "REAP: no taskkill on this host"; exit 3; }

KILLED=0
for pid in $MATCH; do
  # //PID, never //IM: the whole point is that the image name is not the selector.
  if taskkill //F //PID "$pid" >/dev/null 2>&1; then
    echo "REAP killed pid $pid"; KILLED=$((KILLED + 1))
  else
    echo "REAP taskkill failed on pid $pid (already gone?)"
  fi
done

sleep 1
# 2.868: ONE selector call for the whole survivor check, not one PER PID.
#
# The old form put `bash "$SELECT"` INSIDE `for pid in $MATCH`, so a row that killed
# N guests paid 1+N selector launches. reapcost-probe-2867.sh prices one launch at
# 11.8 s on this host (idle), so the survivor check alone cost N x 11.8 s to re-ask
# a question whose answer is identical for every pid in the loop: the selector takes
# the MARKER, not the pid, so all N calls returned the same text.
#
# The one behavioural difference is deliberate and conservative: every pid is now
# graded against the SAME snapshot, taken right after `sleep 1`, instead of pid k
# being graded against a snapshot taken k selector-calls later. Later pids therefore
# get LESS time to disappear, so this can only over-report survivors, never under-
# report them - it fails toward exit 4, not toward a false clean bill.
STILL="$(bash "$SELECT" "$MARKER" 2>/dev/null)"
LEFT=0
for pid in $MATCH; do
  printf '%s\n' "$STILL" | grep -q "^MATCH ${pid}$" && LEFT=$((LEFT + 1))
done

echo "REAP marker=$MARKER killed=$KILLED survived=$LEFT unmarked-left-alone=$NSKIP"
[ "$LEFT" = 0 ] || exit 4
exit 0
