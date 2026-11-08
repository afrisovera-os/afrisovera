#!/usr/bin/env bash
# RX descriptor recycling: prove the NIC can still hear after it has already
# received more frames than it has descriptors.
#
# The defect this suite was written for (found by 2.759, fixed by 2.760):
# `net_setup` posts QSZ = 8 RX descriptors once and NOTHING ever returned a
# consumed buffer to the device, so a boot had a LIFETIME budget of 8 inbound
# frames. Frame 9 had nowhere to land, the device dropped it, and the guest was
# permanently deaf for the rest of that boot — every later ping timed out with
# `rx frames` frozen at 8.
#
# So the test is simply: ping the gateway TEN times in ONE boot. Ten is past the
# eight-descriptor pool with the DHCP lease already spent out of it, which is why
# the old kernel gets roughly half of them and then goes silent forever.
#
# Two witnesses, and the pair is the point:
#   * the SERIAL line — ten "ping OK" replies seen by the DRIVER, and not one
#     "no echo reply within timeout";
#   * the PCAP — ten ICMP echo REPLIES actually on the wire.
# The wire is what separates "the network stopped answering" from "we stopped
# listening": under the old kernel the replies are in the capture and the guest
# never sees them, which places the fault in the driver and nowhere else.
#
# Needs qemu + python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel
PCAP="${TMPDIR:-/tmp}/net-rx-recycle.pcap"
rm -f "$PCAP"

PINGS=10

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -object filter-dump,id=f0,netdev=n0,file="$PCAP" \
    -serial tcp:127.0.0.1:4718,server,nowait \
    -qmp tcp:127.0.0.1:4719,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot

CMDS=(net)
for _ in $(seq 1 "$PINGS"); do CMDS+=("net ping"); done
CMDS+=(net)

SER="$(python3 scripts/shell-demo.py 4718 4719 "${TMPDIR:-/tmp}/net-rx-recycle.png" "${CMDS[@]}" 2>&1)"
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

OK="$(echo "$SER" | grep -c "ping OK")"
LOST="$(echo "$SER" | grep -c "no echo reply within timeout")"
echo "$SER" | grep -E "rx frames" || true
echo "pings answered: $OK/$PINGS   (timed out: $LOST)"

[ "$OK" -eq "$PINGS" ] \
    && echo "RX RECYCLE: all $PINGS echo replies reached the driver in ONE boot (past the 8-descriptor pool)" \
    || { echo "NET RX RECYCLE TEST FAILED (only $OK of $PINGS pings answered; the NIC went deaf mid-boot)"; exit 1; }
[ "$LOST" -eq 0 ] \
    || { echo "NET RX RECYCLE TEST FAILED ($LOST ping(s) timed out)"; exit 1; }

# The last `net` line reports the driver's lifetime RX counter. It must be past
# NBUF: a counter that stops at 8 IS the exhausted pool, whatever else passed.
FRAMES="$(echo "$SER" | grep -o "rx frames [0-9]*" | tail -1 | grep -o "[0-9]*$")"
[ -n "$FRAMES" ] \
    || { echo "NET RX RECYCLE TEST FAILED (no 'rx frames' line to read)"; exit 1; }
[ "$FRAMES" -gt 8 ] \
    && echo "RX COUNTER: $FRAMES frames received this boot (> the 8 descriptors posted at setup, so buffers were reused)" \
    || { echo "NET RX RECYCLE TEST FAILED (rx frames stuck at $FRAMES, i.e. never past the descriptor pool)"; exit 1; }

python3 - "$PCAP" "$PINGS" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
want = int(sys.argv[2])
off, req, rep = 24, 0, 0
while off + 16 <= len(d):
    incl = struct.unpack("<I", d[off+8:off+12])[0]
    frame = d[off+16:off+16+incl]; off += 16 + incl
    if len(frame) < 35 or (frame[12] << 8 | frame[13]) != 0x0800:
        continue
    if frame[14 + 9] != 1:                      # IPv4 proto ICMP
        continue
    ihl = (frame[14] & 0x0f) * 4
    t = frame[14 + ihl]
    if t == 8:
        req += 1
    elif t == 0:
        rep += 1
print(f"wire: {req} ICMP echo request(s) out, {rep} echo reply(s) in")
if rep >= want:
    print(f"NET RX RECYCLE TEST PASSED  ({rep} replies on the wire, all {want} heard by the driver)")
    sys.exit(0)
print(f"NET RX RECYCLE TEST FAILED  (only {rep} echo replies on the wire, wanted {want})")
sys.exit(1)
PY
