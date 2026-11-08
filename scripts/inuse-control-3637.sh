#!/usr/bin/env bash
# Phase 3.637 - the NO-HOLDER CONTROL boot that 2.953 explicitly did not run.
#
# 2.953 drove scripts/tcp-test.sh end to end down its INUSE branch with a
# FOREIGN holder on 127.0.0.1:80 and got INUSE-BOOT-OK / 8 arms / rc=0.  It
# wrote its own gap down, verbatim:
#
#   "NOT CLAIMED: no no-holder control boot was run in this dispatch. ...
#    the two shapes were not observed back to back on this host today."
#
# Without that control, two of 2859's arms are UNGRADED as differential:
#   arm 1  "suite took the INUSE branch"      (HOST-LISTENER=foreign)
#   arm 3  "the suite's losing listener accepted nothing"
# Both would also read the way 2859 wants them to if the suite's listener
# simply never worked.  The only thing that separates "the branch was taken"
# from "the listener is broken" is running the SAME suite with the ONLY
# difference being the presence of the foreign holder, back to back.
#
# METHOD (2.765 rule 5): scripts/tcp-test.sh and scripts/inuse-boot-probe-2859.sh
# are both CALLED UNEDITED.  This script arranges nothing except the ABSENCE of
# a holder on the control side.  Two real QEMU boots.
#
# Exit ladder: 10 PRECONDITION-UNMET / 20 NOT-DIFFERENTIAL / 0 INUSE-CONTROL-GREEN
set -uo pipefail
cd "$(dirname "$0")/.."

T="${TMPDIR:-/tmp}"
SLOG="$T/tcp-test-listener.log"          # the log tcp-test.sh hard-codes
CTL_OUT="$T/inuse-ctl-3637.out"          # control suite transcript
CTL_LOG="$T/inuse-ctl-listener-3637.log" # snapshot of SLOG after the control
TRT_OUT="$T/inuse-trt-3637.out"          # 2859 probe transcript
TRT_LOG="$T/inuse-trt-listener-3637.log" # snapshot of SLOG after the treatment
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
QN=$(ps -W 2>/dev/null | grep -i qemu | grep -vc grep); QN=${QN:-0}
WN=$(ps -W 2>/dev/null | grep -c run-all-tests); WN=${WN:-0}
echo "  qemu processes = $QN ; run-all-tests processes = $WN"
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

# --- CONTROL: the same suite, unedited, with NOTHING holding the port -------
echo "== CONTROL: scripts/tcp-test.sh (unedited), no foreign holder =="
rm -f "$SLOG"
bash scripts/tcp-test.sh >"$CTL_OUT" 2>&1
CRC=$?
cp -f "$SLOG" "$CTL_LOG" 2>/dev/null
echo "  control suite rc=$CRC"
grep -E 'HOST-LISTENER|TCP TEST' "$CTL_OUT" | sed 's/^/  | /'
echo "  control listener log: $(head -1 "$CTL_LOG" 2>/dev/null)"

# the control's listener must be gone again before the treatment arranges its own
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

CWORD="$(grep -m1 '^HOST-LISTENER' "$CTL_OUT" 2>/dev/null)"
TWORD="$(grep -m1 '^HOST-LISTENER' "$TRT_OUT" 2>/dev/null | sed 's/^ *| *//')"

case "$CWORD" in
    HOST-LISTENER=own*) ok "1 CONTROL took the OWN branch: $CWORD" ;;
    *)                  bad "1 CONTROL word is '$CWORD', expected HOST-LISTENER=own" ;;
esac
case "$TWORD" in
    HOST-LISTENER=foreign*) ok "2 TREATMENT took the FOREIGN branch: $TWORD" ;;
    *)                      bad "2 TREATMENT word is '$TWORD', expected HOST-LISTENER=foreign" ;;
esac
if [ -n "$CWORD" ] && [ "$CWORD" != "$TWORD" ]; then
    ok "3 the branch word FLIPPED with the holder - 2859 arm 1 is DIFFERENTIAL, not unconditional"
else
    bad "3 the branch word did NOT flip (control='$CWORD' treatment='$TWORD')"
fi

# the suite's OWN listener log: READY on the control, INUSE on the treatment
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
# is that the holder losing the race, or is the listener simply never accepting?
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

echo "-- $((11-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "INUSE-CONTROL-GREEN"; exit 0; }
echo "NOT-DIFFERENTIAL"; exit 20
