#!/usr/bin/env bash
# Verify `stirling2 <n> <k>` computes Stirling numbers of the second kind S(n,k): S(4,2)=7,
# S(5,3)=25, S(4,4)=1, S(5,2)=15, S(3,5)=0 (k>n). Row-DP recurrence. Self-checking: python
# uses the INDEPENDENT explicit inclusion-exclusion formula S(n,k)=1/k! * sum (-1)^j C(k,j)(k-j)^n.
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
OUT="${TMPDIR:-/tmp}/stirling2.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stirling2.png" \
    "echo warmup" "stirling2 4 2" "stirling2 5 3" "stirling2 4 4" \
    "stirling2 5 2" "stirling2 3 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stirling2:|usage: stirling2" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import comb, factorial
s = open(sys.argv[1], "rb").read().decode("latin1")

def S(n, k):                                   # INDEPENDENT explicit formula (not the DP)
    if k == 0:
        return 1 if n == 0 else 0
    total = sum((-1) ** j * comb(k, j) * (k - j) ** n for j in range(k + 1))
    return total // factorial(k)

for n, k in [(4, 2), (5, 3), (4, 4), (5, 2), (3, 5)]:
    w = f"stirling2: S({n}, {k}) = {S(n, k)}"
    if w not in s:
        print(f"STIRLING2 TEST FAILED (n={n} k={k} missing '{w}')"); sys.exit(1)

# pin the notable branches: S(4,2)=7, S(5,3)=25, S(n,n)=1, S(5,2)=15, k>n -> 0
assert "stirling2: S(4, 2) = 7" in s
assert "stirling2: S(5, 3) = 25" in s
assert "stirling2: S(4, 4) = 1" in s
assert "stirling2: S(5, 2) = 15" in s
assert "stirling2: S(3, 5) = 0" in s

print("STIRLING2 TEST PASSED  (4/2->7, 5/3->25, 4/4->1, 5/2->15, 3/5->0 "
      "vs python inclusion-exclusion formula)")
PY
