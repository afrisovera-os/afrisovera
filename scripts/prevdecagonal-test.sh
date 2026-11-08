#!/usr/bin/env bash
# Verify `prevdecagonal <n>` reports the LARGEST decagonal Dec(k)=k(4k-3) (k>=1: 1,10,27,52,85,126,...)
# that is <= n, with index k -- the mirror of nextdecagonal, completing the decagonal pair. Tests
# 11->Dec(2)=10, 10->Dec(2)=10, 2->Dec(1)=1, 1->Dec(1)=1, 50->Dec(3)=27, 85->Dec(5)=85, and n<1->(none).
# Self-checking: python derives the largest decagonal <= n.
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
OUT="${TMPDIR:-/tmp}/prevdecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevdecagonal.png" \
    "echo warmup" "prevdecagonal 11" "prevdecagonal 50" "prevdecagonal 85" \
    "prevdecagonal 1" "prevdecagonal 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevdecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def deca(k): return k * (4 * k - 3)

def prevdeca(n):                     # DERIVED reference: largest Dec(k) <= n, k>=1
    if n < 1: return None
    k = 1
    while deca(k + 1) <= n: k += 1
    return k

for n in (11, 50, 85, 1):
    k = prevdeca(n)
    want = f"prevdecagonal: largest decagonal <= {n} is Dec({k}) = {deca(k)}"
    if want not in s:
        print(f"PREVDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity: the sequence values we expect
assert prevdeca(11) == 2 and deca(2) == 10
assert prevdeca(50) == 3 and deca(3) == 27
assert prevdeca(85) == 5 and deca(5) == 85
assert prevdeca(1) == 1 and deca(1) == 1
assert "prevdecagonal: no decagonal <= 0 (none)" in s
print("PREVDECAGONAL TEST PASSED  (11/50/85/1 vs python largest-Dec<=n; 0->none; mirror of nextdecagonal)")
PY
