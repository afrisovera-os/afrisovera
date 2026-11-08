#!/usr/bin/env bash
# Verify `stirling1 <n> <k>` computes the unsigned Stirling numbers of the FIRST kind c(n,k) --
# permutations of n elements with exactly k cycles -- via c(n,k)=c(n-1,k-1)+(n-1)*c(n-1,k),
# c(0,0)=1. c(4,2)=11, c(5,2)=50, c(3,1)=2, c(4,1)=6, c(4,4)=1, c(4,5)=0 (k>n). Self-checking:
# python Stirling-first-kind recurrence.
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
OUT="${TMPDIR:-/tmp}/stirling1.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stirling1.png" \
    "echo warmup" "stirling1 3 1" "stirling1 4 1" "stirling1 4 2" \
    "stirling1 5 2" "stirling1 4 4" "stirling1 4 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stirling1:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def stirling1(n, k):                   # DERIVED reference (row DP)
    if k > n:
        return 0
    row = [1]                          # c(0,0) = 1
    for i in range(1, n + 1):
        new = [0] + [0] * i            # c(i,0) = 0 for i >= 1
        for j in range(1, i + 1):
            prev = row[j] if j < len(row) else 0
            prevm1 = row[j - 1] if j - 1 < len(row) else 0
            new[j] = prevm1 + (i - 1) * prev
        row = new
    return row[k] if k < len(row) else 0

CASES = [(3, 1), (4, 1), (4, 2), (5, 2), (4, 4), (4, 5)]
for n, k in CASES:
    want = f"stirling1: c({n}, {k}) = {stirling1(n, k)}"
    if want not in s:
        print(f"STIRLING1 TEST FAILED (missing '{want}')"); sys.exit(1)

assert stirling1(3, 1) == 2
assert stirling1(4, 1) == 6
assert stirling1(4, 2) == 11
assert stirling1(5, 2) == 50
assert stirling1(4, 4) == 1
assert stirling1(4, 5) == 0
for n in range(1, 9):                  # row sums must be n! (over all permutations)
    import math
    assert sum(stirling1(n, k) for k in range(n + 1)) == math.factorial(n)
print("STIRLING1 TEST PASSED  (2 / 6 / 11 / 50 / 1 / 0 + row-sum=n! vs python)")
PY
