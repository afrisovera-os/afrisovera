#!/usr/bin/env bash
# Verify the virtio-rng (entropy) driver: boot with a virtio-rng device, confirm
# the driver negotiates it, and that `entropy` reads real random bytes from it.
# Output is random, so we assert bytes were obtained, not a specific value.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -device virtio-rng-device \
    -serial tcp:127.0.0.1:4670,server,nowait \
    -qmp tcp:127.0.0.1:4671,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/virtio-rng.out"
python3 scripts/shell-demo.py 4670 4671 "${TMPDIR:-/tmp}/virtio-rng.png" "entropy" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "virtio-rng|entropy" "$OUT" | sed 's/^/  /'

# Note: the "[virtio-rng] up" banner prints at boot (before the TCP client
# connects, so it is dropped). Obtaining entropy bytes below is itself proof the
# driver negotiated the device (rng_present() gates the read).
python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"virtio-rng gave (\d+) bytes: ([0-9a-f ]+)", s)
if not m:
    print("VIRTIO-RNG TEST FAILED (entropy command did not return bytes)"); sys.exit(1)
n = int(m.group(1)); hexbytes = m.group(2).split()
if n >= 1 and len(hexbytes) >= 1:
    print(f"VIRTIO-RNG TEST PASSED  (driver up; read {n} hardware entropy bytes)")
    sys.exit(0)
print("VIRTIO-RNG TEST FAILED (no entropy bytes)"); sys.exit(1)
PY
