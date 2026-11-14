#!/usr/bin/env bash
# The packet-length lie (docs/security-model.md, gap G1). The guest dials a peer that answers with ONE 60-byte TCP
# segment whose IPv4 total length says 1400, then an honest 6-byte segment. A kernel that trusts the length reports
# "TCP received 1360 bytes": 1354 of them read from past the frame. The fixed kernel drops the lie (it reads frames
# as slices of the bytes that arrived, kernel/src/netparse.rs) and reports the 6 honest bytes.
#   EXPECT=lie scripts/net-lenlie-test.sh   grades a BEFORE kernel: passes only if the over-read is seen.
# Part 2 (architect's N4): the same lie to a LISTENER, `mesh listen 4401`: the peer dials, completes the handshake
# and sends a 60-byte segment claiming 260 data bytes. Graded on the wire: the guest's ACK says how many bytes it
# accepted (260 = it took bytes that never arrived and handed them to the mesh handler; 0 = it dropped the lie).
# Rig: -netdev socket, scripts/lenlie-peer.py is the whole network (DHCP, ARP, TCP), as in tcp-listen-test.sh.
# Both boots get a virtio-rng device: mesh listen refuses without a healthy entropy source (as in mesh-phase0).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

T="${TMPDIR:-/tmp}"
PORTFILE="$T/net-lenlie.port"
PEERLOG="$T/net-lenlie.peer"
PEERPORT=7311
EXPECT="${EXPECT:-fixed}"
rm -f "$PORTFILE" "$PEERLOG"

python3 scripts/lenlie-peer.py "$PORTFILE" "$PEERPORT" 75 > "$PEERLOG" 2>&1 &
PEER=$!
for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
if [ -z "$PORT" ]; then
    echo "NET-LENLIE TEST FAILED  (the peer never bound a port -- harness, not kernel)"
    kill "$PEER" 2>/dev/null
    exit 1
fi

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none -device virtio-rng-device \
    -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:4556,server,nowait \
    -qmp tcp:127.0.0.1:4557,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

SER="$(python3 scripts/shell-demo.py 4556 4557 "$T/net-lenlie.png" \
        "net tcp send 10.0.2.2:$PEERPORT hello" "echo still-here" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
kill "$PEER" 2>/dev/null
wait "$PEER" 2>/dev/null

echo "$SER" | grep -E "\[net\] TCP|still-here|PANIC" || true
echo "  --- peer ---"
sed 's/^/  /' "$PEERLOG" 2>/dev/null

# ---- part 2: the mesh listener ----
PEERLOG2="$T/net-lenlie-mesh.peer"
rm -f "$PORTFILE" "$PEERLOG2"
python3 scripts/lenlie-peer.py "$PORTFILE" --dial 4401 60 > "$PEERLOG2" 2>&1 &
PEER=$!
for _ in $(seq 1 40); do
    [ -s "$PORTFILE" ] && break
    sleep 0.25
done
PORT="$(cat "$PORTFILE" 2>/dev/null)"
qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M     -global virtio-mmio.force-legacy=false -device ramfb -display none -device virtio-rng-device     -netdev socket,id=n0,connect=127.0.0.1:$PORT -device virtio-net-device,netdev=n0     -serial tcp:127.0.0.1:4556,server,nowait     -qmp tcp:127.0.0.1:4557,server,nowait -kernel "$BIN" &
QPID=$!
afrish_wait_boot
SER2="$(python3 scripts/shell-demo.py 4556 4557 "$T/net-lenlie-mesh.png" "mesh listen 4401 12" "echo still-here" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
kill "$PEER" 2>/dev/null
wait "$PEER" 2>/dev/null
echo "$SER2" | grep -E "\[mesh\]|\[net\] serve|still-here|PANIC" | head -20 || true
echo "  --- mesh peer ---"
sed 's/^/  /' "$PEERLOG2" 2>/dev/null

FAIL=0
if ! grep -qF "LIE sent" "$PEERLOG2" 2>/dev/null; then
    echo "  mesh part: the peer never sent the lie (no handshake) -- harness, not a verdict"
    FAIL=1
fi
if echo "$SER2" | grep -qF "KERNEL PANIC"; then
    echo "  mesh part: the kernel panicked"
    FAIL=1
fi
if [ "$EXPECT" = lie ]; then
    grep -qF "acknowledged 260 lie byte(s)" "$PEERLOG2" || { echo "  BEFORE kernel, mesh part: the guest did not accept the 260 claimed bytes"; FAIL=1; }
else
    grep -qF "acknowledged 0 lie byte(s)" "$PEERLOG2" || { echo "  mesh part: the guest acknowledged bytes that never arrived"; FAIL=1; }
fi
if ! grep -qF "LIE sent" "$PEERLOG" 2>/dev/null; then
    echo "  the peer never sent the lie (the guest's data never arrived) -- harness, not a verdict"
    FAIL=1
fi
if echo "$SER" | grep -qF "KERNEL PANIC"; then
    echo "  the kernel panicked"
    FAIL=1
fi
if [ "$EXPECT" = lie ]; then
    if ! echo "$SER" | grep -qF "TCP received 1360 bytes"; then
        echo "  BEFORE kernel: the over-read was not seen (expected 'TCP received 1360 bytes')"
        FAIL=1
    fi
else
    if echo "$SER" | grep -qF "TCP received 1360 bytes"; then
        echo "  the kernel trusted the IP length: 1360 bytes 'received' from a 60-byte frame"
        FAIL=1
    fi
    if ! echo "$SER" | grep -qF "TCP received 6 bytes"; then
        echo "  the honest 6-byte segment after the lie was not received"
        FAIL=1
    fi
fi
if [ "$FAIL" -eq 0 ]; then
    echo "NET-LENLIE TEST PASSED ($EXPECT)"
    exit 0
fi
echo "NET-LENLIE TEST FAILED ($EXPECT)"
exit 1
