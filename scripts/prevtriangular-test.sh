#!/usr/bin/env bash
# Verify `prevtriangular <n>` reports the LARGEST triangular T(k)=k(k+1)/2 <= n + index -- mirror of
# nexttriangular: 11->T(4)=10, 10->T(4)=10 (already), 9->T(3)=6, 1->T(1)=1, 0->T(0)=0. Distinct from
# triangular (membership) / sumto (nth). Self-checking: python floor-index.
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
OUT="${TMPDIR:-/tmp}/prevtriangular.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevtriangular.png" \
    "echo warmup" "prevtriangular 11" "prevtriangular 10" "prevtriangular 9" \
    "prevtriangular 1" "prevtriangular 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevtriangular:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference: floor triangular index
    k = 0
    while (k + 1) * (k + 2) // 2 <= n:
        k += 1
    return f"prevtriangular: largest triangular <= {n} is T({k}) = {k*(k+1)//2}"

cases = [11, 10, 9, 1, 0]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"PREVTRIANGULAR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin notable branches: round down, already triangular, round down further, one, zero
assert "prevtriangular: largest triangular <= 11 is T(4) = 10" in s
assert "prevtriangular: largest triangular <= 10 is T(4) = 10" in s
assert "prevtriangular: largest triangular <= 9 is T(3) = 6" in s
assert "prevtriangular: largest triangular <= 1 is T(1) = 1" in s
assert "prevtriangular: largest triangular <= 0 is T(0) = 0" in s

print("PREVTRIANGULAR TEST PASSED  (11->T(4)=10, 10->T(4)=10, 9->T(3)=6, 1->T(1)=1, 0->T(0)=0 "
      "vs python floor-index)")
PY
