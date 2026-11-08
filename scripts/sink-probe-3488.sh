#!/usr/bin/env bash
# FALSIFY-FIRST PROBE 3.488: is there ANY route for bulk bytes INTO the guest?
#
# 3.486 decided the Piper shape is (B) host-side synthesis, guest consumes PCM,
# and pre-registered the one measurement (B) rests on: >= 1x real time inbound
# (>= 44 kB/s at 22050 Hz mono, held >= 10 s). 3.487 measured the ROUTE before
# the RATE (the 3.261 rule) and found the route does not exist -- the PCM sink
# is green but its only two callers are local, and the only inbound verb ECHOES.
#
# The owed verb is `net tcp sink <port> <secs>`: accept one connection, ACK the
# payload, echo NOTHING, print bytes + elapsed at close. A NEW DISJOINT GRAMMAR
# beside `serve`, never a change to it (the 2.762 rule).
#
# THIS PROBE IS WRITTEN BEFORE THE VERB. On the unpatched tree it is EXPECTED
# to exit 10. The ladder the next steps walk down:
#     10  SINK-ABSENT    no SYN/ACK on the sink port -- `sink` parses as nothing
#     20  SINK-ECHOES    a connection, but our bytes came back: that is `serve`
#      0  SINK-MEASURED  ACKs advanced, nothing echoed -> a rate, from TWO
#                        independent derivations (the peer's, and the guest's
#                        own printed line)
#      1  harness / inconclusive
#
# Pre-committed non-evidence (stamped now, before any patch): echo throughput is
# NOT ingest throughput; a `longspeak`/`speaktext`/`audio` PASS says nothing
# about INBOUND throughput; the `[asili voice]` serial line is not evidence
# about audio at all (3.254).
#
# This script is CALLED, never edited, by whatever suite later grades it (the
# 2.765 rule 5). Needs python3; QEMU is invoked by bare name.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

if [ "${1:-}" = "--selftest" ]; then
    exec python3 scripts/sink-peer-3488.py --selftest
fi

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "PROBE HARNESS FAILED (cargo build)"; exit 1; }

T="${TMPDIR:-/tmp}"
# PER-RUN ARTEFACT NAMES (3.491's honest gap): until now every run wrote the SAME
# $T/sink-probe-3488.pcap, so 3.491's run silently overwrote the capture 3.490 had
# decoded and the two readings became permanently uncomparable -- the instrument
# destroyed its own comparison target. A run id fixes that AND removes the shared-
# worktree collision (two sessions probing at once used to share one port file).
RUNID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
PCAP="$T/sink-probe-3488-$RUNID.pcap"
PORTFILE="$T/sink-probe-3488-$RUNID.port"
PEERLOG="$T/sink-probe-3488-$RUNID.peer"
PNG="$T/sink-probe-3488-$RUNID.png"
GPORT=7795              # inside the guest and the peer only; never bound on the host
GUEST_SECS=20           # the sink window the guest is asked to hold
PUSH_SECS=12            # >= 10 s, the 3.486 hold
PEER_SECS=75
SERIAL=5124             # 5108/5109 are the peerip probe's
QMP=5125
rm -f "$PCAP" "$PORTFILE" "$PEERLOG" "$PNG"

echo "== 3.488 bulk-ingress falsifier =="
echo "  run id: $RUNID (every artefact this run writes carries it)"
echo "  asks the guest for 'net tcp sink $GPORT $GUEST_SECS' and pushes 512-byte segments at it"

# The grader is falsified BEFORE a boot is spent (Law 9, the 2.766 lesson).
if ! python3 scripts/sink-peer-3488.py --selftest; then
    echo "PROBE HARNESS FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi

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

SER="$(python3 scripts/shell-demo.py $SERIAL $QMP "$PNG" \
        "net tcp sink $GPORT $GUEST_SECS" "net" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "  --- guest serial (the sink's own account -- the SECOND derivation) ---"
echo "$SER" | grep -iE "sink|usage|LISTEN|SYN|ESTABLISHED|bytes|elapsed|rx frames" | sed 's/^/  /' || true
echo "  --- probe peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null
echo "  --- capture ---"
echo "  pcap:    $PCAP"
echo "  peerlog: $PEERLOG"
echo "  (per-run names: no earlier run capture was overwritten)"

echo
case "$PEERRC" in
  0)  echo "PROBE 3.488 -> SINK-MEASURED (exit 0): the guest consumed bulk inbound bytes."
      echo "  Read the rate off the peer line above and grade it against 3.486's threshold." ;;
  30) echo "PROBE 3.488 -> SINK-RATE-SHORT (exit 30): the route works, the RATE does not"
      echo "  meet 3.486's >= 44100 B/s. Only reachable with SINK_RATE_STRICT=1." ;;
  31) echo "PROBE 3.488 -> SINK-RATE-UNHELD (exit 31): fast enough, but not sustained"
      echo "  for 10 s. Only reachable with SINK_RATE_STRICT=1." ;;
  20) echo "PROBE 3.488 -> SINK-ECHOES (exit 20): a connection, but the bytes came back."
      echo "  That is 'serve'. Echo throughput is NOT ingest throughput." ;;
  10) echo "PROBE 3.488 -> SINK-ABSENT (exit 10): nothing answered on the sink port."
      echo "  THIS IS THE EXPECTED RESULT ON THE UNPATCHED TREE." ;;
  *)  echo "PROBE 3.488 -> INCONCLUSIVE (exit $PEERRC): see the peer log above." ;;
esac
exit "$PEERRC"
