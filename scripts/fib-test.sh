#!/usr/bin/env bash
# Verify the `fib` command computes the nth Fibonacci number (F0=0, F1=1) with an
# honest u64 overflow guard. Deterministic (constants confirmed against Python):
#   fib 10 -> 55
#   fib 90 -> 2880067194370816120
#   fib 93 -> 12200160415121876738   (largest Fibonacci that fits in u64)
#   fib 94 -> overflow (reported, not silently wrapped)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4758,server,nowait \
    -qmp tcp:127.0.0.1:4759,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/fib.out"
python3 scripts/shell-demo.py 4758 4759 "${TMPDIR:-/tmp}/fib.png" "fib 10" "fib 90" "fib 93" "fib 94" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fib: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "fib: F(10) = 55",
    "fib: F(90) = 2880067194370816120",
    "fib: F(93) = 12200160415121876738",
    "fib: F(94) overflows u64",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"FIB TEST FAILED (missing: {missing})"); sys.exit(1)
print("FIB TEST PASSED  (F10=55; F90/F93 exact; F94 overflow guarded)")
PY
