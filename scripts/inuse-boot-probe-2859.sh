#!/usr/bin/env bash
# Phase 2.859 - drive scripts/tcp-test.sh END TO END down its INUSE branch.
#
# 2.840 reached all three words of the host-listener contract (READY / INUSE /
# BIND-FAIL) against real sockets and MIRRORED tcp-test.sh's case statement,
# but wrote its own gap down: "it does NOT drive tcp-test.sh end-to-end down
# the INUSE path, because that costs a QEMU boot on the fixed serial ports
# 4540/4541 and would risk making a LIVE walk row lie".  No walk is live now.
#
# METHOD (2.765 rule 5): tcp-test.sh and tcp-host-listener.py are CALLED
# UNEDITED.  Only the ENVIRONMENT is arranged - a FOREIGN holder owns
# 127.0.0.1:80 before the suite starts, so the suite's own bind must lose.
#
# The claim under test is not "the case statement has an INUSE branch" (2.840
# proved that offline).  It is: with a stranger on the port, the suite still
# GRADES THE GUEST - the guest's SYN reaches the STRANGER's socket through
# slirp, the handshake completes, the suite PASSes, and it does not kill a
# process it never owned.
#
# Exit ladder: 10 INUSE-NOT-REACHED / 20 INUSE-PARTIAL / 0 INUSE-BOOT-OK.
set -uo pipefail
cd "$(dirname "$0")/.."

T="${TMPDIR:-/tmp}"
FLOG="$T/inuse-foreign-2859.log"      # the FOREIGN holder's own log
SLOG="$T/tcp-test-listener.log"       # the log tcp-test.sh writes (hard-coded)
OUT="$T/inuse-suite-2859.out"
PORT=80

FAIL=0
ok()  { echo "  ok   $*"; }
bad() { echo "  FAIL $*"; FAIL=$((FAIL+1)); }

FPID=""
cleanup() { [ -n "$FPID" ] && kill "$FPID" 2>/dev/null; }
trap cleanup EXIT

# --- arrange: a FOREIGN holder takes 127.0.0.1:80 first --------------------
rm -f "$FLOG"; : > "$FLOG"
python3 scripts/lib/tcp-host-listener.py 127.0.0.1 "$PORT" "$FLOG" &
FPID=$!
for _ in $(seq 1 50); do
    grep -qE '^(READY|INUSE|BIND-FAIL)' "$FLOG" 2>/dev/null && break
    sleep 0.1
done
FIRST="$(head -1 "$FLOG" 2>/dev/null)"
case "$FIRST" in
    READY*) echo "foreign holder: $FIRST (pid $FPID)" ;;
    *)      echo "PRECONDITION UNMET: foreign holder did not bind: '$FIRST'"
            echo "INUSE-NOT-REACHED"; exit 10 ;;
esac

# --- act: the suite, UNEDITED, one real QEMU boot --------------------------
echo "== running scripts/tcp-test.sh (unedited) against the held port =="
bash scripts/tcp-test.sh >"$OUT" 2>&1
SRC=$?
echo "suite rc=$SRC"
grep -E 'HOST-LISTENER|TCP TEST|host listener accepted' "$OUT" | sed 's/^/  | /'

# --- grade -----------------------------------------------------------------
echo "== grading =="
grep -q '^HOST-LISTENER=foreign' "$OUT" \
    && ok "1 suite took the INUSE branch: $(grep -m1 '^HOST-LISTENER' "$OUT")" \
    || { bad "1 suite did NOT print HOST-LISTENER=foreign: $(grep -m1 '^HOST-LISTENER' "$OUT")"
         echo "INUSE-NOT-REACHED"; exit 10; }

case "$(head -1 "$SLOG" 2>/dev/null)" in
    INUSE*) ok "2 the suite's own listener log says INUSE: $(head -1 "$SLOG")" ;;
    *)      bad "2 suite listener log first line is '$(head -1 "$SLOG" 2>/dev/null)', expected INUSE" ;;
esac
if grep -q '^ACCEPTED' "$SLOG" 2>/dev/null; then
    bad "3 the suite's losing listener logged an ACCEPTED - it never owned the socket"
else
    ok "3 the suite's losing listener accepted nothing (it never owned the socket)"
fi

GACC=$(grep -c '^ACCEPTED' "$FLOG" 2>/dev/null)
[ "${GACC:-0}" -ge 1 ] \
    && ok "4 the GUEST reached the FOREIGN socket: $GACC ACCEPTED - $(grep -m1 '^ACCEPTED' "$FLOG")" \
    || bad "4 foreign holder accepted 0 connections - the guest never got through slirp"

if kill -0 "$FPID" 2>/dev/null; then
    ok "5 foreign holder still alive after the suite (LPID=\"\" honoured: no stranger killed)"
else
    bad "5 foreign holder is GONE - the suite killed a process it never owned"
fi

if [ "$(python3 -c "
import socket,sys
s=socket.socket(); s.settimeout(3)
try:
    s.connect(('127.0.0.1',$PORT)); print('CONNECTED')
except OSError as e: print('NO:%s'%e)
finally: s.close()")" = CONNECTED ]; then
    ok "6 foreign holder still SERVES after the suite"
else
    bad "6 foreign holder no longer serves"
fi

[ "$SRC" = "0" ] && ok "7 suite rc=0 (it graded the GUEST, not the port owner)" \
                 || bad "7 suite rc=$SRC, expected 0"
grep -q '^TCP TEST PASSED' "$OUT" \
    && ok "8 verdict: $(grep -m1 '^TCP TEST PASSED' "$OUT")" \
    || bad "8 no TCP TEST PASSED line: $(grep -m1 'TCP TEST' "$OUT")"

echo "-- $((8-FAIL)) passed, $FAIL failed --"
[ "$FAIL" -eq 0 ] && { echo "INUSE-BOOT-OK"; exit 0; }
echo "INUSE-PARTIAL"; exit 20
