#!/usr/bin/env bash
# TCP RETRANSMISSION check: prove the guest recovers a LOST segment on its own
# timer, with the recovery witnessed by a peer that never saw the lost copy.
#
# The rig is different from tcp-data/tcp-close on purpose, and the difference is
# the whole point. Those suites talk to an ordinary host AF_INET socket through
# SLIRP, which means the HOST's TCP stack sits underneath the Python peer and
# acknowledges every segment before a byte reaches it: there is no seam in that
# rig to drop a packet into, and no amount of teaching the peer changes that.
# Here QEMU gets `-netdev socket` instead of `-netdev user` -- raw Ethernet
# frames over a length-prefixed stream -- and scripts/tcp-lossy-peer.py is the
# stack: ARP, IPv4 and TCP all happen in Python, where a segment can simply be
# thrown away.
#
# It throws away the FIRST copy of the data segment and nothing else. So:
#
#   1. The peer's own account. It reports the payload only after ACCEPTING COPY 2,
#      and it cannot report bytes it was never given -- the copy it acknowledges
#      and answers is, necessarily, the retransmission. A kernel that cannot
#      retransmit leaves this peer reporting "the guest NEVER retransmitted".
#   2. The serial line: the retransmit timer firing, then the ACK, the reply, and
#      a clean close on top of the recovered connection. A retransmission that
#      recovers nothing is not recovery.
#   3. The capture, parsed here, which is where the claim is actually settled:
#        * EXACTLY TWO segments from us carry the payload, with the SAME sequence
#          number (a resend that invented a new seq would leave the peer a hole);
#        * the peer sends NOTHING between them -- no ACK, no duplicate ACK,
#          nothing. The second copy was prompted by our own timer and by nothing
#          on the wire;
#        * the peer's ACK covering those bytes appears only AFTER the second copy;
#        * and the gap between the copies is a TIMER's worth of time, not an
#          instant re-send in the same breath.
#
# Boot: replacing SLIRP also removes its DHCP server, and the kernel runs a
# DISCOVER/REQUEST at boot. Measured, not estimated: left unanswered that
# exchange sits in its 60-million-spin timeout long enough that the first cut of
# this rig saw ONE frame on the wire and no shell at all within 36 s. So the peer
# answers DHCP as well, handing out SLIRP's own lease (10.0.2.15), and the prompt
# arrives in about a second of guest time. The suite still WAITS FOR THE PROMPT
# rather than sleeping a fixed number of seconds: boot on a loaded host stretches,
# and a fixed sleep turns host contention into a fake kernel regression (the 2.755
# lesson). Nothing this suite claims depends on the lease either way -- TCP's
# addresses here are static (10.0.2.15 -> 10.0.2.2) -- it is purely about getting
# to a prompt in seconds instead of minutes.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-retransmit-test.pcap"
PORTFILE="$T/tcp-retransmit-test.port"
PEERLOG="$T/tcp-retransmit-test.peer"
PAYLOAD="AFRISOVERA-TCP-RETRANSMIT-SLICE"
# The guest-visible TCP port. Fixed, and safe to fix: nothing on the host binds
# it -- it exists only inside the Python peer, which answers whatever arrives.
# The one port that IS bound on the host (QEMU's end of the wire) is chosen by
# the OS, port 0, the 2.750 lesson.
GPORT=7777
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

python3 scripts/tcp-lossy-peer.py "$PORTFILE" "$GPORT" 75 > "$PEERLOG" 2>&1 &
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-RETRANSMIT TEST FAILED  (the lossy peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  wire on 127.0.0.1:$PORT; the peer answers as 10.0.2.2:$GPORT and drops the first data segment"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4544,server,nowait \
    -qmp tcp:127.0.0.1:4545,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

# Wait for the shell prompt on the serial socket, then hand the socket over to
# shell-demo.py (QEMU re-listens after the disconnect -- verified, not assumed).
python3 - <<'PY'
import socket, sys, time
t0 = time.time()
try:
    s = socket.create_connection(("127.0.0.1", 4544), timeout=20)
except OSError as e:
    print("  the serial port never opened (%s)" % e)
    sys.exit(1)
s.settimeout(1.0)
buf = b""
while time.time() - t0 < 120:
    try:
        b = s.recv(4096)
    except socket.timeout:
        continue
    if not b:
        break
    buf += b
    if buf.rstrip().endswith(b"afrisovera>"):
        print("  shell prompt after %.1fs" % (time.time() - t0))
        s.close()
        sys.exit(0)
s.close()
print("  NO SHELL PROMPT within 120s -- the guest never got far enough to be tested")
sys.exit(1)
PY
BOOTED=$?
sleep 1

SER=""
if [ "$BOOTED" -eq 0 ]; then
    SER="$(python3 scripts/shell-demo.py 4544 4545 "$T/tcp-retransmit-test.png" \
            "net arp" "net tcp send $GPORT $PAYLOAD" 2>&1)"
fi
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null
PEERRC=$?

echo "$SER" | grep -E "ESTABLISHED|TCP data sent|retransmit|peer ACKed|TCP received|round-trip OK|FIN|TIME_WAIT|gone|timed out" || true
echo "  --- lossy peer (exit $PEERRC) ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0
[ "$BOOTED" -eq 0 ] || FAIL=1

# 1. The peer's account: it dropped one copy, and the bytes it finally reports
#    are the ones it accepted on the second.
for want in "DROPPED copy 1 of seq" \
            "ACCEPTED copy 2 of seq" \
            "peer received ${#PAYLOAD} bytes: $PAYLOAD" \
            "delivered on copy 2, 1 copy/copies dropped first"; do
    if ! grep -qF "$want" "$PEERLOG" 2>/dev/null; then
        echo "  the lossy peer never reported: $want"
        FAIL=1
    fi
done
if grep -qF "the guest NEVER retransmitted" "$PEERLOG" 2>/dev/null; then
    echo "  the lossy peer dropped the segment and the guest never sent it again"
    FAIL=1
fi
if [ "$PEERRC" -ne 0 ]; then
    echo "  the lossy peer exited $PEERRC (it delivers nothing unless a retransmission arrives)"
    FAIL=1
fi

# 2. The kernel's account.
for want in "TCP retransmit 1/4" "sent again after 300 ms with no ACK" \
            "TCP peer ACKed our ${#PAYLOAD} bytes" "TCP data round-trip OK" \
            "state TIME_WAIT"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done
for bad in "the peer is gone" "TCP data: timed out" "could not rebuild the segment"; do
    if echo "$SER" | grep -qF "$bad"; then
        echo "  serial reports a failed send: $bad"
        FAIL=1
    fi
done

# 3. The wire.
python3 - "$PCAP" "$PAYLOAD" "$GPORT" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
want = sys.argv[2].encode()
port = int(sys.argv[3])
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
    payload = seg[hlen:]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", len(seg))
    ck_ok = (csum(pseudo + seg) == 0)
    ours = (src == "10.0.2.15" and dport == port)
    theirs = (src == "10.0.2.2" and sport == port)
    if not (ours or theirs):
        continue
    segs.append((ts + tus / 1e6, src, seq, ack, flags, payload, ck_ok))

t0 = segs[0][0] if segs else 0
for (t, src, seq, ack, flags, p, ck) in segs:
    name = "".join(n for bit, n in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %8.3fs %-10s %-4s seq %#010x ack %#010x payload %3d checksum %s"
          % (t - t0, src, name or "-", seq, ack, len(p), "OK" if ck else "BAD"))

bad = 0
def fail(msg):
    global bad
    print("  " + msg)
    bad = 1

copies = [i for i, (t, src, s, a, f, p, ck) in enumerate(segs)
          if src == "10.0.2.15" and p == want]
if len(copies) != 2:
    fail("the payload is on the wire %d time(s), not twice -- one lost copy plus one "
         "retransmission is the whole claim" % len(copies))
    sys.exit(1)
i1, i2 = copies
t1, _s1, seq1, _a1, _f1, _p1, ck1 = segs[i1]
t2, _s2, seq2, _a2, _f2, _p2, ck2 = segs[i2]

if seq1 != seq2:
    fail("the retransmission carries seq %#010x, not the original %#010x -- the peer "
         "would see a hole where the lost bytes were" % (seq2, seq1))
if not (ck1 and ck2):
    fail("a copy of the payload has a BAD checksum")

# Nothing from the peer between the copies: the second one was prompted by our
# own timer, not by anything the peer said.
between = [segs[i] for i in range(i1 + 1, i2) if segs[i][1] == "10.0.2.2"]
if between:
    fail("the peer sent %d segment(s) between the two copies -- the retransmission "
         "may have been a reaction to those, not to a timer" % len(between))

# And its ACK of those bytes lands only after the second copy.
after = (seq1 + len(want)) & 0xffffffff
acks = [i for i, (t, src, s, a, f, p, ck) in enumerate(segs)
        if src == "10.0.2.2" and (f & 0x10) and ((a - after) & 0xffffffff) < 0x80000000]
if not acks:
    fail("the peer never ACKed the payload (nothing covering %#010x)" % after)
elif acks[0] < i2:
    fail("the peer ACKed the payload BEFORE the second copy -- the first one was not "
         "actually lost, so nothing here tests retransmission")

# A timer's worth of time, not the same breath.
gap = t2 - t1
if gap < 0.10:
    fail("the copies are %.3fs apart -- too close together to be a timer firing" % gap)
elif gap > 5.0:
    fail("the copies are %.3fs apart -- far past the 300 ms timer" % gap)
else:
    print("  the two copies are %.3fs apart (the retransmit timer)" % gap)

# The recovery has to be worth something: the peer's reply must be on the wire.
if not any(src == "10.0.2.2" and p for (t, src, s, a, f, p, ck) in segs):
    fail("the peer never sent data back -- the connection did not survive the loss")
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-RETRANSMIT TEST PASSED  (first copy of the ${#PAYLOAD}-byte payload dropped by the peer; resent on the 300 ms timer with the same seq, unprompted; accepted on copy 2, answered, closed)"
    exit 0
fi
echo "TCP-RETRANSMIT TEST FAILED"
exit 1
