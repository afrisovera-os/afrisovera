#!/usr/bin/env bash
# Verify `eulerian <n> <k>` computes the Eulerian numbers A(n,k) -- permutations of 1..n with
# exactly k descents -- via A(n,k)=(k+1)A(n-1,k)+(n-k)A(n-1,k-1), A(0,0)=1. A(3,1)=4, A(4,1)=11,
# A(4,2)=11, A(5,2)=66, A(6,3)=302, A(4,5)=0 (k>=n). Self-checking: python Eulerian recurrence.
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
OUT="${TMPDIR:-/tmp}/eulerian.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/eulerian.png" \
    "echo warmup" "eulerian 3 1" "eulerian 4 1" "eulerian 4 2" \
    "eulerian 5 2" "eulerian 6 3" "eulerian 4 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "eulerian:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def eulerian(n, k):                    # DERIVED reference (row DP)
    if k >= max(n, 1) and not (n == 0 and k == 0):
        return 0
    row = [1]                          # A(0,0) = 1
    for i in range(1, n + 1):
        new = [1] + [0] * i            # A(i,0) = 1
        for j in range(1, i):
            new[j] = (j + 1) * row[j] + (i - j) * row[j - 1]
        row = new
    return row[k] if k < len(row) else 0

CASES = [(3, 1), (4, 1), (4, 2), (5, 2), (6, 3), (4, 5)]
for n, k in CASES:
    want = f"eulerian: A({n}, {k}) = {eulerian(n, k)}"
    if want not in s:
        print(f"EULERIAN TEST FAILED (missing '{want}')"); sys.exit(1)

assert eulerian(3, 1) == 4
assert eulerian(4, 1) == 11
assert eulerian(4, 2) == 11
assert eulerian(5, 2) == 66
assert eulerian(6, 3) == 302
assert eulerian(4, 5) == 0
import math
for n in range(1, 9):                  # row sums must be n!
    assert sum(eulerian(n, k) for k in range(n)) == math.factorial(n)
print("EULERIAN TEST PASSED  (4 / 11 / 11 / 66 / 302 / 0 + row-sum=n! vs python)")
PY
