#!/usr/bin/env bash
# virtio-net check: boot with a virtio-net device (modern transport) + a packet
# dump, send an ARP request from the driver, and confirm the frame was actually
# transmitted onto the wire (proves TX works end to end). SLIRP's reply also shows
# in the dump. Needs qemu + python3.
#
# NOTE: forces the MODERN virtio-mmio transport (-global virtio-mmio.force-legacy=
# false); the driver's queue setup uses the v2 registers.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
PCAP="${TMPDIR:-/tmp}/net-test.pcap"
rm -f "$PCAP"

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4496,server,nowait \
    -qmp tcp:127.0.0.1:4497,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
SER="$(python3 scripts/shell-demo.py 4496 4497 "${TMPDIR:-/tmp}/net-test.png" "net" "net arp" "ping" "net dns" "net demo" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null
echo "$SER" | grep -iE "virtio-net|ping OK" || true
echo "$SER" | grep -q "round-trip OK" \
    && echo "DRIVER RX: the ARP reply was received in the driver (round-trip OK)" \
    || { echo "NET TEST FAILED (driver did not receive the ARP reply)"; exit 1; }
echo "$SER" | grep -q "ping OK" \
    && echo "PING: ICMP echo reply received (IPv4 + ICMP round-trip OK)" \
    || { echo "NET TEST FAILED (no ICMP echo reply)"; exit 1; }
echo "$SER" | grep -q "UDP round-trip OK" \
    && echo "DNS: resolved example.com over UDP (UDP/DNS round-trip OK)" \
    || { echo "NET TEST FAILED (no DNS response)"; exit 1; }
echo "$SER" | grep -q "app called host 'net' without the capability" \
    && echo "CAP: a WASM app without CAP_NET was refused network access (capability gate works)" \
    || { echo "NET TEST FAILED (capability gate did not deny net access)"; exit 1; }

python3 - "$PCAP" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
off, n, sent_arp = 24, 0, False
while off + 16 <= len(d):
    incl = struct.unpack("<I", d[off+8:off+12])[0]
    frame = d[off+16:off+16+incl]; off += 16 + incl; n += 1
    if len(frame) >= 22 and (frame[12] << 8 | frame[13]) == 0x0806:
        op = frame[20] << 8 | frame[21]
        print(f"  frame {n}: {len(frame)}B ARP op={op} ({'request' if op==1 else 'reply'})")
        if op == 1:
            sent_arp = True
print(f"frames captured: {n}")
if sent_arp:
    print("NET TEST PASSED  (virtio-net transmitted a real ARP request)")
    sys.exit(0)
print("NET TEST FAILED  (no ARP request on the wire)")
sys.exit(1)
PY
