#!/usr/bin/env bash
# MEASURE-FIRST PROBE 2.769: does the listener know WHO CALLED IT?
#
# 2.767 made the DESTINATION a parameter for the dialling side -- `net tcp send
# <host>:<port>` now threads dst_ip from build_tcp_frame down through both the
# pseudo-header and the IP header. The listening side was not touched, and it
# still hard-codes `GW_IP` at every reply site (kernel/src/virtio.rs:2186, 2244,
# 2307, 2525, 2571, 2583, 2665, 2694, 2723, 2769). Worse, `parse_tcp` (:1773)
# never extracts the source address at all -- `Seg` has no field for it -- so the
# listener has no way to know who called even if it wanted to.
#
# That is not visible in any of the 683 green suites, because every inbound rig
# this project owns dials from 10.0.2.2: the one address the guest assumes has
# always been the right one. This probe changes the caller's address and nothing
# else, so what it measures is the assumption, not the rig.
#
# It reads TWO things off the wire (see scripts/peerip-peer-2769.py):
#   (1) the IP destination of the guest's own SYN/ACK, when the caller is at
#       10.0.2.99;
#   (2) whether a THIRD address, 10.0.2.88, can speak into that connection and
#       be echoed -- asked only AFTER the caller's own exchange has succeeded,
#       so a silent answer cannot be misread as a deaf guest.
#
# `net tcp serve` is used rather than `net tcp listen` because serve ECHOES, and
# an echo is what makes arm (2) observable at all: `listen` accepts and hangs up
# with a connection lifetime of zero data segments (2.762's header).
#
# EXIT CODE IS THE POINT -- this is a falsifier, and on the unpatched tree it is
# EXPECTED to be 10. The ladder the next steps walk down:
#     10  PEERIP-MISDIRECTED  the SYN/ACK went to the gateway (today)
#     20  PEERIP-PORTS-ONLY   destination fixed, matching still ports-only
#      0  PEERIP-ANSWERED     both halves fixed
#      1  harness/inconclusive
#
# This script is CALLED, never edited, by whatever suite later grades it -- the
# 2.765 rule 5: prose names the hazard, only a script names the artefact that
# avoids it.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

if [ "${1:-}" = "--selftest" ]; then
    exec python3 scripts/peerip-peer-2769.py --selftest
fi

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "PROBE HARNESS FAILED (cargo build)"; exit 1; }

T="${TMPDIR:-/tmp}"
PCAP="$T/peerip-probe-2769.pcap"
PORTFILE="$T/peerip-probe-2769.port"
PEERLOG="$T/peerip-probe-2769.peer"
GPORT=7791              # inside the guest and the Python peer only; never bound on the host
GUEST_SECS=25
PEER_SECS=75
SERIAL=5108             # 5102-5107 are taken by the existing net suites
QMP=5109
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

echo "== 2.769 peer-address probe =="
echo "  the caller dials from 10.0.2.99; the gateway is 10.0.2.2; a third party is 10.0.2.88"

# The grader is falsified BEFORE a boot is spent (Law §9, the 2.766 lesson).
if ! python3 scripts/peerip-peer-2769.py --selftest; then
    echo "PROBE HARNESS FAILED (grader selftest) -- no QEMU run spent"
    exit 1
fi

python3 scripts/peerip-peer-2769.py "$PORTFILE" "$GPORT" "$PEER_SECS" > "$PEERLOG" 2>&1 &
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
echo "  wire on 127.0.0.1:$PORT; the guest serves $GPORT for ${GUEST_SECS}s"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:$SERIAL,server,nowait \
    -qmp tcp:127.0.0.1:$QMP,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# `net tcp serve` FIRST, while the gateway MAC is still unknown: the ARP that
# resolves it is the signal the peer dials on (2.759's readiness rule).
SER="$(python3 scripts/shell-demo.py $SERIAL $QMP "$T/peerip-probe-2769.png" \
        "net tcp serve $GPORT $GUEST_SECS" "net" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "  --- guest serial (the listener's own account) ---"
echo "$SER" | grep -E "LISTEN|SYN|ESTABLISHED|ACCEPTED|serve:|FIN|WAIT|rx frames|echo" | sed 's/^/  /' || true
echo "  --- probe peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

echo "  --- capture ---"
echo "  pcap: $PCAP"

echo
case "$PEERRC" in
  0)  echo "PROBE 2.769 -> PEERIP-ANSWERED (exit 0): the listener answers its caller and "
      echo "  rejects a third party. Both halves fixed." ;;
  20) echo "PROBE 2.769 -> PEERIP-PORTS-ONLY (exit 20): the destination is fixed; inbound "
      echo "  matching is still ports-only." ;;
  10) echo "PROBE 2.769 -> PEERIP-MISDIRECTED (exit 10): the listener answered a host that "
      echo "  never called. THIS IS THE EXPECTED RESULT ON THE UNPATCHED TREE." ;;
  *)  echo "PROBE 2.769 -> INCONCLUSIVE (exit $PEERRC): see the peer log above." ;;
esac
exit "$PEERRC"
