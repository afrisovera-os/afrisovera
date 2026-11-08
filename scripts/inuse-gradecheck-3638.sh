#!/usr/bin/env bash
# Phase 3.638 - CLOSE THE TWO ARMS 3.637 LEFT UNGRADED, AND MAKE ARM 10's ZERO MEAN SOMETHING.
#
# 3.637 (.inuse-3637-gradecheck.out, 04:36Z) ended GRADECHECK-RED 11/2 with:
#     FAIL 11 could not read a WINPID for msys pid 209593 - replacement instrument ungraded
#     FAIL 12 (same)
# It did NOT say why. Measured here over three real launches, not argued: the
# process it tried to name, `bash scripts/run-all-tests.sh --list`, LIVES ~0.36 s
# -- SHORTER THAN THE SAMPLE THAT MUST NAME IT. Reproduced 3/3 with liveness
# asked on BOTH sides of the read:
#     run 1 before=live afterPsW=dead afterPs=dead winpid=EMPTY
#     run 2 before=live afterPsW=live afterPs=dead winpid=EMPTY
#     run 3 before=live afterPsW=live afterPs=dead winpid=EMPTY
# `ps` was right; the process was gone. And `ps | awk '$1==p{print $4}'` is a
# correct WINPID read -- ps's header here is `PID PPID PGID WINPID ...`, column 4,
# verified against a holder that outlives the sample (arm 14 below).
#
# THAT SAME MEASUREMENT CONDEMNS 3.637's ARM 9/10 PAIR. Arm 9 asked liveness ONCE,
# before the sample, and arm 10 then graded `ps -W | grep -c run-all-tests` = 0 as
# "structurally vacuous WHILE IT IS RUNNING". Run 1 above shows the process can be
# DEAD by then -- so on run 1 that zero would have been counted off a corpse and
# printed the same green line. The finding (defect C) survives; the INSTRUMENT did
# not earn it. Repaired two ways here:
#   * LIVENESS BRACKET: live asserted BEFORE and AFTER the ps -W sample, retried
#     until a bracket closes; only a bracketed zero is graded.
#   * THE ZERO IS MADE MEANINGFUL: `ps -W` is shown to be listing processes at
#     that instant, and to carry greppable EXE text (bash), so 0 for the SCRIPT
#     name is a statement about ps -W's content, not about ps -W being empty.
#
# ARMS 11/12 no longer borrow the runner as their holder. What they grade is
# writer-guard.sh REFUSING a handle whose winpid is LIVE -- that needs a live
# WINPID, not a live runner. A deterministic long-lived holder is used and said
# so plainly, its liveness confirmed by tasklist INDEPENDENTLY of ps before the
# guard is asked. And the refusal is graded BY REASON: writer-guard returns 88
# for BOTH rc=0 (live) and rc=37 (UNKNOWN), so 3.637's bare `-eq 88` would have
# passed on a probe that ERRORED. The LIVE line is now required.
#
# Arms 1-8 and the md5 arm are 3.637's, unchanged, on the same real fixtures.
# Every arm is offline: no guest is booted, no port is bound, nothing in the
# shared .rat-* namespace is written. Exit 0 GRADECHECK-GREEN / 20 GRADECHECK-RED.
set -uo pipefail
cd "$(dirname "$0")/.."

TRT_STDOUT=".inuse-3637.out"            # the 2859 probe's STDOUT (03:28Z run)
TRT_SUITE=".inuse-3637-trt-suite.out"   # the RAW suite transcript of that run
TRT_SLOG=".inuse-3637-trt-slog.log"     # the suite's own listener log, INUSE
FAKE=".inuse-3638-fake.tsv"

FAIL=0
N=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; FAIL=$((FAIL+1)); }

HOLDPID=""
cleanup() {
    [ -n "$HOLDPID" ] && kill "$HOLDPID" 2>/dev/null
    rm -f "$FAKE" "${FAKE}.handle"
}
trap cleanup EXIT

for f in "$TRT_STDOUT" "$TRT_SUITE" "$TRT_SLOG"; do
    [ -s "$f" ] || { echo "PRECONDITION UNMET: fixture $f missing/empty"; exit 10; }
done
echo "== fixtures (real, from the 03:28Z treatment run) =="
md5sum "$TRT_STDOUT" "$TRT_SUITE" "$TRT_SLOG" | sed 's/^/  /'

# taken BEFORE anything runs, so arm 18 compares two real readings (3.637 had a
# BEFORE; the first draft of this script lost it and would have compared a value
# with itself -- a green arm asserting nothing).
RATMD5_BEFORE="$(md5sum .rat-results.tsv 2>/dev/null | cut -d' ' -f1)"

# ---- DEFECT A: the orphan's expression reads NOTHING off the probe's stdout ---
echo "== A: wrong stream =="
OLDN=$(grep -c '^HOST-LISTENER' "$TRT_STDOUT" || true); OLDN=${OLDN:-0}
ANYN=$(grep -c 'HOST-LISTENER'  "$TRT_STDOUT" || true); ANYN=${ANYN:-0}
[ "$OLDN" -eq 0 ] \
    && ok "1 orphan expression '^HOST-LISTENER' matches 0 lines of the probe's stdout" \
    || bad "1 orphan expression matched $OLDN lines - defect A does not reproduce"
[ "$ANYN" -ge 1 ] \
    && ok "2 ...yet the word IS there, indented: $(grep -m1 'HOST-LISTENER' "$TRT_STDOUT" | sed 's/^ *//')" \
    || bad "2 the fixture carries no HOST-LISTENER text at all - wrong fixture"

NEWT="$(grep -m1 '^HOST-LISTENER' "$TRT_SUITE" || true)"
case "$NEWT" in
    HOST-LISTENER=foreign*) ok "3 repaired source reads the treatment word: $NEWT" ;;
    *)                      bad "3 repaired source read '$NEWT', expected HOST-LISTENER=foreign*" ;;
esac

# ---- DEFECT B: the orphan's arm 3 passes on that empty read -------------------
echo "== B: the flattering pass =="
CW='HOST-LISTENER=own (this suite bound 127.0.0.1:80)'
TW_EMPTY=''
if [ -n "$CW" ] && [ "$CW" != "$TW_EMPTY" ]; then
    ok "4 orphan arm-3 predicate is TRUE with an EMPTY treatment word - it would print 'FLIPPED' off a failed read"
else
    bad "4 orphan arm-3 predicate did not reproduce its flattering pass"
fi
if [ -n "$CW" ] && [ -n "$TW_EMPTY" ] && [ "$CW" != "$TW_EMPTY" ]; then
    bad "5 repaired arm-3 predicate STILL passes on an empty treatment word"
else
    ok "5 repaired arm-3 predicate REFUSES an empty treatment word"
fi
if [ -n "$CW" ] && [ -n "$NEWT" ] && [ "$CW" != "$NEWT" ]; then
    ok "6 repaired arm-3 predicate is TRUE on the genuine pair (own vs foreign) - both its arms are reached"
else
    bad "6 repaired arm-3 predicate is FALSE on the genuine pair - it would refuse a real differential"
fi

case "$(head -1 "$TRT_SLOG")" in
    INUSE*) ok "7 treatment listener log fixture says INUSE: $(head -1 "$TRT_SLOG")" ;;
    *)      bad "7 treatment listener log fixture says '$(head -1 "$TRT_SLOG")'" ;;
esac
TACC=$(grep -c '^ACCEPTED' "$TRT_SLOG" || true); TACC=${TACC:-0}
[ "$TACC" -eq 0 ] && ok "8 treatment's losing listener accepted 0 (the zero the control must make meaningful)" \
                  || bad "8 treatment's losing listener accepted $TACC"

# ---- 3.638 ARM: NAME WHY 11/12 WENT UNGRADED, BY MEASUREMENT -----------------
# The runner is launched for real and its lifetime timed to the millisecond, three
# times. Nothing is inferred from the 04:36Z transcript.
echo "== C0: the runner's own lifetime, measured (why 3.637 could not name it) =="
LIVES=""
MAXMS=0
for _ in 1 2 3; do
    T0=$(date +%s%3N)
    bash scripts/run-all-tests.sh --list >/dev/null 2>&1 &
    LP=$!
    while kill -0 "$LP" 2>/dev/null; do sleep 0.02; done
    wait "$LP" 2>/dev/null
    MS=$(( $(date +%s%3N) - T0 ))
    LIVES="${LIVES}${MS}ms "
    [ "$MS" -gt "$MAXMS" ] && MAXMS="$MS"
done
if [ "$MAXMS" -gt 0 ] && [ "$MAXMS" -lt 2000 ]; then
    ok "9 run-all-tests.sh --list lifetimes: ${LIVES}(max ${MAXMS} ms) - too short-lived to be named by a later sample"
else
    bad "9 --list lifetimes ${LIVES}(max ${MAXMS} ms) - the 3.637 cause does not reproduce; re-derive before trusting arm 10"
fi

# ---- DEFECT C, inside a CLOSED liveness bracket -------------------------------
echo "== C: the vacuous precondition, measured INSIDE a closed liveness bracket =="
ATT=0; BRACKET=0; WN=-1; PSW_LINES=0; PSW_BASH=0
while [ "$ATT" -lt 15 ]; do
    ATT=$((ATT+1))
    bash scripts/run-all-tests.sh --list >/dev/null 2>&1 &
    LP=$!
    kill -0 "$LP" 2>/dev/null || { wait "$LP" 2>/dev/null; continue; }   # pre-assert
    PSW="$(ps -W 2>/dev/null || true)"
    if kill -0 "$LP" 2>/dev/null; then                                   # post-assert
        BRACKET=1
        WN=$(printf '%s\n' "$PSW" | grep -c 'run-all-tests' || true); WN=${WN:-0}
        PSW_LINES=$(printf '%s\n' "$PSW" | grep -c . || true); PSW_LINES=${PSW_LINES:-0}
        PSW_BASH=$(printf '%s\n' "$PSW" | grep -ci 'bash' || true); PSW_BASH=${PSW_BASH:-0}
    fi
    kill "$LP" 2>/dev/null; wait "$LP" 2>/dev/null
    [ "$BRACKET" -eq 1 ] && break
done
if [ "$BRACKET" -eq 1 ]; then
    ok "10 liveness BRACKET closed on attempt ${ATT}: the runner was live before AND after the ps -W sample"
    # the zero is only a statement about ps -W's CONTENT if ps -W had content
    if [ "$PSW_LINES" -ge 3 ] && [ "$PSW_BASH" -ge 1 ]; then
        ok "11 ps -W was listing at that instant: ${PSW_LINES} lines, ${PSW_BASH} of them matching 'bash' - so a 0 is about its CONTENT"
    else
        bad "11 ps -W gave ${PSW_LINES} lines / ${PSW_BASH} bash - an empty ps -W would make arm 12's zero vacuous"
    fi
    [ "$WN" -eq 0 ] \
        && ok "12 orphan precondition 'ps -W | grep -c run-all-tests' = 0 WITH THE RUNNER PROVABLY LIVE - structurally vacuous (ps -W carries EXE paths, never script names)" \
        || bad "12 orphan precondition counted ${WN} inside the bracket - defect C does not reproduce"
else
    bad "10 no liveness bracket closed in ${ATT} attempts - defect C stays UNGRADED (3.637's outcome, honestly)"
    bad "11 (ungraded)"
    bad "12 (ungraded)"
fi

# ---- the replacement instrument, on a holder that OUTLIVES the sample ---------
# What arms 13-15 grade is writer-guard.sh refusing a handle whose winpid is LIVE.
# That needs a live WINPID, not a live runner -- said plainly rather than borrowed.
echo "== D: writer-guard.sh vs a live winpid (holder outlives the sample) =="
sleep 45 &
HOLDPID=$!
HWIN=""
for _ in $(seq 1 30); do
    HWIN=$(ps 2>/dev/null | awk -v p="$HOLDPID" '$1==p{print $4}' | head -1)
    [ -n "${HWIN:-}" ] && break
    sleep 0.1
done
if [ -n "${HWIN:-}" ]; then
    ok "13 ps named a WINPID for the holder (msys ${HOLDPID} -> winpid ${HWIN}) - the column-4 read 3.637 used is CORRECT; its subject was the problem"
    # liveness confirmed INDEPENDENTLY of ps, by the same instrument the guard trusts
    TL="$(MSYS_NO_PATHCONV=1 tasklist /FI "PID eq ${HWIN}" 2>/dev/null || true)"
    if printf '%s\n' "$TL" | grep -q "${HWIN}"; then
        ok "14 tasklist lists winpid=${HWIN} INDEPENDENTLY of ps - the guard's premise is measured, not assumed"
    else
        bad "14 tasklist does not list winpid=${HWIN}; the guard's 88 below could not be attributed to liveness"
    fi
    : > "$FAKE"
    printf 'WRITER\tresults=%s\tpid=%s\twinpid=%s\tstartedZ=offline\n' "$FAKE" "$HOLDPID" "$HWIN" > "${FAKE}.handle"
    GOUT="$(bash scripts/writer-guard.sh "$FAKE" 2>&1)"; GRC=$?
    # 88 alone is NOT enough: writer-guard returns 88 for rc=0 (LIVE) and rc=37
    # (UNKNOWN) alike. 3.637's bare `-eq 88` would have passed on an ERRORED probe.
    if [ "$GRC" -eq 88 ] && printf '%s\n' "$GOUT" | grep -q 'names a LIVE writer'; then
        ok "15 writer-guard.sh REFUSES (88) BY REASON 'names a LIVE writer' on a handle naming winpid=${HWIN}"
    elif [ "$GRC" -eq 88 ]; then
        bad "15 writer-guard.sh returned 88 but NOT for liveness: $(printf '%s\n' "$GOUT" | head -1)"
    else
        bad "15 writer-guard.sh returned $GRC on a live open handle, expected 88"
    fi
    printf 'EXIT\trows=0\tdetail=offline\n' >> "${FAKE}.handle"
    GOUT2="$(bash scripts/writer-guard.sh "$FAKE" 2>&1)"; GRC2=$?
    if [ "$GRC2" -eq 0 ] && printf '%s\n' "$GOUT2" | grep -q 'EXIT row'; then
        ok "16 ...and PROCEEDS (0) once that handle carries an EXIT row, with the holder STILL live - both arms reached, and the flip is the EXIT row, not the corpse"
    elif [ "$GRC2" -eq 0 ]; then
        bad "16 writer-guard.sh proceeded but not on the EXIT row: $(printf '%s\n' "$GOUT2" | head -1)"
    else
        bad "16 writer-guard.sh returned $GRC2 on a closed handle, expected 0"
    fi
    # the holder must still be alive NOW, or arm 16's PROCEED is a corpse's doing
    kill -0 "$HOLDPID" 2>/dev/null \
        && ok "17 holder still live after both guard calls - arm 16's PROCEED is attributable to the EXIT row alone" \
        || bad "17 holder died mid-check; arm 16's PROCEED cannot be attributed to the EXIT row"
    kill "$HOLDPID" 2>/dev/null; HOLDPID=""
else
    bad "13 no WINPID for the holder after 3 s - replacement instrument UNGRADED"
    bad "14 (ungraded)"; bad "15 (ungraded)"; bad "16 (ungraded)"; bad "17 (ungraded)"
fi

# ---- nothing shared was touched ----------------------------------------------
echo "== E: shared namespace =="
RATMD5_AFTER="$(md5sum .rat-results.tsv 2>/dev/null | cut -d' ' -f1)"
if [ -z "${RATMD5_BEFORE:-}" ]; then
    bad "18 no BEFORE md5 was read for .rat-results.tsv - the unmoved claim cannot be made"
elif [ "$RATMD5_BEFORE" = "$RATMD5_AFTER" ]; then
    ok "18 shared .rat-results.tsv md5 unmoved by this check: ${RATMD5_AFTER}"
else
    bad "18 shared .rat-results.tsv CHANGED: ${RATMD5_BEFORE} -> ${RATMD5_AFTER}"
fi

N=18
echo "-- $((N-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "GRADECHECK-GREEN"; exit 0; }
echo "GRADECHECK-RED"; exit 20
