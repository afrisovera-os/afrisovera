#!/usr/bin/env bash
# Verify the `mean` command computes the integer (truncated) arithmetic mean.
# Deterministic:
#   mean 2 4 6       -> sum 12, count 3, mean 4
#   mean 10 20 30 40 -> sum 100, count 4, mean 25
#   mean 1 2         -> sum 3, count 2, mean 1  (integer/truncated: 1.5 -> 1)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4796,server,nowait \
    -qmp tcp:127.0.0.1:4797,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/mean.out"
python3 scripts/shell-demo.py 4796 4797 "${TMPDIR:-/tmp}/mean.png" "mean 2 4 6" "mean 10 20 30 40" "mean 1 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "mean: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "mean: sum 12, count 3, mean 4",
    "mean: sum 100, count 4, mean 25",
    "mean: sum 3, count 2, mean 1",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"MEAN TEST FAILED (missing: {missing})"); sys.exit(1)
print("MEAN TEST PASSED  (2,4,6->4; 10,20,30,40->25; 1,2->1 truncated)")
PY
