#!/usr/bin/env bash
# Verify `nexttriangular <n>` reports the SMALLEST triangular T(k)=k(k+1)/2 >= n + its index:
# 7->T(4)=10, 10->T(4)=10 (already), 11->T(5)=15, 1->T(1)=1, 0->T(0)=0. Sibling of nextsquare/
# nextcube; distinct from triangular (membership) / sumto (nth). Self-checking: python ceil-index.
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
OUT="${TMPDIR:-/tmp}/nexttriangular.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nexttriangular.png" \
    "echo warmup" "nexttriangular 7" "nexttriangular 10" "nexttriangular 11" \
    "nexttriangular 1" "nexttriangular 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nexttriangular:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference: ceil triangular index
    k = 0
    while k * (k + 1) // 2 < n:
        k += 1
    return f"nexttriangular: smallest triangular >= {n} is T({k}) = {k*(k+1)//2}"

cases = [7, 10, 11, 1, 0]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"NEXTTRIANGULAR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin notable branches: round up, already triangular, round up again, one, zero
assert "nexttriangular: smallest triangular >= 7 is T(4) = 10" in s
assert "nexttriangular: smallest triangular >= 10 is T(4) = 10" in s
assert "nexttriangular: smallest triangular >= 11 is T(5) = 15" in s
assert "nexttriangular: smallest triangular >= 1 is T(1) = 1" in s
assert "nexttriangular: smallest triangular >= 0 is T(0) = 0" in s

print("NEXTTRIANGULAR TEST PASSED  (7->T(4)=10, 10->T(4)=10, 11->T(5)=15, 1->T(1)=1, 0->T(0)=0 "
      "vs python ceil-index)")
PY
