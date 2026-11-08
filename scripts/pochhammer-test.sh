#!/usr/bin/env bash
# Verify `pochhammer <n> <k>` computes the RISING factorial (n)^(k) = n(n+1)...(n+k-1). Tests
# 3 4 -> 360, 2 3 -> 24, 5 0 -> 1 (empty product), 1 5 -> 120 (=5!), 10 3 -> 1320. The rising
# companion of npr (falling factorial). Self-checking: python product of n..n+k-1.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/pochhammer.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pochhammer.png" \
    "echo warmup" "pochhammer 3 4" "pochhammer 2 3" "pochhammer 5 0" \
    "pochhammer 1 5" "pochhammer 10 3" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pochhammer:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import prod
s = open(sys.argv[1], "rb").read().decode("latin1")

def poch(n, k): return prod(range(n, n + k)) if k > 0 else 1

for n, k in [(3, 4), (2, 3), (5, 0), (1, 5), (10, 3)]:
    want = f"pochhammer: ({n})^({k}) = {poch(n, k)}"
    if want not in s:
        print(f"POCHHAMMER TEST FAILED (missing '{want}')"); sys.exit(1)

assert poch(3, 4) == 360 and poch(2, 3) == 24 and poch(5, 0) == 1
assert poch(1, 5) == 120 and poch(10, 3) == 1320
print("POCHHAMMER TEST PASSED  (3^4=360, 2^3=24, 5^0=1, 1^5=120, 10^3=1320 vs python)")
PY
