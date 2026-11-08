#!/usr/bin/env bash
# PROBE 3.518: READ the 3.517 absorption split -- do `rx raw` and `matched`
# actually DIVERGE on a run that discards traffic?
#
# WHY THIS FILE EXISTS AT ALL (it is not a second rig for its own sake):
# 3.517 added `RX_RAW_COUNT` beside the matched `RX_COUNT` and the guest prints
# `[net] rx raw N matched M`. It could not be READ, because
# `sink-probe-3488.sh:103` shows the guest serial through
#     grep -iE "sink|usage|LISTEN|SYN|ESTABLISHED|bytes|elapsed|rx frames"
# and that line matches NONE of those tokens. Editing that probe so it can see
# its own evidence is forbidden (2.765 rule 5), so 3.517 pre-registered exactly
# this: "a NEW caller (never an edit to sink-probe-3488.sh) that boots the same
# sink and greps the FULL serial for `rx raw`".
#
# WHAT IS REUSED RATHER THAN RE-ROLLED: the peer is `scripts/sink-peer-3488.py`
# CALLED UNEDITED -- the traffic-discarding regime (512 B segments, forced gaps,
# the guest dropping in-window frames) is the SAME one 3.513 located the loss in,
# not an imitation of it. Only the serial FILTER differs, which is the whole
# point of the tick.
#
# GRADING is delegated to `scripts/rxsplit-grade-3518.py` (pure, 14 arms, both
# directions reachable offline) and runs BEFORE any boot is spent (Law 9).
#     0  RXSPLIT-DIVERGES     raw > matched -- the split is load-bearing
#    10  RXSPLIT-IDENTICAL    raw == matched -- the split buys NOTHING; 3.517's
#                             prediction is FALSIFIED and the counter reverts
#     2  RXSPLIT-IMPOSSIBLE   raw < matched -- wiring is wrong
#     1  harness / inconclusive (incl. the line never printed)
#
# PRE-COMMITTED NON-EVIDENCE (stamped in the file, before the run): a DIVERGENCE
# here does NOT name the loss and does NOT claim the missing frames are the sink's
# dropped segments -- ARP, off-tuple and malformed frames are recycled uncounted
# too, and this probe cannot tell those apart. It answers ONE question: is the
# raw tally distinguishable from the matched one. The rate is NOT graded (3.486
# stays unmet), and the sink's own exit code is NOT this probe's verdict.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

if [ "${1:-}" = "--selftest" ]; then
    exec python3 scripts/rxsplit-grade-3518.py --selftest
fi

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "PROBE HARNESS FAILED (cargo build)"; exit 1; }

T="${TMPDIR:-/tmp}"
RUNID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
PCAP="$T/rxsplit-3518-$RUNID.pcap"
PORTFILE="$T/rxsplit-3518-$RUNID.port"
PEERLOG="$T/rxsplit-3518-$RUNID.peer"
SERFILE="$T/rxsplit-3518-$RUNID.serial"
PNG="$T/rxsplit-3518-$RUNID.png"
GPORT=7795              # inside the guest and the peer only
GUEST_SECS=20
PUSH_SECS=12
PEER_SECS=75
SERIAL=5126             # 5124/5125 are the sink probe's -- disjoint on purpose
QMP=5127
rm -f "$PCAP" "$PORTFILE" "$PEERLOG" "$SERFILE" "$PNG"

echo "== 3.518 absorption-split reader =="
echo "  run id: $RUNID"
echo "  same sink regime as 3.488 (peer CALLED unedited); the FULL serial is kept"

# Both graders are falsified before a boot is spent: mine, and the peer's own.
if ! python3 scripts/rxsplit-grade-3518.py --selftest; then
    echo "PROBE HARNESS FAILED (rxsplit grader selftest) -- no QEMU run spent"
    exit 1
fi
if ! python3 scripts/sink-peer-3488.py --selftest > "$T/rxsplit-3518-$RUNID.peerselftest" 2>&1; then
    echo "PROBE HARNESS FAILED (peer grader selftest) -- no QEMU run spent"
    exit 1
fi
echo "  peer selftest: $(grep -c '^  ok' "$T/rxsplit-3518-$RUNID.peerselftest") ok arm(s)"

python3 scripts/sink-peer-3488.py "$PORTFILE" "$GPORT" "$PEER_SECS" "$PUSH_SECS" > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "PROBE HARNESS FAILED (the peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the guest sinks $GPORT for ${GUEST_SECS}s"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:$SERIAL,server,nowait \
    -qmp tcp:127.0.0.1:$QMP,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

python3 scripts/shell-demo.py $SERIAL $QMP "$PNG" \
    "net tcp sink $GPORT $GUEST_SECS" "net" > "$SERFILE" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "  --- guest serial: the lines the 3.488 filter drops ---"
grep -nE "^\[net\] rx raw |rx frames " "$SERFILE" | sed 's/^/  /' || true
echo "  --- sink account (context only -- NOT this probe's verdict) ---"
grep -E "^\[net\] sink:" "$SERFILE" | sed 's/^/  /' || true
echo "  peer exit $PEERRC; serial kept at $SERFILE ($(wc -c < "$SERFILE") bytes)"

echo
python3 scripts/rxsplit-grade-3518.py --grade "$SERFILE"
RC=$?
case "$RC" in
  0)  echo "PROBE 3.518 -> RXSPLIT-DIVERGES (exit 0): frames come off the RX ring"
      echo "  that the matched tally never counts. The 3.517 split is load-bearing."
      echo "  NOT CLAIMED: that the difference IS the sink's lost segments." ;;
  10) echo "PROBE 3.518 -> RXSPLIT-IDENTICAL (exit 10): the two tallies agree on a"
      echo "  run that demonstrably discards traffic. 3.517's prediction is"
      echo "  FALSIFIED and the split buys nothing -- revert it." ;;
  2)  echo "PROBE 3.518 -> RXSPLIT-IMPOSSIBLE (exit 2): a tally taken BEFORE the"
      echo "  filter trails one taken after it. The counter wiring is wrong." ;;
  *)  echo "PROBE 3.518 -> INCONCLUSIVE (exit $RC): the guest never printed the"
      echo "  line, or the boot did not reach the 'net' command. Harness, not a"
      echo "  verdict about the counters." ;;
esac
exit "$RC"
