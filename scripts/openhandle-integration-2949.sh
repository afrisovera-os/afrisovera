#!/usr/bin/env bash
# 2.949 -- INTEGRATION proof for docs/patch-2.908-open-corpse-handle.diff.
#
# 2.928 ruled the DECIDER (scripts/handle-stale-check.sh, 7/7 arms) and 2.941's
# gap line said so in as many words: it proved what the decider ANSWERS on real
# fixtures, NOT that the patched arm_own_handle CALLS it correctly at runtime.
# This script closes exactly that gap by starting the REAL runner twice over an
# identical fixture -- once on the version at git HEAD (pre-patch) and once on
# the live patched tree -- and grading the two transcripts against each other.
#
# THE FIXTURE IS THE REAL CORPSE, and it is the one that SURVIVES on disk:
# .rat-results.2948.tsv.handle.CONTAMINATED, an OPEN handle (no EXIT row) naming
# winpid=30188 startedZ=2026-08-17T02:23:57Z, dead. It is NOT winpid=31936 --
# that is the number the patch's own comment cites (2.907 launch #1, dead since
# 04:46:30Z) and no handle carrying it is left in the tree, so a later agent
# grading this by the comment would go looking for a file that does not exist.
# Both are corpses of the same dead walk 2948 and both have the shape under test.
#
# ONE FIELD of the copy is rewritten and it is disclosed here: results= is
# repointed at the scratch file, because stamp-handle.sh:134 compares that field
# against the file it was handed and a mismatch would fire a DIFFERENT refusal,
# confounding the arm. winpid=30188 -- the thing actually under test -- and the
# absence of an EXIT row are copied byte-for-byte.
#
# Run UNPIPED. rc=0 only if BOTH arms read as required.
set -u
cd "$(dirname "$0")/.." || exit 90

CORPSE=.rat-results.2948.tsv.handle.CONTAMINATED
SUITE="${INTEG_SUITE:-smoke}"
[ -f "$CORPSE" ] || { echo "NO-FIXTURE (90): $CORPSE is gone; this proof cannot be graded"; exit 90; }
grep -qE '^EXIT'$'\t' "$CORPSE" && { echo "BAD-FIXTURE (90): $CORPSE carries an EXIT row; it is not an OPEN corpse"; exit 90; }
grep -q 'winpid=30188' "$CORPSE" || { echo "BAD-FIXTURE (90): $CORPSE does not name winpid=30188"; exit 90; }

# The corpse must still be a corpse AT RUN TIME, not merely when it was written.
if MSYS_NO_PATHCONV=1 tasklist /FI "PID eq 30188" 2>/dev/null | grep -q 30188; then
    echo "FIXTURE-ALIVE (91): winpid 30188 is running again (pid reuse); this proof would grade LIVE, not STALE"
    exit 91
fi

fixture() {  # $1 = scratch results path
    rm -f "$1" "$1".handle "$1".handle.prev "$1".keeper.* "$1".holder* 2>/dev/null
    : > "$1"                                     # 0 rows, the precondition under test
    sed "s#results=[^	]*#results=$1#" "$CORPSE" > "$1".handle
}

run_arm() {  # $1 = label, $2 = runner path, $3 = scratch results, $4 = out file
    fixture "$3"
    RAT_KEEPER=1 bash "$2" --only "$SUITE" --results "$3" > "$4" 2>&1
    echo "  [$1] runner rc=$? ; handle lines:"
    grep -a '\[run-all\] handle:' "$4" | sed 's/^/      /'
}

# The control copy MUST live in scripts/, not /tmp. MEASURED, not predicted: the
# runner self-locates with `cd "$(dirname "$0")/.."` (line 67), so a copy run from
# /tmp puts its cwd at the filesystem ROOT, where `mkdir -p .rat-logs` is denied
# and it exits 2 having never reached arm_own_handle at all. The first run of this
# proof did exactly that. It failed LOUDLY only because arm1 demands the pre-patch
# runner's own positive string; an arm phrased as "control did not print 'moved to'"
# would have graded a runner that never started as a clean reproduction of the defect.
HEADCOPY=scripts/.rat-head-2949.sh
trap 'rm -f "$HEADCOPY"' EXIT
git show HEAD:scripts/run-all-tests.sh > "$HEADCOPY" 2>/dev/null || { echo "NO-HEAD (90): cannot extract the pre-patch runner"; exit 90; }
grep -q 'handle-stale-check.sh' "$HEADCOPY" && { echo "BAD-CONTROL (90): the HEAD copy already contains the patch; the control would be vacuous"; exit 90; }

echo "== 2.949 open-corpse handle INTEGRATION (fixture winpid=30188, NOT 31936) =="
echo "-- CONTROL: pre-patch runner from git HEAD --"
run_arm CONTROL "$HEADCOPY" .rat-2949-control.tsv .integ-2949-control.out
echo "-- LIVE: patched runner in the working tree --"
run_arm LIVE scripts/run-all-tests.sh .rat-2949-live.tsv .integ-2949-live.out

fail=0
# ARM 1: the pre-patch runner must REFUSE to move the corpse (this is the defect).
if grep -qa 'already exists; not re-stamping' .integ-2949-control.out \
   && ! grep -qa 'moved to' .integ-2949-control.out; then
    echo "  PASS  arm1 control-refuses: pre-patch runner left the corpse in place (the defect reproduces)"
else
    echo "  FAIL  arm1 control-refuses: pre-patch runner did not exhibit the defect; the flip below is vacuous"; fail=1
fi
# ARM 2: the patched runner must MOVE it, and must say WHY in the decider's words.
if grep -qa 'moved to .rat-2949-live.tsv.handle.prev' .integ-2949-live.out \
   && grep -qa 'winpid=30188, which is DEAD' .integ-2949-live.out; then
    echo "  PASS  arm2 live-moves: patched runner moved the corpse aside, citing winpid=30188 DEAD"
else
    echo "  FAIL  arm2 live-moves: patched runner did not move the corpse (or did not cite the reason)"; fail=1
fi
# ARM 3: MOVED, never deleted -- the corpse survives at .prev with its winpid intact.
if [ -f .rat-2949-live.tsv.handle.prev ] && grep -q 'winpid=30188' .rat-2949-live.tsv.handle.prev; then
    echo "  PASS  arm3 moved-not-deleted: .prev exists and still names winpid=30188"
else
    echo "  FAIL  arm3 moved-not-deleted: the corpse was destroyed, not preserved"; fail=1
fi
# ARM 4: the point of the whole patch -- a FRESH handle is armed over the freed name.
if [ -f .rat-2949-live.tsv.handle ] && ! grep -q 'winpid=30188' .rat-2949-live.tsv.handle; then
    echo "  PASS  arm4 rearmed: a fresh handle now names a DIFFERENT writer ($(grep -o 'winpid=[0-9]*' .rat-2949-live.tsv.handle | tail -1))"
else
    echo "  FAIL  arm4 rearmed: no fresh handle, or it still names the corpse -- the walk is still disarmed"; fail=1
fi
# ARM 5: anti-blindness. The patch must not move a handle over a file WITH rows.
fixture .rat-2949-rows.tsv
printf 'smoke\tPASS\t1\n' > .rat-2949-rows.tsv
RAT_KEEPER=1 bash scripts/run-all-tests.sh --only "$SUITE" --results .rat-2949-rows.tsv > .integ-2949-rows.out 2>&1
if grep -qa 'HAS-ROWS' .integ-2949-rows.out && [ ! -f .rat-2949-rows.tsv.handle.prev ]; then
    echo "  PASS  arm5 rows-held: same dead winpid, but rows on disk -> handle NOT moved"
else
    echo "  FAIL  arm5 rows-held: the patched runner moved a handle over a file holding rows"; fail=1
fi

[ "$fail" = 0 ] && echo "== INTEGRATION PROVEN (5/5) ==" || echo "== INTEGRATION NOT PROVEN =="
exit "$fail"
