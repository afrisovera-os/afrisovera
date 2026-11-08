#!/usr/bin/env bash
# Phase 3.641 - what the ORPHAN instrument would actually have DONE, measured
# rather than argued.
#
# scripts/inuse-gradecheck-3638.sh names defect B "THE FLATTERING PASS":
#   "arm 3 was [ -n "$CWORD" ] && [ "$CWORD" != "$TWORD" ], which is TRUE when
#    TWORD is EMPTY.  So defect A would have printed 'the branch word FLIPPED'
#    off a failed read - green for the wrong reason, in the flattering
#    direction, exactly the shape this project keeps losing to."
#
# That is a claim about a PREDICATE.  It was never carried through to a claim
# about the SCRIPT.  This session carries it through, on the four real
# artefacts its own 3.641 control run produced, and the answer is a correction
# in the UN-flattering direction:
#
#   the orphan's arm 2 reads the SAME empty TWORD and calls it FAIL, so the
#   orphan would have exited 20 NOT-DIFFERENTIAL.  Defect A+B produce a
#   MISLEADING arm-3 line INSIDE A RED RUN - not a false green.
#
# This does not retire defects A or B.  A "FLIPPED" line printed under a red
# banner is still a line a later session can quote out of its run, and the
# repair is still correct.  It retires the *severity*: the orphan chain's own
# framing said this project nearly lost a false green here, and it did not.
#
# METHOD: the orphan grader is not re-typed.  Its lines 96..159 are extracted
# VERBATIM from scripts/inuse-control-3637.sh (md5-pinned) and executed in a
# subshell against the artefacts of THIS session's control run.
#
# Exit ladder: 10 FIXTURES-UNMET / 20 ORPHANDIFF-RED / 0 ORPHANDIFF-GREEN
set -uo pipefail
cd "$(dirname "$0")/.."

T="${TMPDIR:-/tmp}"
CTL_OUT="$T/inuse-ctl-3639.out"
TRT_OUT="$T/inuse-trt-3639.out"
CTL_LOG="$T/inuse-ctl-listener-3639.log"
TRT_LOG="$T/inuse-trt-listener-3639.log"
ORPHAN=scripts/inuse-control-3637.sh
BLOCK="$T/orphan-grader-3641.sh"
ORPHOUT="$T/orphan-verdict-3641.out"

FAIL=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; FAIL=$((FAIL+1)); }

echo "== fixtures: the four raw artefacts of THIS session's control run =="
for f in "$CTL_OUT" "$TRT_OUT" "$CTL_LOG" "$TRT_LOG" "$ORPHAN"; do
    [ -s "$f" ] || { echo "FIXTURES UNMET: $f missing or empty"; echo "FIXTURES-UNMET"; exit 10; }
    echo "  $(md5sum "$f")"
done
grep -q '^HOST-LISTENER=own'     "$CTL_OUT" || { echo "FIXTURES UNMET: control is not the OWN shape"; echo "FIXTURES-UNMET"; exit 10; }
grep -q 'HOST-LISTENER=foreign'  "$TRT_OUT" || { echo "FIXTURES UNMET: treatment is not the FOREIGN shape"; echo "FIXTURES-UNMET"; exit 10; }

echo "== extract the orphan grader VERBATIM (no re-typing) =="
sed -n '96,159p' "$ORPHAN" > "$BLOCK"
NL=$(wc -l < "$BLOCK")
echo "  $ORPHAN:96..159 -> $NL lines, md5 $(md5sum "$BLOCK" | cut -d' ' -f1)"
grep -q 'CWORD="\$(grep -m1' "$BLOCK" && ok "1 the extracted block really is the orphan's grader (its CWORD read is present)" \
    || bad "1 the extracted block does not contain the orphan's CWORD read - wrong line range"
grep -q 'NOT-DIFFERENTIAL' "$BLOCK" && ok "2 the extracted block carries the orphan's own exit ladder" \
    || bad "2 the extracted block has no exit ladder - the verdict below would be manufactured"

echo "== run the orphan grader on THIS run's real artefacts =="
(
    FAIL=0
    ok()  { echo "  orphan| ok   $*"; }
    bad() { echo "  orphan| FAIL $*"; FAIL=$((FAIL+1)); }
    CRC=0; TRC=0
    CTL_OUT="$CTL_OUT" TRT_OUT="$TRT_OUT" CTL_LOG="$CTL_LOG" TRT_LOG="$TRT_LOG"
    . "$BLOCK"
) > "$ORPHOUT" 2>&1
ORC=$?
sed 's/^/  /' "$ORPHOUT"
echo "  orphan grader exit = $ORC"

echo "== grading the orphan against the repair =="
ONF=$(grep -c 'orphan| FAIL' "$ORPHOUT"); ONF=${ONF:-0}

grep -q "orphan| FAIL 2 TREATMENT word is ''" "$ORPHOUT" \
    && ok "3 defect A reproduces on a LIVE artefact: the orphan's arm 2 read the treatment word EMPTY" \
    || bad "3 the orphan's arm 2 did not report an empty treatment word: $(grep -m1 'arm 2\|FAIL 2\|ok   2' "$ORPHOUT")"

grep -q 'orphan| ok   3 the branch word FLIPPED' "$ORPHOUT" \
    && ok "4 defect B reproduces on a LIVE artefact: arm 3 printed FLIPPED off that SAME failed read" \
    || bad "4 the orphan's arm 3 did not print FLIPPED - defect B did not reproduce"

# THE CORRECTION.  A predicate that flatters is not a script that flatters.
if [ "$ORC" -eq 20 ] && [ "$ONF" -ge 1 ]; then
    ok "5 CORRECTION: the orphan still exits 20 NOT-DIFFERENTIAL ($ONF FAIL arm(s)) - arm 2 is the backstop, so defect B is a MISLEADING LINE IN A RED RUN, not a false green"
elif [ "$ORC" -eq 0 ]; then
    bad "5 the orphan exited 0 GREEN - defect B IS sufficient for a false green after all; 3.638's framing stands and this arm is the finding"
else
    bad "5 the orphan exited $ORC with $ONF FAIL arm(s) - neither 0 nor 20; the extraction is not faithful"
fi

# ...and the same four artefacts must drive the REPAIRED read to the right answer.
read_word() { sed -n 's/^[[:space:]]*|\{0,1\}[[:space:]]*\(HOST-LISTENER=.*\)$/\1/p' "$1" 2>/dev/null | head -1; }
RC_W="$(read_word "$CTL_OUT")"; RT_W="$(read_word "$TRT_OUT")"
case "$RC_W:$RT_W" in
    HOST-LISTENER=own*:HOST-LISTENER=foreign*)
        ok "6 the REPAIRED read on the SAME two files yields the genuine pair: '$RC_W' -> '$RT_W'" ;;
    *)  bad "6 the repaired read yields '$RC_W' -> '$RT_W', not the own->foreign pair" ;;
esac
[ -n "$RT_W" ] && [ -z "$(grep -m1 '^HOST-LISTENER' "$TRT_OUT" 2>/dev/null)" ] \
    && ok "7 DIFFERENTIAL on one file: the orphan expression reads EMPTY where the repaired one reads '$RT_W'" \
    || bad "7 the two reads did not differ on $TRT_OUT - the defect is not exercised by this fixture"

RATMD5="$(md5sum .rat-results.tsv 2>/dev/null | cut -d' ' -f1)"
BASEMD5="$(md5sum .rat-results.pre3600.tsv 2>/dev/null | cut -d' ' -f1)"
[ "$BASEMD5" = 376a22f25b2d583afcd42bd4543e8126 ] \
    && ok "8 pinned green baseline .rat-results.pre3600.tsv still $BASEMD5" \
    || bad "8 green baseline md5 is $BASEMD5, expected 376a22f25b2d583afcd42bd4543e8126"
[ "$RATMD5" = 0cda4771528b6d8903c21667580d475a ] \
    && ok "9 shared .rat-results.tsv still $RATMD5 - this check is read-only in the shared namespace" \
    || bad "9 shared .rat-results.tsv md5 is $RATMD5, expected 0cda4771528b6d8903c21667580d475a"

echo "-- $((9-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "ORPHANDIFF-GREEN"; exit 0; }
echo "ORPHANDIFF-RED"; exit 20
