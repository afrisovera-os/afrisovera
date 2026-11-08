#!/usr/bin/env bash
# TCP PASSIVE OPEN check: prove the guest ACCEPTS a connection it did not start.
#
# Every TCP suite before this one -- tcp, tcp-data, tcp-close, tcp-retransmit --
# has the kernel dialing out. That leaves the listening half of TCP untested by
# construction: nothing on the wire ever addressed the guest first, so a stack
# that cannot answer a SYN would pass all four. Here the PEER sends the SYN and
# the guest is the one that has to answer it.
#
# The rig is 2.756's, for a reason that is not convenience: SLIRP will not carry
# an inbound connection to the guest without hostfwd, and hostfwd puts the HOST's
# TCP stack back underneath the peer -- the SYN we meant to send is then not the
# SYN that arrives. So QEMU gets `-netdev socket` (raw Ethernet frames over a
# length-prefixed stream) and scripts/tcp-dial-peer.py is the whole stack: ARP,
# DHCP, IPv4 and TCP, with the dial under our control.
#
# WHEN to dial is a readiness question, and it is answered on the wire rather
# than with a sleep (the 2.758 lesson). `tcp_ready()` ARPs for the gateway when
# the MAC is still unknown, which is precisely what `net tcp listen` does first,
# so the peer dials on seeing that ARP. Frames on one socket are ordered, so the
# SYN cannot overtake the ARP reply the guest is waiting for.
#
# What is settled, and where:
#   1. The peer's own account -- it dialed, and the guest answered SYN/ACK with
#      an ack that counts the peer's SYN. A guest that cannot listen leaves the
#      peer reporting "the guest NEVER answered our SYN".
#   2. The serial line -- LISTEN, SYN_RECEIVED, SYN/ACK sent, ESTABLISHED,
#      ACCEPTED, and a clean TIME_WAIT close on top of the accepted connection.
#   3. The capture, which is where the direction is actually proven: the FIRST
#      segment of this connection travels 10.0.2.2 -> 10.0.2.15, and the guest
#      never sends a SYN of its own on that port. That is the difference between
#      a passive open and every suite before it, and only the wire shows it.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-listen-test.pcap"
PORTFILE="$T/tcp-listen-test.port"
PEERLOG="$T/tcp-listen-test.peer"
# The port the guest listens on. Fixed, and safe to fix: it is never bound on the
# host -- it exists only inside the guest and inside the Python peer. The one
# host port that IS bound (QEMU's end of the wire) is chosen by the OS, port 0,
# the 2.750 lesson.
GPORT=7778
PEERPORT=50001          # the peer's source port, fixed so the serial can name it
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

python3 scripts/tcp-dial-peer.py "$PORTFILE" "$GPORT" 75 4 > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-LISTEN TEST FAILED  (the dialing peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the peer dials 10.0.2.15:$GPORT from port $PEERPORT"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4546,server,nowait \
    -qmp tcp:127.0.0.1:4547,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# `net tcp listen` is the FIRST net command on purpose: it has to run while the
# gateway MAC is still unknown, because the ARP that resolves it is the signal
# the peer dials on. A `net arp` before this would resolve it silently and the
# peer would never learn when to call.
SER="$(python3 scripts/shell-demo.py 4546 4547 "$T/tcp-listen-test.png" \
        "net tcp listen $GPORT 20" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "$SER" | grep -E "LISTEN|SYN|ESTABLISHED|ACCEPTED|FIN|TIME_WAIT|listen:" || true
echo "  --- dialing peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0

# 1. The peer's account: it called, and the guest picked up.
for want in "dialing the guest" \
            "SYN/ACK from the guest" \
            "handshake COMPLETE, ESTABLISHED" \
            "peer verdict: the guest ACCEPTED an inbound connection"; do
    if ! grep -qF "$want" "$PEERLOG" 2>/dev/null; then
        echo "  the dialing peer never reported: $want"
        FAIL=1
    fi
done
for bad in "the guest NEVER answered our SYN" \
           "nothing is listening on port" \
           "the guest did not count our SYN" \
           "never dialed"; do
    if grep -qF "$bad" "$PEERLOG" 2>/dev/null; then
        echo "  the dialing peer reports: $bad"
        FAIL=1
    fi
done
if [ "$PEERRC" -ne 0 ]; then
    echo "  the dialing peer exited $PEERRC (it only exits 0 on a completed passive open)"
    FAIL=1
fi

# Informational, deliberately NOT a failure: whether the FIRST SYN was answered.
# It is the measure of the RX-cursor fix (the cursor is taken before the ARP that
# announces the listener, so a SYN that arrives in that gap is still seen), but a
# loaded host can stretch the guest past the peer's 4s retry, and a contention
# stretch must not be reported as a broken subsystem -- the 2.755/2.757 lesson.
if grep -qF "1 SYN(s) sent" "$PEERLOG" 2>/dev/null; then
    echo "  the FIRST SYN was answered (no dial retry was needed)"
else
    echo "  NOTE: the peer had to retry its SYN -- the passive open still completed"
fi

# 2. The kernel's account.
for want in "state LISTEN" \
            "TCP SYN received from 10.0.2.2:$PEERPORT" \
            "state SYN_RECEIVED" \
            "TCP SYN/ACK sent" \
            "connection ESTABLISHED" \
            "TCP inbound connection ACCEPTED on port $GPORT from 10.0.2.2:$PEERPORT" \
            "state TIME_WAIT"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done
for bad in "no SYN arrived on port" \
           "usage: net tcp listen" \
           "SYN/ACK unacknowledged" \
           "did NOT close cleanly"; do
    if echo "$SER" | grep -qF "$bad"; then
        echo "  serial reports a failed listen: $bad"
        FAIL=1
    fi
done

# 3. The wire: the direction of the open, which is the whole claim.
python3 - "$PCAP" "$GPORT" "$PEERPORT" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
gport, pport = int(sys.argv[2]), int(sys.argv[3])
off = 24

def csum(b):
    s = 0
    for i in range(0, len(b) - 1, 2):
        s += (b[i] << 8) | b[i + 1]
    if len(b) & 1:
        s += b[-1] << 8
    while s >> 16:
        s = (s & 0xffff) + (s >> 16)
    return (~s) & 0xffff

segs = []   # (t, src, seq, ack, flags, payload_len, ck_ok), in wire order
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
    seg = fr[toff:14 + ip_total]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", len(seg))
    ck_ok = (csum(pseudo + seg) == 0)
    ours = (src == "10.0.2.15" and sport == gport and dport == pport)
    theirs = (src == "10.0.2.2" and dport == gport and sport == pport)
    if not (ours or theirs):
        continue
    segs.append((ts + tus / 1e6, src, seq, ack, flags, len(seg) - hlen, ck_ok))

t0 = segs[0][0] if segs else 0
for (t, src, seq, ack, flags, n, ck) in segs:
    name = "".join(c for bit, c in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %8.3fs %-10s %-4s seq %#010x ack %#010x payload %3d checksum %s"
          % (t - t0, src, name or "-", seq, ack, n, "OK" if ck else "BAD"))

bad = 0
def fail(msg):
    global bad
    print("  " + msg)
    bad = 1

if not segs:
    fail("no TCP segment for port %d is on the wire at all" % gport)
    sys.exit(1)

# The direction. This is the assertion no earlier TCP suite could make: the
# connection's first segment came from the PEER, not from us.
t, src, seq0, _a, f0, _n, ck0 = segs[0]
if src != "10.0.2.2" or (f0 & 0x02) == 0 or (f0 & 0x10):
    fail("the first segment on port %d is %s flags %#04x -- an INBOUND connection "
         "must open with a SYN from 10.0.2.2" % (gport, src, f0))
    sys.exit(1)
print("  the connection opens with a SYN from 10.0.2.2 -- inbound, not ours")

# We must never have dialed on this port: a bare SYN from the guest would mean
# the suite proved an active open wearing a listener's name.
if any(src == "10.0.2.15" and (f & 0x02) and not (f & 0x10) for (t, src, s, a, f, n, ck) in segs):
    fail("the GUEST sent a bare SYN on port %d -- that is an active open, not a listen" % gport)

# The SYN/ACK: ours, and it has to count their SYN.
sa = [i for i, (t, src, s, a, f, n, ck) in enumerate(segs)
      if src == "10.0.2.15" and (f & 0x12) == 0x12]
if not sa:
    fail("the guest never sent a SYN/ACK -- nothing accepted the connection")
    sys.exit(bad)
i = sa[0]
_t, _s, gseq, gack, _f, _n, gck = segs[i]
if gack != (seq0 + 1) & 0xffffffff:
    fail("the SYN/ACK acks %#010x, not %#010x -- the guest did not count the peer's SYN"
         % (gack, (seq0 + 1) & 0xffffffff))
if not gck:
    fail("the SYN/ACK has a BAD TCP checksum")
if not all(ck for (t, src, s, a, f, n, ck) in segs):
    fail("a segment on this connection has a BAD checksum")

# And the handshake's third segment, from them, covering our SYN.
third = [j for j in range(i + 1, len(segs))
         if segs[j][1] == "10.0.2.2" and (segs[j][4] & 0x10)
         and ((segs[j][3] - ((gseq + 1) & 0xffffffff)) & 0xffffffff) < 0x80000000]
if not third:
    fail("the peer never ACKed our SYN/ACK -- the handshake did not complete")
else:
    print("  handshake complete on the wire: SYN <- 10.0.2.2, SYN/ACK -> , ACK <-")

# The close, from our side, on top of the accepted connection.
if not any(src == "10.0.2.15" and (f & 0x01) for (t, src, s, a, f, n, ck) in segs):
    fail("the guest never sent a FIN -- the accepted connection was abandoned, not closed")
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-LISTEN TEST PASSED  (a peer dialed 10.0.2.15:$GPORT; the guest answered SYN/ACK from LISTEN, reached ESTABLISHED on a connection it did not start, and closed it to TIME_WAIT)"
    exit 0
fi
echo "TCP-LISTEN TEST FAILED"
exit 1
