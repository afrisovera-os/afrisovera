#!/usr/bin/env bash
# TCP ACTIVE CLOSE check: prove a connection ENDS from our side, rather than
# being abandoned when the shell command returns. Three assertions, each of which
# can fail on its own:
#
#   1. The host peer's own account. It reads the payload, answers, and then reads
#      AGAIN — a read that returns b"" is EOF, which is the peer observing our
#      FIN. This is the assertion a kernel cannot fake by printing "closed": the
#      old code left the peer blocked in that second read until its timeout.
#   2. The serial line, walked through the states in order: FIN_WAIT_1 (our FIN
#      sent), FIN_WAIT_2 (their ACK of it), their FIN received and ACKed,
#      TIME_WAIT.
#   3. The capture, parsed here — the four closing segments AND the sequence
#      arithmetic that ties them to the payload:
#        our FIN seq  == our data seq + len(payload)   (the FIN follows the bytes)
#        their ACK    >= our FIN seq + 1               (a FIN consumes a seqno)
#        our last ACK == their FIN seq + 1             (so does theirs)
#      A stack that only ever opened a connection has never had to be right about
#      any of that, because it never sent a segment after its payload.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-close-test.pcap"
PORTFILE="$T/tcp-close-test.port"
PEERLOG="$T/tcp-close-test.peer"
PAYLOAD="AFRISOVERA-TCP-CLOSE-SLICE"
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

# ---- the peer -------------------------------------------------------------
# Port 0 so the OS picks a free one: the 2.750 lesson (a busy port on the dev box
# is a harness fault wearing the costume of a kernel regression).
python3 - "$PORTFILE" > "$PEERLOG" 2>&1 <<'PY' &
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
port = s.getsockname()[1]
s.listen(1)
s.settimeout(90)
with open(sys.argv[1], "w") as f:
    f.write(str(port))
print("peer listening on 127.0.0.1:%d" % port, flush=True)
try:
    c, a = s.accept()
except socket.timeout:
    print("peer: NO CONNECTION within 90s", flush=True)
    sys.exit(1)
c.settimeout(30)
try:
    data = c.recv(1024)
except socket.timeout:
    data = b""
print("peer received %d bytes: %s" % (len(data), data.decode("ascii", "replace")), flush=True)
if data:
    c.sendall(b"AFRISOVERA-REPLY-OK " + str(len(data)).encode() + b"\n")
    print("peer replied", flush=True)

# The point of this suite. The peer does NOT close first: it reads again and
# waits for the guest to end the connection. b"" is EOF -- the guest's FIN,
# observed by the other end rather than announced by ours. Against a kernel that
# abandons the connection this read blocks until the timeout below.
c.settimeout(25)
try:
    tail = c.recv(1024)
except socket.timeout:
    print("peer: NO EOF within 25s -- the guest never closed its side", flush=True)
    tail = None
except OSError as e:
    # An abandoned connection typically ends as a RESET when the guest goes
    # away, not as EOF. Reported as the statement it is: a traceback here reads
    # like a broken harness when it is actually the defect being measured.
    print("peer: connection RESET, never EOF (%s) -- the guest never closed its side" % e,
          flush=True)
    tail = None
if tail == b"":
    print("peer saw EOF: the guest closed its side", flush=True)
elif tail:
    print("peer received %d unexpected trailing bytes" % len(tail), flush=True)
# Only now does the peer close, which is what puts its FIN after ours on the
# wire and makes the direction of the close unambiguous in the capture.
c.close()
s.close()
print("peer closed", flush=True)
PY
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-CLOSE TEST FAILED  (the host peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  peer port $PORT (guest reaches it as 10.0.2.2:$PORT through SLIRP)"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4544,server,nowait \
    -qmp tcp:127.0.0.1:4545,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4544 4545 "$T/tcp-close-test.png" \
        "net arp" "net tcp send $PORT $PAYLOAD" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null

echo "$SER" | grep -E "ESTABLISHED|TCP data sent|peer ACKed|TCP received|round-trip OK|FIN|TIME_WAIT|CLOSED|closed" || true
echo "  --- host peer ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0

# 1. The peer's own account: the bytes arrived, and then EOF did.
if ! grep -q "peer received ${#PAYLOAD} bytes: $PAYLOAD" "$PEERLOG" 2>/dev/null; then
    echo "  the host peer did not receive our exact ${#PAYLOAD}-byte payload"
    FAIL=1
fi
if ! grep -q "peer saw EOF: the guest closed its side" "$PEERLOG" 2>/dev/null; then
    echo "  the host peer never saw EOF -- the connection was abandoned, not closed"
    FAIL=1
fi

# 2. The kernel's account, state by state.
for want in "TCP data round-trip OK" "state FIN_WAIT_1" "state FIN_WAIT_2" \
            "TCP peer FIN received" "state TIME_WAIT"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done
# And it must NOT report the failure states.
for bad in "did NOT close cleanly" "never ACKed" "closed this end; the peer did not"; do
    if echo "$SER" | grep -qF "$bad"; then
        echo "  serial reports a failed close: $bad"
        FAIL=1
    fi
done

# 3. The wire.
python3 - "$PCAP" "$PAYLOAD" "$PORT" <<'PY'
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

segs = []   # (src, sport, dport, seq, ack, flags, plen, payload, ck_ok), in wire order
while off + 16 <= len(d):
    incl = struct.unpack("<I", d[off + 8:off + 12])[0]
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
    seq = struct.unpack(">I", fr[toff + 4:toff + 8])[0]
    ack = struct.unpack(">I", fr[toff + 8:toff + 12])[0]
    flags = fr[toff + 13]
    seg = fr[toff:14 + ip_total]
    payload = seg[hlen:]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", len(seg))
    ck_ok = (csum(pseudo + seg) == 0)
    # Only this connection: the boot's DHCP/ARP traffic is not part of the claim.
    ours = (src == "10.0.2.15" and dport == port)
    theirs = (src == "10.0.2.2" and sport == port)
    if not (ours or theirs):
        continue
    segs.append((src, seq, ack, flags, len(payload), payload, ck_ok))

for (src, seq, ack, flags, plen, _p, ck) in segs:
    name = "".join(n for bit, n in ((0x02, "S"), (0x10, "A"), (0x08, "P"), (0x01, "F"), (0x04, "R"))
                   if flags & bit)
    print("  %-10s %-4s seq %#010x ack %#010x payload %3d checksum %s"
          % (src, name or "-", seq, ack, plen, "OK" if ck else "BAD"))

bad = 0
def fail(msg):
    global bad
    print("  " + msg)
    bad = 1

our_data = next(((s, p) for (src, s, a, f, pl, p, ck) in segs
                 if src == "10.0.2.15" and p == want), None)
if our_data is None:
    fail("our payload segment is not on the wire")
    sys.exit(1)
data_seq = our_data[0]

# Our FIN: after the payload, and only after it.
our_fin = next(((s, a, ck) for (src, s, a, f, pl, p, ck) in segs
                if src == "10.0.2.15" and (f & 0x01)), None)
if our_fin is None:
    fail("we never sent a FIN -- the connection was abandoned")
    sys.exit(1)
fin_seq, _fa, fin_ck = our_fin
if not fin_ck:
    fail("our FIN has a BAD checksum")
want_fin_seq = (data_seq + len(want)) & 0xffffffff
if fin_seq != want_fin_seq:
    fail("our FIN seq %#010x does not account for the %d-byte payload (want %#010x)"
         % (fin_seq, len(want), want_fin_seq))

# Their ACK of it. A FIN consumes one sequence number, so it must cover seq+1.
after_fin = (fin_seq + 1) & 0xffffffff
if not any(src == "10.0.2.2" and (f & 0x10) and
           ((a - after_fin) & 0xffffffff) < 0x80000000
           for (src, s, a, f, pl, p, ck) in segs):
    fail("the peer never ACKed our FIN (nothing covering %#010x)" % after_fin)

# Their FIN, and our final ACK of it -- which must be their FIN's seq + 1.
their_fin = next(((s, pl) for (src, s, a, f, pl, p, ck) in segs
                  if src == "10.0.2.2" and (f & 0x01)), None)
if their_fin is None:
    fail("the peer never sent a FIN")
else:
    want_last_ack = (their_fin[0] + their_fin[1] + 1) & 0xffffffff
    if not any(src == "10.0.2.15" and (f & 0x10) and not (f & 0x01)
               and a == want_last_ack and ck
               for (src, s, a, f, pl, p, ck) in segs):
        fail("we never sent the final ACK of their FIN (want ack %#010x)" % want_last_ack)

# Ordering: our FIN has to come after our payload, or "close" meant something else.
idx_data = next(i for i, (src, s, a, f, pl, p, ck) in enumerate(segs)
                if src == "10.0.2.15" and p == want)
idx_fin = next(i for i, (src, s, a, f, pl, p, ck) in enumerate(segs)
               if src == "10.0.2.15" and (f & 0x01))
if idx_fin < idx_data:
    fail("our FIN is on the wire BEFORE our payload")
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-CLOSE TEST PASSED  (active close: our FIN after ${#PAYLOAD} payload bytes, their ACK, their FIN, our final ACK; peer observed EOF; FIN_WAIT_1 -> FIN_WAIT_2 -> TIME_WAIT)"
    exit 0
fi
echo "TCP-CLOSE TEST FAILED"
exit 1
