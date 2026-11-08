#!/usr/bin/env bash
# Verify the `lsvirtio` command enumerates the virtio-mmio transport slots (read-only
# bus discovery). Boots with a virtio-rng device attached so at least one populated
# slot (device id 4, virtio-rng) is guaranteed. Asserts the magic-gated scan lists
# the rng device and prints a device-count line >= 1. Slot numbers are NOT asserted
# (QEMU assigns them top-down and they depend on which devices are attached).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -device virtio-rng-device \
    -serial tcp:127.0.0.1:4760,server,nowait \
    -qmp tcp:127.0.0.1:4761,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/lsvirtio.out"
python3 scripts/shell-demo.py 4760 4761 "${TMPDIR:-/tmp}/lsvirtio.png" "lsvirtio" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lsvirtio:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

if "id 4 (virtio-rng)" not in s:
    print("LSVIRTIO TEST FAILED (virtio-rng (id 4) not enumerated)"); sys.exit(1)

m = re.search(r"lsvirtio: (\d+) virtio device\(s\) present", s)
if not m:
    print("LSVIRTIO TEST FAILED (missing device-count line)"); sys.exit(1)
count = int(m.group(1))
if count < 1:
    print(f"LSVIRTIO TEST FAILED (count {count} < 1)"); sys.exit(1)

print(f"LSVIRTIO TEST PASSED  (virtio-rng id 4 enumerated; {count} device(s) present -- magic-gated scan)")
PY
