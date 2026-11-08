#!/usr/bin/env bash
# reapcost-probe-2867.sh - WHERE does kill_qemu's 51.7 s go?
#
# 2.864 measured kill_qemu at 51.7 s/row and one powershell launch at 15.1 s. The
# cache branch (SESSION_PLAN NEXT item 2) assumes the cost is "a CIM query per row".
# This probe does NOT assume it - it counts the SELECTOR INVOCATIONS that kill_qemu
# actually performs and prices each one, so the branch is aimed at a measured number.
#
# Reading scripts/qemu-reap-2886.sh, the selector is called:
#   - once up front (line ~27), ALWAYS; then
#   - once PER MATCHED PID in the survivor check (line ~59), inside a for loop.
# So the NO-OP row costs 1 selector; a row that kills N guests costs 1 + N selectors
# plus a hard `sleep 1`. If one selector is ~15 s, a 2-guest row is ~46 s + 1 s - which
# lands on 2.864's 51.7 s WITHOUT any per-row CIM query being intrinsic to the design.
#
# SAFETY: this probe only ever passes a marker that matches nothing (a random-free
# constant that no walk mints), so the selector returns SKIP-only and the reaper takes
# its NO-OP exit BEFORE reaching taskkill. It cannot touch the live walk's guests.
# It is READ-ONLY on the tree: no file in scripts/ is edited by running it.
#
# Exit: 0 = measured, 3 = missing dependency.
set -u

SELECT=scripts/qemu-select-2881.sh
REAP=scripts/qemu-reap-2886.sh
for f in "$SELECT" "$REAP"; do
    [ -f "$f" ] || { echo "PROBE: missing $f"; exit 3; }
done

# A marker no walk mints, but OF THE SHAPE the selector stands behind.
#
# 2.867 hole, kept because it failed FLATTERING: the first version of this probe used
# "afrishwalkNOSUCHMARKER2867" (no hyphens). qemu-select-2881.sh:22 requires
# ^afrish-walk-[A-Za-z0-9._-]+$ and exits 2 BEFORE launching powershell, so all three
# ARM-1 trials measured the REFUSAL path at 385 ms and the probe reported kill_qemu as
# a ~3 s/row cost - i.e. it would have argued the cache branch away. The real call is
# 26.6 s. Law §9: a probe whose cheap arm is the REFUSAL arm measures the guard, not
# the work; assert the expensive path was actually entered (ARM 1 checks rc != 2).
DEAD_MARKER="afrish-walk-000000000000000001"

ms_now() { date +%s%3N; }

echo "== ARM 1: one selector invocation, 3 trials =="
tot=0
for t in 1 2 3; do
    a=$(ms_now)
    out="$(bash "$SELECT" "$DEAD_MARKER" 2>&1)"; src=$?
    b=$(ms_now)
    d=$((b - a)); tot=$((tot + d))
    nmatch=$(printf '%s\n' "$out" | grep -c '^MATCH ')
    nskip=$(printf '%s\n' "$out" | grep -c '^SKIP ')
    echo "  trial $t: ${d} ms  rc=$src  MATCH=$nmatch SKIP=$nskip"
    [ "$src" = 2 ] && { echo "PROBE VOID: selector REFUSED the marker (rc=2) - the"; \
        echo "  expensive path was never entered. Fix the marker shape, do not read"; \
        echo "  these numbers."; exit 3; }
done
SEL_MS=$((tot / 3))
echo "SELECT_MS_MEAN $SEL_MS"

echo "== ARM 2: kill_qemu's whole reaper on the NO-OP path, 2 trials =="
tot=0
for t in 1 2; do
    a=$(ms_now)
    out="$(bash "$REAP" "$DEAD_MARKER" 2>&1)"; rc=$?
    b=$(ms_now)
    d=$((b - a)); tot=$((tot + d))
    verdict="$(printf '%s\n' "$out" | grep -m1 '^REAP')"
    echo "  trial $t: ${d} ms  rc=$rc  ${verdict:-<no REAP line>}"
done
REAP_MS=$((tot / 2))
echo "REAP_NOOP_MS_MEAN $REAP_MS"

echo "== ARM 3: how many live qemu does the selector have to walk past? =="
NSKIP=$(bash "$SELECT" "$DEAD_MARKER" 2>/dev/null | grep -c '^SKIP ')
echo "LIVE_QEMU_SEEN $NSKIP"

echo "== ARITHMETIC =="
# The reaper's own overhead beyond the single selector call it makes on this path.
OVER=$((REAP_MS - SEL_MS))
echo "reaper overhead beyond its 1 selector call: ${OVER} ms"
echo "predicted cost of a row that kills N guests: (1+N)*${SEL_MS} ms + 1000 ms sleep + ${OVER} ms"
for n in 1 2 3; do
    p=$(( (1 + n) * SEL_MS + 1000 + OVER ))
    echo "  N=${n} -> $((p / 1000)).$(( (p % 1000) / 100 )) s"
done
echo "2.864 measured kill_qemu at 51.7 s/row; compare the N rows above."
echo "PROBE DONE"
exit 0
