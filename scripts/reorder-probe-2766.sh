#!/usr/bin/env bash
# OUT-OF-ORDER RECEIVE PROBE (2.766) -- a MEASUREMENT, not a pass/fail suite.
#
# The question: when a segment arrives that is in-window but AHEAD of a hole,
# what does `net tcp serve` do with it? kernel/src/virtio.rs:2308-2320 says in
# prose that it drops it and repeats the ACK ("With no reassembly queue the only
# honest answer is..."), and 2.764 taught this project that a written claim about
# this stack's TCP behaviour is worth NOTHING until the real build produces it:
# design-2.764 §5 predicted a retransmission that four runs then showed does not
# happen, and a "not needed" finding about fix M3 reversed the moment it was
# ablated in the real kernel. So this script does not read the comment. It puts
# the reordered segments on the wire and reports what came back.
#
# The rig is scripts/tcp-serve-test.sh's, unchanged where it is load-bearing:
# `-netdev socket` (SLIRP cannot carry an inbound SYN without hostfwd, and
# hostfwd puts the host's stack under the peer), and the READINESS RULE -- the
# peer dials on the guest's own ARP who-has 10.0.2.2, never on a sleep.
# scripts/tcp-chat-peer.py --reorder-at N does the reordering: request N's second
# half goes out FIRST at its own sequence number, then its first half fills the
# hole. Same bytes, same places, only the arrival order is wrong.
#
# WHAT IT DECIDES, and the exit code that carries it (a verdict that survives
# being piped -- the 2.765 lesson, where `gate | tail -25` returned tail's 0 off
# a RED gate):
#     0  OOO-HELD     the guest kept the early half and echoed the request WHOLE
#                     in one segment with nothing retransmitted
#    10  OOO-DROPPED  the guest discarded it; the request only came back whole
#                     after the peer said it again, and came back in pieces
#    20  OOO-STALLED  the request never came back whole at all
#     1  RIG-FAILED   the probe never got far enough to decide (harness, not kernel)
# The same verdict is printed as the LAST line, so either derivation reads it.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel || { echo "PROBE VERDICT: RIG-FAILED (the kernel did not build)"; exit 1; }

T="${TMPDIR:-/tmp}"
PCAP="$T/reorder-probe-2766.pcap"
PORTFILE="$T/reorder-probe-2766.port"
PEERLOG="$T/reorder-probe-2766.peer"
GPORT=7783              # not 7777/7778/7779/7781 -- those belong to other suites
PEERPORT=50002          # tcp-chat-peer.py's OUR_PORT
EXCHANGES=4
REORDER_AT="${REORDER_AT:-3}"   # which exchange arrives out of order
GUEST_SECS=30
PEER_SECS=90
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

python3 scripts/tcp-chat-peer.py --selftest > "$T/reorder-probe-2766.selftest" 2>&1
if ! grep -q "SELFTEST PASS" "$T/reorder-probe-2766.selftest"; then
    sed 's/^/  /' "$T/reorder-probe-2766.selftest"
    echo "PROBE VERDICT: RIG-FAILED (the peer's own arithmetic selftest did not pass)"
    exit 1
fi
echo "  peer selftest PASS (4/4, including the reorder split and the three echo arms)"

python3 scripts/tcp-chat-peer.py "$PORTFILE" "$GPORT" "$PEER_SECS" "$EXCHANGES" \
        --reorder-at "$REORDER_AT" > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    kill "$PEER" 2>/dev/null
    echo "PROBE VERDICT: RIG-FAILED (the chat peer never bound a port)"
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the peer dials 10.0.2.15:$GPORT from port $PEERPORT,"
echo "  $EXCHANGES exchange(s), exchange $REORDER_AT arriving OUT OF ORDER"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:5104,server,nowait \
    -qmp tcp:127.0.0.1:5105,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# `net tcp serve` FIRST, while the gateway MAC is still unknown: the ARP that
# resolves it is the signal the peer dials on.
SER="$(python3 scripts/shell-demo.py 5104 5105 "$T/reorder-probe-2766.png" \
        "net tcp serve $GPORT $GUEST_SECS" "net" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "  --- the guest's serial ---"
echo "$SER" | grep -E "LISTEN|SYN|ESTABLISHED|ACCEPTED|serve:|FIN|WAIT|rx frames" | sed 's/^/  /'
echo "  --- the peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

# ---- the wire, around the reordered request --------------------------------
echo "  --- the wire ---"
python3 - "$PCAP" "$GPORT" "$PEERPORT" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
gport, pport = int(sys.argv[2]), int(sys.argv[3])
off, segs = 24, []
while off + 16 <= len(d):
    ts, tus, incl = struct.unpack("<III", d[off:off + 12])
    fr = d[off + 16:off + 16 + incl]
    off += 16 + incl
    if len(fr) < 54 or (fr[12] << 8 | fr[13]) != 0x0800 or fr[23] != 6:
        continue
    src = ".".join(str(x) for x in fr[26:30])
    ihl = (fr[14] & 0x0f) * 4
    ip_total = (fr[16] << 8) | fr[17]
    toff = 14 + ihl
    hlen = (fr[toff + 12] >> 4) * 4
    sport = (fr[toff] << 8) | fr[toff + 1]
    dport = (fr[toff + 2] << 8) | fr[toff + 3]
    seq, ack = struct.unpack(">II", fr[toff + 4:toff + 12])
    flags = fr[toff + 13]
    if not ((src == "10.0.2.15" and sport == gport and dport == pport) or
            (src == "10.0.2.2" and dport == gport and sport == pport)):
        continue
    segs.append((ts + tus / 1e6, src, seq, ack, flags, fr[toff + hlen:14 + ip_total]))
t0 = segs[0][0] if segs else 0
for (t, src, seq, ack, flags, pay) in segs:
    name = "".join(c for bit, c in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %8.3fs %-10s %-4s seq %#010x ack %#010x len %3d %r"
          % (t - t0, src, name or "-", seq, ack, len(pay), pay[:20]))
# The guest's distinct replies, keyed by seq so a retransmission collapses.
byseq = {}
for (t, src, seq, ack, flags, pay) in segs:
    if src == "10.0.2.15" and pay:
        byseq.setdefault(seq, pay)
print("  %d distinct reply segment(s) from the guest: %s"
      % (len(byseq), [byseq[k] for k in sorted(byseq)]))
PY

VERDICT="$(grep -o "OOO-[A-Z-]*" "$PEERLOG" 2>/dev/null | tail -1)"
DROPLINE=0
echo "$SER" | grep -qF "out-of-order segment at seq" && DROPLINE=1
echo "  the guest's own serial says it dropped an out-of-order segment: $DROPLINE"

case "$VERDICT" in
    OOO-HELD)
        echo "PROBE VERDICT: OOO-HELD (the guest reassembled; serial drop line: $DROPLINE)"
        exit 0 ;;
    OOO-DROPPED)
        echo "PROBE VERDICT: OOO-DROPPED (the guest discarded the early half; serial drop line: $DROPLINE)"
        exit 10 ;;
    OOO-STALLED)
        echo "PROBE VERDICT: OOO-STALLED (the reordered request never came back whole)"
        exit 20 ;;
    *)
        echo "PROBE VERDICT: RIG-FAILED (no OOO measurement in the peer log; peer exit $PEERRC)"
        exit 1 ;;
esac
