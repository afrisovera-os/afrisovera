#!/usr/bin/env bash
# TCP DATA check: prove the kernel can carry BYTES on an established connection,
# not merely reach one. Three assertions, each independent of the others:
#
#   1. `net tcp test` — the segment builder against synthetic segments, checked
#      by recomputing both checksums the way a peer would. No wire involved.
#   2. A real peer on the host: this script listens on 127.0.0.1:<port> (chosen
#      by the OS, so a busy port cannot red the suite), the kernel opens
#      10.0.2.2:<port> through SLIRP, sends a payload, and the peer reports the
#      bytes it actually received and answers. The peer's own log is the check
#      that the bytes ARRIVED — a kernel that prints "sent" proves nothing.
#   3. The packet capture, parsed here: our data segment with a valid
#      pseudo-header checksum over header+payload (the checksum a handshake-only
#      stack never had to get right, since its TCP length was always 20), the
#      peer's data segment, and our ACK covering the peer's bytes.
#
# Needs python3. QEMU is invoked by bare name (the runner puts it on PATH).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PCAP="$T/tcp-data-test.pcap"
PORTFILE="$T/tcp-data-test.port"
PEERLOG="$T/tcp-data-test.peer"
PAYLOAD="AFRISOVERA-TCP-DATA-SLICE"
rm -f "$PCAP" "$PORTFILE" "$PEERLOG"

# ---- the peer -------------------------------------------------------------
# Binds port 0 so the OS picks a free port: a hard-coded port that something
# else on the dev box happens to hold would be a harness fault wearing the
# costume of a kernel regression.
python3 - "$PORTFILE" > "$PEERLOG" 2>&1 <<'PY' &
import socket, sys, time
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
time.sleep(2)
c.close()
s.close()
PY
PEER=$!

for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "TCP-DATA TEST FAILED  (the host peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi
echo "  peer port $PORT (guest reaches it as 10.0.2.2:$PORT through SLIRP)"

# ---- the kernel -----------------------------------------------------------
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4542,server,nowait \
    -qmp tcp:127.0.0.1:4543,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4542 4543 "$T/tcp-data-test.png" \
        "net tcp test" "net arp" "net tcp send $PORT $PAYLOAD" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
wait "$PEER" 2>/dev/null

echo "$SER" | grep -E "tcpseg\] [0-9]+/|TCP SYN sent|ESTABLISHED|TCP data sent|peer ACKed|TCP received|round-trip OK|TCP data:" || true
echo "  --- host peer ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

FAIL=0

# 1. The segment self-test.
if ! echo "$SER" | grep -qE "\[tcpseg\] 10/10 passed"; then
    echo "  segment self-test did NOT report 10/10"
    FAIL=1
fi

# 2. The peer's own account of what arrived.
if ! grep -q "peer received ${#PAYLOAD} bytes: $PAYLOAD" "$PEERLOG" 2>/dev/null; then
    echo "  the host peer did not receive our exact ${#PAYLOAD}-byte payload"
    FAIL=1
fi

# 3. The kernel's account of the round trip.
for want in "TCP data sent: ${#PAYLOAD} bytes" "TCP peer ACKed our ${#PAYLOAD} bytes" \
            "TCP received " "TCP data round-trip OK"; do
    if ! echo "$SER" | grep -qF "$want"; then
        echo "  serial is missing: $want"
        FAIL=1
    fi
done

# 4. The wire.
python3 - "$PCAP" "$PAYLOAD" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
want = sys.argv[2].encode()
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

our_data = None          # (seq, payload) of our PSH|ACK carrying the payload
our_data_ck = False
peer_data = None         # (seq, len) of the peer's answer
our_ack_of_peer = False

segs = []
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
    seq = struct.unpack(">I", fr[toff + 4:toff + 8])[0]
    ack = struct.unpack(">I", fr[toff + 8:toff + 12])[0]
    flags = fr[toff + 13]
    seg = fr[toff:14 + ip_total]
    payload = seg[hlen:]
    pseudo = fr[26:30] + fr[30:34] + bytes([0, 6]) + struct.pack(">H", len(seg))
    ck_ok = (csum(pseudo + seg) == 0)
    segs.append((src, seq, ack, flags, len(payload), ck_ok))
    if src == "10.0.2.15" and payload == want:
        our_data = (seq, payload)
        our_data_ck = ck_ok
    elif src == "10.0.2.2" and len(payload) > 0:
        if peer_data is None:
            peer_data = (seq, len(payload))

if peer_data is not None:
    end = (peer_data[0] + peer_data[1]) & 0xffffffff
    for (src, seq, ack, flags, plen, ck) in segs:
        if src == "10.0.2.15" and (flags & 0x10) and plen == 0 and ack == end and ck:
            our_ack_of_peer = True

for (src, seq, ack, flags, plen, ck) in segs:
    print("  %-10s flags %#04x payload %3d checksum %s" % (src, flags, plen, "OK" if ck else "BAD"))

bad = 0
if our_data is None:
    print("  our payload segment is NOT on the wire")
    bad = 1
elif not our_data_ck:
    print("  our payload segment has a BAD checksum (header+payload)")
    bad = 1
if peer_data is None:
    print("  the peer sent no data segment")
    bad = 1
if not our_ack_of_peer:
    print("  we never ACKed the peer's data")
    bad = 1
sys.exit(bad)
PY
[ $? -ne 0 ] && FAIL=1

if [ "$FAIL" -eq 0 ]; then
    echo "TCP-DATA TEST PASSED  (10/10 segment cases; ${#PAYLOAD} bytes delivered to a real host peer, its reply received and ACKed, checksums OK on the wire)"
    exit 0
fi
echo "TCP-DATA TEST FAILED"
exit 1
