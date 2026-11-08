#!/usr/bin/env bash
# Verify the `ilog2` command computes floor(log2 n) + power-of-two check.
# Deterministic:
#   ilog2 1024 -> 2^10, "= 10 (exact power of 2)"
#   ilog2 1000 -> "= 9" (not a power of 2)
#   ilog2 1    -> "= 0 (exact power of 2)"
#   ilog2 0    -> "undefined for 0"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4748,server,nowait \
    -qmp tcp:127.0.0.1:4749,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/ilog2.out"
python3 scripts/shell-demo.py 4748 4749 "${TMPDIR:-/tmp}/ilog2.png" "ilog2 1024" "ilog2 1000" "ilog2 1" "ilog2 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ilog2: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "ilog2: floor(log2(1024)) = 10 (exact power of 2)",
    "ilog2: floor(log2(1000)) = 9",
    "ilog2: floor(log2(1)) = 0 (exact power of 2)",
    "ilog2: undefined for 0",
]
missing = [w for w in want if w not in s]
# Guard: 1000 must NOT be flagged a power of 2.
if "floor(log2(1000)) = 9 (exact power of 2)" in s:
    print("ILOG2 TEST FAILED (1000 wrongly flagged as a power of 2)"); sys.exit(1)
if missing:
    print(f"ILOG2 TEST FAILED (missing: {missing})"); sys.exit(1)
print("ILOG2 TEST PASSED  (1024->10 pow2; 1000->9; 1->0 pow2; 0->undefined)")
PY
