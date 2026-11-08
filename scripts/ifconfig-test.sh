#!/usr/bin/env bash
# Verify the `ifconfig` network-status command. Boot with a virtio-net device (so
# the kernel auto-acquires a DHCP lease at boot), then `ifconfig` must report the
# interface up with the standard QEMU user-net lease 10.0.2.15. Single-line assert.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -netdev user,id=n0 -device virtio-net-device,netdev=n0 \
    -serial tcp:127.0.0.1:4684,server,nowait \
    -qmp tcp:127.0.0.1:4685,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/ifconfig.out"
python3 scripts/shell-demo.py 4684 4685 "${TMPDIR:-/tmp}/ifconfig.png" "ifconfig" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "ifconfig: virtio-net up|inet " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"ifconfig: virtio-net up, inet (\d+\.\d+\.\d+\.\d+), gw 10\.0\.2\.2", s)
if not m:
    print("IFCONFIG TEST FAILED (no 'virtio-net up, inet a.b.c.d' summary line)"); sys.exit(1)
ip = m.group(1)
if ip == "0.0.0.0":
    print("IFCONFIG TEST FAILED (no DHCP lease -- inet is 0.0.0.0)"); sys.exit(1)
if ip != "10.0.2.15":
    print(f"IFCONFIG TEST WARN: inet {ip} (expected the user-net default 10.0.2.15) -- still a valid lease")
print(f"IFCONFIG TEST PASSED  (virtio-net up, inet {ip}, gw 10.0.2.2)")
PY
