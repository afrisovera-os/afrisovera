#!/usr/bin/env bash
# Phase 3.639 - THE NO-HOLDER CONTROL BOOT, on an instrument whose three named
# defects are REPAIRED.
#
# WHY A NEW FILE.  scripts/inuse-control-3637.sh (untracked orphan, 01:40Z) is
# the same experiment, but scripts/inuse-gradecheck-3638.sh MEASURED three
# defects in it and this session reproduced that gradecheck independently
# (GRADECHECK-GREEN, 18 ok / 0 FAIL, fresh pids + fresh 299/307/326 ms
# lifetimes).  The orphan is kept unedited as the falsified original:
#
#   A  WRONG STREAM.   It read the treatment word with `grep -m1 '^HOST-LISTENER'`
#      off the 2859 PROBE's stdout, where the word is INDENTED (`  | HOST-...`).
#      That expression matches 0 lines - the read always fails.
#   B  FLATTERING PASS. Its arm 3 was `[ -n "$CWORD" ] && [ "$CWORD" != "$TWORD" ]`,
#      which is TRUE when TWORD is EMPTY.  So defect A would have printed
#      "the branch word FLIPPED" off a failed read - green for the wrong reason,
#      in the flattering direction, exactly the shape this project keeps losing to.
#   C  VACUOUS PRECONDITION. `ps -W | grep -c run-all-tests` is 0 WITH A RUNNER
#      PROVABLY LIVE (3.638 arms 10-12, inside a closed liveness bracket):
#      ps -W carries Windows EXE paths, never script names.  Measured again here
#      before this script trusts its own replacement: plain `ps` shows only
#      /usr/bin/bash too; `ps -f` DOES carry the script's command line.
#
# REPAIRS
#   A  the treatment word is read with the probe's '  | ' prefix stripped.
#   B  arm 3 REFUSES an empty word on either side, and the flip must be the
#      own->foreign pair, not merely "two different strings".
#   C  the walk detector is `ps -f | grep '[r]un-all-tests'`, and it is proven
#      NON-VACUOUS in the same sample by requiring it to find THIS SCRIPT's own
#      command line.  A detector that cannot see a bash script name fails the
#      precondition instead of silently reporting "nothing is running".
#
# WHAT THE EXPERIMENT IS FOR.  2.953 drove tcp-test.sh down its INUSE branch
# with a FOREIGN holder on 127.0.0.1:80 (INUSE-BOOT-OK, 8 arms, rc=0) and wrote
# its own gap down verbatim: "NOT CLAIMED: no no-holder control boot was run in
# this dispatch ... the two shapes were not observed back to back on this host
# today."  Without that control two of those arms are UNGRADED as differential:
#   arm 1  "suite took the INUSE branch"          (HOST-LISTENER=foreign)
#   arm 3  "the losing listener accepted nothing" (0 accepts)
# Both would read the same way if the suite's listener were simply BROKEN.  The
# only thing separating "the branch was taken" from "the listener never works"
# is running the SAME suite back to back with the holder as the ONLY difference.
#
# METHOD (2.765 rule 5): scripts/tcp-test.sh and scripts/inuse-boot-probe-2859.sh
# are both CALLED UNEDITED.  This script arranges nothing but the ABSENCE of a
# holder on the control side.  Two real QEMU boots.  Nothing in the shared
# .rat-* namespace is written; the shared results md5 is graded before and after.
#
# Exit ladder: 10 PRECONDITION-UNMET / 20 NOT-DIFFERENTIAL / 0 INUSE-CONTROL-GREEN
set -uo pipefail
cd "$(dirname "$0")/.."

T="${TMPDIR:-/tmp}"
SLOG="$T/tcp-test-listener.log"            # the log tcp-test.sh hard-codes
CTL_OUT="$T/inuse-ctl-3639.out"            # control suite transcript
CTL_LOG="$T/inuse-ctl-listener-3639.log"   # snapshot of SLOG after the control
TRT_OUT="$T/inuse-trt-3639.out"            # 2859 probe transcript
TRT_LOG="$T/inuse-trt-listener-3639.log"   # snapshot of SLOG after the treatment
PORT=80

FAIL=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; FAIL=$((FAIL+1)); }

port_free() {
    python3 -c "
import socket
s=socket.socket(); s.settimeout(3)
try:
    s.connect(('127.0.0.1',$PORT)); print('OCCUPIED')
except OSError: print('FREE')
finally: s.close()"
}

# --- PRECONDITION ----------------------------------------------------------
echo "== precondition =="

# C-repair, part 1: the detector must be able to see a bash SCRIPT NAME at all.
# It is asked for THIS script, which is provably live (it is running).
PSF="$(ps -f 2>/dev/null)"
SELFN=$(printf '%s\n' "$PSF" | grep -c "[i]nuse-control-3639"); SELFN=${SELFN:-0}
OLDN=$(ps -W 2>/dev/null | grep -c "[r]un-all-tests"); OLDN=${OLDN:-0}
echo "  detector self-test: ps -f sees this script's own cmdline $SELFN time(s)"
if [ "$SELFN" -lt 1 ]; then
    echo "PRECONDITION UNMET: the walk detector cannot see a live bash script name -"
    echo "  it would report 'no walk' for a reason that has nothing to do with walks"
    echo "PRECONDITION-UNMET"; exit 10
fi

WN=$(printf '%s\n' "$PSF" | grep -c "[r]un-all-tests"); WN=${WN:-0}
QN=$(ps -W 2>/dev/null | grep -i qemu | grep -vc grep); QN=${QN:-0}
echo "  qemu processes = $QN ; run-all-tests processes = $WN (ps -f)"
echo "  [the orphan's ps -W expression would have said $OLDN either way]"
if [ "$QN" -ne 0 ] || [ "$WN" -ne 0 ]; then
    echo "PRECONDITION UNMET: a guest or a walk is live - a boot would make a live row lie"
    echo "PRECONDITION-UNMET"; exit 10
fi

PF="$(port_free)"
echo "  127.0.0.1:$PORT = $PF"
if [ "$PF" != FREE ]; then
    echo "PRECONDITION UNMET: something already holds :$PORT - the CONTROL side would not be a control"
    echo "PRECONDITION-UNMET"; exit 10
fi

RATMD5_BEFORE="$(md5sum .rat-results.tsv 2>/dev/null | cut -d' ' -f1)"
echo "  shared .rat-results.tsv md5 before = ${RATMD5_BEFORE:-<absent>}"

# --- CONTROL: the same suite, unedited, with NOTHING holding the port -------
echo "== CONTROL: scripts/tcp-test.sh (unedited), no foreign holder =="
rm -f "$SLOG"
bash scripts/tcp-test.sh >"$CTL_OUT" 2>&1
CRC=$?
cp -f "$SLOG" "$CTL_LOG" 2>/dev/null
echo "  control suite rc=$CRC"
grep -E 'HOST-LISTENER|TCP TEST' "$CTL_OUT" | sed 's/^/  | /'
echo "  control listener log: $(head -1 "$CTL_LOG" 2>/dev/null)"

PF2="$(port_free)"
echo "  :$PORT after control = $PF2"
if [ "$PF2" != FREE ]; then
    echo "PRECONDITION UNMET: control left :$PORT held; treatment would not be clean"
    echo "PRECONDITION-UNMET"; exit 10
fi

# --- TREATMENT: 2.953's probe, unedited, foreign holder on the port ---------
echo "== TREATMENT: scripts/inuse-boot-probe-2859.sh (unedited) =="
rm -f "$SLOG"
bash scripts/inuse-boot-probe-2859.sh >"$TRT_OUT" 2>&1
TRC=$?
cp -f "$SLOG" "$TRT_LOG" 2>/dev/null
echo "  probe rc=$TRC"
grep -E 'HOST-LISTENER|INUSE-|passed,' "$TRT_OUT" | sed 's/^/  | /'
echo "  treatment listener log: $(head -1 "$TRT_LOG" 2>/dev/null)"

# --- GRADE: the two shapes must DIFFER in exactly the predicted places ------
echo "== grading the differential =="

# A-repair: the control prints the word at column 0; the PROBE re-prints it
# behind a '  | ' prefix.  Both are read with the prefix stripped, so neither
# read depends on which stream carried it.
read_word() { sed -n 's/^[[:space:]]*|\{0,1\}[[:space:]]*\(HOST-LISTENER=.*\)$/\1/p' "$1" 2>/dev/null | head -1; }
CWORD="$(read_word "$CTL_OUT")"
TWORD="$(read_word "$TRT_OUT")"
echo "  CWORD='$CWORD'"
echo "  TWORD='$TWORD'"

case "$CWORD" in
    HOST-LISTENER=own*) ok "1 CONTROL took the OWN branch: $CWORD" ;;
    *)                  bad "1 CONTROL word is '$CWORD', expected HOST-LISTENER=own" ;;
esac
case "$TWORD" in
    HOST-LISTENER=foreign*) ok "2 TREATMENT took the FOREIGN branch: $TWORD" ;;
    *)                      bad "2 TREATMENT word is '$TWORD', expected HOST-LISTENER=foreign" ;;
esac
# B-repair: BOTH words must be non-empty, they must differ, AND the difference
# must be the own->foreign pair.  An empty read can no longer print FLIPPED.
if [ -z "$CWORD" ] || [ -z "$TWORD" ]; then
    bad "3 a branch word is EMPTY (control='$CWORD' treatment='$TWORD') - a failed READ is not a flip"
elif [ "$CWORD" = "$TWORD" ]; then
    bad "3 the branch word did NOT flip - both sides say '$CWORD'"
else
    case "$CWORD:$TWORD" in
        HOST-LISTENER=own*:HOST-LISTENER=foreign*)
            ok "3 the branch word FLIPPED own->foreign with the holder - 2859 arm 1 is DIFFERENTIAL, not unconditional" ;;
        *)  bad "3 the words differ but not as own->foreign (control='$CWORD' treatment='$TWORD')" ;;
    esac
fi

if [ ! -s "$CTL_LOG" ]; then
    bad "4 control listener log missing/empty - nothing to grade"
else
    case "$(head -1 "$CTL_LOG")" in
        READY*) ok "4 control listener log says READY: $(head -1 "$CTL_LOG")" ;;
        *)      bad "4 control listener log says '$(head -1 "$CTL_LOG")', expected READY" ;;
    esac
fi
if [ ! -s "$TRT_LOG" ]; then
    bad "5 treatment listener log missing/empty - a MISSING file must not grade as 'accepted nothing'"
else
    case "$(head -1 "$TRT_LOG")" in
        INUSE*) ok "5 treatment listener log says INUSE: $(head -1 "$TRT_LOG")" ;;
        *)      bad "5 treatment listener log says '$(head -1 "$TRT_LOG")', expected INUSE" ;;
    esac
fi

# THE arm 2859 could not grade alone: its losing listener accepted NOTHING -
# is that the holder winning the race, or a listener that never accepts?
CACC=$(grep -c '^ACCEPTED' "$CTL_LOG" 2>/dev/null); CACC=${CACC:-0}
TACC=$(grep -c '^ACCEPTED' "$TRT_LOG" 2>/dev/null); TACC=${TACC:-0}
echo "  ACCEPTED on the SUITE'S OWN log: control=$CACC treatment=$TACC"
if [ -s "$CTL_LOG" ] && [ "$CACC" -ge 1 ]; then
    ok "6 the suite's own listener DOES accept when it owns the socket ($CACC) - so 2859 arm 3's zero is the HOLDER, not a dead listener"
else
    bad "6 control listener accepted $CACC - cannot distinguish 2859's zero from a broken listener"
fi
if [ -s "$TRT_LOG" ] && [ "$TACC" -eq 0 ]; then
    ok "7 treatment's losing listener accepted 0 - and arm 6 proves that zero is meaningful"
else
    bad "7 treatment's losing listener accepted $TACC (log present=$([ -s "$TRT_LOG" ] && echo yes || echo NO)), expected a present log with 0"
fi

# both shapes must still GRADE THE GUEST green - the branch changes plumbing, not the verdict
[ "$CRC" = 0 ] && ok "8 control suite rc=0" || bad "8 control suite rc=$CRC, expected 0"
grep -q '^TCP TEST PASSED' "$CTL_OUT" \
    && ok "9 control verdict: $(grep -m1 '^TCP TEST PASSED' "$CTL_OUT")" \
    || bad "9 control printed no TCP TEST PASSED: $(grep -m1 'TCP TEST' "$CTL_OUT")"
[ "$TRC" = 0 ] && ok "10 2859 probe rc=0" || bad "10 2859 probe rc=$TRC, expected 0"
grep -q '^INUSE-BOOT-OK' "$TRT_OUT" \
    && ok "11 2859 probe verdict INUSE-BOOT-OK (re-run today, not quoted from 2.953)" \
    || bad "11 2859 probe did not print INUSE-BOOT-OK: $(grep -m1 'INUSE-' "$TRT_OUT")"

# the guest must be graded GREEN on BOTH sides by the SAME words - otherwise the
# holder changed the verdict, not just the plumbing
if grep -q '^TCP TEST PASSED' "$CTL_OUT" && grep -q 'TCP TEST PASSED' "$TRT_OUT"; then
    ok "12 the GUEST verdict is identical on both sides - the holder moved the plumbing, not the pass"
else
    bad "12 the guest verdict is not identical on both sides"
fi

# namespace hygiene: this experiment must not touch the shared results file
RATMD5_AFTER="$(md5sum .rat-results.tsv 2>/dev/null | cut -d' ' -f1)"
if [ "$RATMD5_BEFORE" = "$RATMD5_AFTER" ]; then
    ok "13 shared .rat-results.tsv md5 unmoved: ${RATMD5_AFTER:-<absent>}"
else
    bad "13 shared .rat-results.tsv md5 MOVED: $RATMD5_BEFORE -> $RATMD5_AFTER"
fi

# nothing of ours may outlive the run
PF3="$(port_free)"
[ "$PF3" = FREE ] \
    && ok "14 127.0.0.1:$PORT is FREE again after both boots - no listener of ours leaked" \
    || bad "14 127.0.0.1:$PORT is $PF3 after the run - something of ours is still holding it"

echo "-- $((14-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "INUSE-CONTROL-GREEN"; exit 0; }
echo "NOT-DIFFERENTIAL"; exit 20
