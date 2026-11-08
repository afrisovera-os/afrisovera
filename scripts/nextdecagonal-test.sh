#!/usr/bin/env bash
# Verify `nextdecagonal <n>` reports the SMALLEST decagonal Dec(k)=k(4k-3) (k>=1) >= n + index -- the
# decagonal member of the nextsquare/.../nextnonagonal family: 2->Dec(2)=10, 10->Dec(2)=10 (already),
# 11->Dec(3)=27, 0->Dec(1)=1, 60->Dec(5)=85. Distinct from decagonal (membership). Python smallest-k.
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
OUT="${TMPDIR:-/tmp}/nextdecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextdecagonal.png" \
    "echo warmup" "nextdecagonal 2" "nextdecagonal 10" "nextdecagonal 11" \
    "nextdecagonal 0" "nextdecagonal 60" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextdecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def deca(k): return k * (4 * k - 3)

def want(n):                          # DERIVED reference: smallest k>=1 with Dec(k) >= n
    k = 1
    while deca(k) < n:
        k += 1
    return f"nextdecagonal: smallest decagonal >= {n} is Dec({k}) = {deca(k)}"

for n in (2, 10, 11, 0, 60):
    w = want(n)
    if w not in s:
        print(f"NEXTDECAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "nextdecagonal: smallest decagonal >= 2 is Dec(2) = 10" in s
assert "nextdecagonal: smallest decagonal >= 10 is Dec(2) = 10" in s
assert "nextdecagonal: smallest decagonal >= 11 is Dec(3) = 27" in s
assert "nextdecagonal: smallest decagonal >= 0 is Dec(1) = 1" in s
assert "nextdecagonal: smallest decagonal >= 60 is Dec(5) = 85" in s

print("NEXTDECAGONAL TEST PASSED  (2->Dec(2)=10, 10->Dec(2)=10, 11->Dec(3)=27, 0->Dec(1)=1, "
      "60->Dec(5)=85 vs python smallest-k)")
PY
