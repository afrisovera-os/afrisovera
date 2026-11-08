#!/usr/bin/env bash
# Verify the `isqrt` command computes the integer floor square root (Newton's
# method, no floats). Deterministic:
#   isqrt 100     -> "isqrt: isqrt(100) = 10 (10^2 = 100)"      (perfect square)
#   isqrt 99      -> "isqrt: isqrt(99) = 9 (9^2 = 81)"          (floor of a non-square)
#   isqrt 1000000 -> "isqrt: isqrt(1000000) = 1000 (1000^2 = 1000000)"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4730,server,nowait \
    -qmp tcp:127.0.0.1:4731,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/isqrt.out"
python3 scripts/shell-demo.py 4730 4731 "${TMPDIR:-/tmp}/isqrt.png" "isqrt 100" "isqrt 99" "isqrt 1000000" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isqrt: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["isqrt(100) = 10", "isqrt(99) = 9", "isqrt(1000000) = 1000"]
missing = [w for w in want if w not in s]
if missing:
    print(f"ISQRT TEST FAILED (missing: {missing})"); sys.exit(1)
print("ISQRT TEST PASSED  (100->10; 99->9; 1000000->1000)")
PY
