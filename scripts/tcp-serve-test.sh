#!/usr/bin/env bash
# TCP SERVE check: prove an accepted connection SURVIVES more than one exchange.
#
# Suite 678 (`tcp-listen`) proved the guest can be CALLED. It then hangs up
# immediately -- `net_tcp_listen` falls from TcpAccept::Established straight into
# tcp_close, so the accepted connection's lifetime is zero data segments. Suites
# tcp/tcp-data/tcp-close/tcp-retransmit converse, but always as the DIALLING side
# and always for exactly one reply (`if acked && got > 0` returns). So "does a
# connection this guest did not start stay up for a CONVERSATION?" has never been
# asked. That is the only question this suite asks.
#
# The service is echo, chosen because it is the smallest server whose answer is
# DERIVABLE from the question: the reply can be asserted byte-for-byte instead of
# asserted to merely exist. The milestone is not the echo -- it is the second,
# third and fourth exchange riding the SAME connection.
#
# The rig is 2.756's wire and 2.759's readiness rule, unchanged, because both are
# load-bearing: SLIRP cannot carry an inbound connection to the guest without
# hostfwd, and hostfwd puts the HOST's TCP stack underneath the peer, so the SYN
# we meant to send is not the SYN that arrives; and WHEN to dial is answered on
# the wire (the guest's own ARP who-has 10.0.2.2) rather than by a sleep, the
# 2.758 lesson. scripts/tcp-chat-peer.py is the whole stack: ARP, DHCP, IPv4, TCP.
#
# What is settled, and where:
#   1. The peer's account -- N exchanges verified byte-for-byte, and a SECOND
#      SYN/ACK counted as a FAILURE rather than a retry, since three exchanges
#      over three connections is precisely what this suite exists to rule out.
#   2. The serial line -- LISTEN -> ACCEPTED -> per-exchange echo -> a close.
#   3. `rx frames` past 8. This path is the FIRST in this kernel that consumes
#      more than one descriptor pool's worth of frames on a single connection,
#      so 2.760's recycling is not incidentally exercised here, it is required:
#      without a RxSlot bind in the serve loop the listener goes deaf at frame 9,
#      mid-conversation, every time.
#   4. The capture -- ONE SYN, travelling 10.0.2.2 -> 10.0.2.15, ONE SYN/ACK, and
#      the guest's payloads equal to the peer's in order. Only the wire can show
#      that the exchanges shared one connection rather than three.
#
# TCP_SERVE_DUP=<n> (unset by default) additionally replays exchange n at its
# ORIGINAL seq -- a peer retransmit the guest has already consumed, which it must
# re-ACK and must NOT echo twice. That hazard is real (no path in this stack has
# ever read a second data segment, so "have I seen these bytes?" has never been
# asked) but it is deliberately OPT-IN: a doubled echo fails byte-for-byte
# INTERMITTENTLY, and the base suite stays decidable. See design §4(c).
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-serve-test.pcap"
PORTFILE="$T/tcp-serve-test.port"
PEERLOG="$T/tcp-serve-test.peer"
# The port the guest serves on. Fixed, and safe to fix: it is never bound on the
# host -- it exists only inside the guest and inside the Python peer. The one
# host port that IS bound (QEMU's end of the wire) is chosen by the OS, port 0,
# the 2.750 lesson.
GPORT=7779
PEERPORT=50002          # tcp-chat-peer.py's OUR_PORT, fixed so the serial can name it
EXCHANGES=6             # >=3 is the design bar; 6 also puts `rx frames` clear of 8
GUEST_SECS=30           # the serve window the guest is given
PEER_SECS=90            # the peer's own deadline, generously past the guest's
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

DUPARG=()
[ -n "${TCP_SERVE_DUP:-}" ] && DUPARG=(--dup-after "$TCP_SERVE_DUP")

python3 scripts/tcp-chat-peer.py "$PORTFILE" "$GPORT" "$PEER_SECS" "$EXCHANGES" \
        "${DUPARG[@]+"${DUPARG[@]}"}" > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-SERVE TEST FAILED  (the chat peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the peer dials 10.0.2.15:$GPORT from port $PEERPORT,"
echo "  then holds the connection open for $EXCHANGES exchange(s)"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:5102,server,nowait \
    -qmp tcp:127.0.0.1:5103,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# `net tcp serve` is the FIRST net command on purpose, exactly as in tcp-listen:
# it must run while the gateway MAC is still unknown, because the ARP that
# resolves it is the signal the peer dials on. A `net` or `net arp` before this
# could resolve it silently and the peer would never learn when to call. The
# `rx frames` reading is therefore taken AFTER the conversation, which is the
# only place it means anything anyway.
SER="$(python3 scripts/shell-demo.py 5102 5103 "$T/tcp-serve-test.png" \
        "net tcp serve $GPORT $GUEST_SECS" "net" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "$SER" | grep -E "LISTEN|SYN|ESTABLISHED|ACCEPTED|serve:|FIN|WAIT|rx frames" || true
echo "  --- chat peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0

# ---- 1. the peer's account ------------------------------------------------
for want in "dialing the guest" \
            "SYN/ACK from the guest" \
            "handshake COMPLETE, ESTABLISHED" \
            "exchange 3/$EXCHANGES VERIFIED byte-for-byte" \
            "exchange $EXCHANGES/$EXCHANGES VERIFIED byte-for-byte" \
            "echoed byte-for-byte on ONE connection"; do
    if ! grep -qF "$want" "$PEERLOG" 2>/dev/null; then
        echo "  the chat peer never reported: $want"
        FAIL=1
    fi
done
for bad in "the guest NEVER answered our SYN" \
           "nothing is listening on port" \
           "the exchanges did not share ONE connection" \
           "does not echo faithfully" \
           "the connection did not survive the conversation" \
           "duplicate inbound data is not detected" \
           "never dialed"; do
    if grep -qF "$bad" "$PEERLOG" 2>/dev/null; then
        echo "  the chat peer reports: $bad"
        FAIL=1
    fi
done
if [ "$PEERRC" -ne 0 ]; then
    echo "  the chat peer exited $PEERRC (it only exits 0 when every exchange came back"
    echo "  byte-for-byte on a single connection)"
    FAIL=1
fi

# ---- 2. the kernel's account ----------------------------------------------
# The serial contract this suite pins down (the kernel side of 2.762 must print
# these; see docs/design-2.762-long-lived-listener.md §5a).
for want in "state LISTEN" \
            "TCP SYN received from 10.0.2.2:$PEERPORT" \
            "connection ESTABLISHED" \
            "TCP inbound connection ACCEPTED on port $GPORT from 10.0.2.2:$PEERPORT" \
            "serve: exchange 1" \
            "serve: exchange 3" \
            "serve: exchange $EXCHANGES" \
            "serve: $EXCHANGES exchange(s) on one connection"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done
# The peer FINs once its exchanges are done, so this end closes from CLOSE_WAIT.
if ! echo "$SER" | grep -qE "state (CLOSE_WAIT|LAST_ACK|TIME_WAIT)"; then
    echo "  serial never shows the connection closing (CLOSE_WAIT/LAST_ACK/TIME_WAIT)"
    FAIL=1
fi
for bad in "usage: net tcp serve" \
           "unknown command" \
           "no SYN arrived on port" \
           "serve: RX starved" \
           "did NOT close cleanly"; do
    if echo "$SER" | grep -qF "$bad"; then
        echo "  serial reports a failed serve: $bad"
        FAIL=1
    fi
done

# ---- 3. rx frames past the descriptor pool --------------------------------
# QSZ is 8. A serve loop that forgets to recycle reads 8 frames and goes deaf
# mid-conversation; the counter is where that is observable rather than argued.
FRAMES="$(echo "$SER" | grep -o "rx frames [0-9]*" | tail -1 | grep -o "[0-9]*$")"
if [ -z "$FRAMES" ]; then
    echo "  no 'rx frames' line to read -- cannot tell whether the pool was recycled"
    FAIL=1
elif [ "$FRAMES" -le 8 ]; then
    echo "  rx frames $FRAMES -- at or under the 8-descriptor pool, so the serve loop"
    echo "  never returned a buffer and the listener went deaf mid-conversation"
    FAIL=1
else
    echo "  rx frames $FRAMES -- past the 8-descriptor pool, buffers were recycled"
fi

# ---- 4. the wire ----------------------------------------------------------
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

segs = []   # (t, src, seq, ack, flags, payload, ck_ok), in wire order
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
    segs.append((ts + tus / 1e6, src, seq, ack, flags, seg[hlen:], ck_ok))

t0 = segs[0][0] if segs else 0
for (t, src, seq, ack, flags, pay, ck) in segs:
    name = "".join(c for bit, c in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %8.3fs %-10s %-4s seq %#010x ack %#010x payload %3d %-18r checksum %s"
          % (t - t0, src, name or "-", seq, ack, len(pay), pay[:16], "OK" if ck else "BAD"))

bad = 0
def fail(msg):
    global bad
    print("  " + msg)
    bad = 1

if not segs:
    fail("no TCP segment for port %d is on the wire at all" % gport)
    sys.exit(1)

# --- the open: inbound, and there is only ONE of it ---
t, src, seq0, _a, f0, _p, _ck = segs[0]
if src != "10.0.2.2" or (f0 & 0x02) == 0 or (f0 & 0x10):
    fail("the first segment on port %d is %s flags %#04x -- an INBOUND connection "
         "must open with a SYN from 10.0.2.2" % (gport, src, f0))
    sys.exit(1)

# Dedupe by seq: a RETRANSMITTED SYN carries the same ISN, a SECOND CONNECTION
# does not. Only the second is a failure -- a loaded host may stretch the guest
# past the peer's dial retry, and a contention stretch must never be reported as
# a broken subsystem (the 2.755/2.757 lesson).
syn_isns = {s for (t, src, s, a, f, p, ck) in segs
            if src == "10.0.2.2" and (f & 0x02) and not (f & 0x10)}
sa_isns = {s for (t, src, s, a, f, p, ck) in segs
           if src == "10.0.2.15" and (f & 0x12) == 0x12}
if len(syn_isns) != 1:
    fail("%d distinct SYN ISNs from 10.0.2.2 -- the peer opened more than one connection"
         % len(syn_isns))
if len(sa_isns) != 1:
    fail("%d distinct SYN/ACK ISNs from the guest -- it handshook more than once, so the "
         "exchanges did NOT ride one connection" % len(sa_isns))
if not sa_isns:
    fail("the guest never sent a SYN/ACK -- nothing accepted the connection")
    sys.exit(bad)
if any(src == "10.0.2.15" and (f & 0x02) and not (f & 0x10)
       for (t, src, s, a, f, p, ck) in segs):
    fail("the GUEST sent a bare SYN on port %d -- that is an active open, not a serve" % gport)
gseq = sorted(sa_isns)[0]
gack = [a for (t, src, s, a, f, p, ck) in segs
        if src == "10.0.2.15" and (f & 0x12) == 0x12][0]
if gack != (seq0 + 1) & 0xffffffff:
    fail("the SYN/ACK acks %#010x, not %#010x -- the guest did not count the peer's SYN"
         % (gack, (seq0 + 1) & 0xffffffff))
if not all(ck for (t, src, s, a, f, p, ck) in segs):
    fail("a segment on this connection has a BAD checksum")
print("  one SYN 10.0.2.2 -> 10.0.2.15 (ISN %#010x), one SYN/ACK back (ISN %#010x)"
      % (sorted(syn_isns)[0], gseq))

# --- the conversation: the guest's payloads equal the peer's, in order ---
# Keyed by seq so a RETRANSMISSION of the same bytes at the same place collapses
# into the one segment it is, rather than counting as a second exchange.
def payloads(who):
    byseq = {}
    for (t, src, s, a, f, p, ck) in segs:
        if src == who and p:
            byseq.setdefault(s, p)
    return [byseq[k] for k in sorted(byseq)]

asked = payloads("10.0.2.2")
answered = payloads("10.0.2.15")
print("  %d distinct request(s) from the peer, %d distinct reply(ies) from the guest"
      % (len(asked), len(answered)))
if len(asked) < 3:
    fail("only %d request(s) reached the guest -- the connection did not survive long "
         "enough to be a conversation" % len(asked))
if len(answered) < 3:
    fail("the guest answered %d time(s) on this connection -- fewer than the 3 exchanges "
         "that make it a SERVER rather than an acceptor" % len(answered))
n = min(len(asked), len(answered))
for i in range(n):
    if answered[i] != asked[i]:
        fail("exchange %d came back WRONG on the wire: asked %r, answered %r"
             % (i + 1, asked[i], answered[i]))
        break
if len(answered) > len(asked):
    fail("the guest sent %d replies to %d requests -- it echoed something twice, so a "
         "duplicate segment was re-processed" % (len(answered), len(asked)))
if not bad:
    print("  %d exchange(s) echoed byte-for-byte on ONE connection" % n)

# --- the close ---
if not any(src == "10.0.2.2" and (f & 0x01) for (t, src, s, a, f, p, ck) in segs):
    fail("the peer never FINed -- the conversation was cut off, not finished")
if not any(src == "10.0.2.15" and (f & 0x01) for (t, src, s, a, f, p, ck) in segs):
    fail("the guest never sent a FIN -- the served connection was abandoned, not closed")
if any(f & 0x04 for (t, src, s, a, f, p, ck) in segs):
    fail("a RST is on the wire -- the connection was torn down, not closed")
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-SERVE TEST PASSED  (a peer dialed 10.0.2.15:$GPORT; the guest accepted and then STAYED, echoing $EXCHANGES exchange(s) byte-for-byte on ONE connection, past the 8-descriptor pool, and closed cleanly)"
    exit 0
fi
echo "TCP-SERVE TEST FAILED"
exit 1
