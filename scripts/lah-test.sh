#!/usr/bin/env bash
# Verify `lah <n> <k>` computes the unsigned Lah numbers L(n,k) (partitions into k ordered lists).
# Tests L(4,2)=36, L(3,2)=6, L(4,1)=24, L(5,3)=120, and L(3,5)=0 (k>n). Self-checking: python
# recurrence L(n,k)=L(n-1,k-1)+(n+k-1)L(n-1,k).
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
OUT="${TMPDIR:-/tmp}/lah.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lah.png" \
    "echo warmup" "lah 4 2" "lah 3 2" "lah 4 1" "lah 5 3" "lah 3 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lah:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from functools import lru_cache
s = open(sys.argv[1], "rb").read().decode("latin1")

@lru_cache(maxsize=None)
def L(n, k):
    if n == 0 and k == 0: return 1
    if k <= 0 or k > n: return 0
    return L(n - 1, k - 1) + (n + k - 1) * L(n - 1, k)

for n, k in [(4, 2), (3, 2), (4, 1), (5, 3), (3, 5)]:
    want = f"lah: L({n}, {k}) = {L(n, k)}"
    if want not in s:
        print(f"LAH TEST FAILED (missing '{want}')"); sys.exit(1)

assert L(4, 2) == 36 and L(3, 2) == 6 and L(4, 1) == 24 and L(5, 3) == 120 and L(3, 5) == 0
print("LAH TEST PASSED  (L(4,2)=36, L(3,2)=6, L(4,1)=24, L(5,3)=120, L(3,5)=0 vs python)")
PY
