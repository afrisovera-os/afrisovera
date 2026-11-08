#!/usr/bin/env bash
# Verify `prevpentagonal <n>` reports the LARGEST pentagonal P(k)=k(3k-1)/2 (k>=1) <= n + index -- the
# mirror of nextpentagonal: 13->P(3)=12, 12->P(3)=12 (already), 4->P(1)=1, 1->P(1)=1, 50->P(5)=35.
# Distinct from pentagonal (membership). Self-checking: python largest-k.
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
OUT="${TMPDIR:-/tmp}/prevpentagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevpentagonal.png" \
    "echo warmup" "prevpentagonal 13" "prevpentagonal 12" "prevpentagonal 4" \
    "prevpentagonal 1" "prevpentagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevpentagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def penta(k): return k * (3 * k - 1) // 2

def want(n):                          # DERIVED reference: largest k>=1 with P(k) <= n
    k = 1
    while penta(k + 1) <= n:
        k += 1
    return f"prevpentagonal: largest pentagonal <= {n} is P({k}) = {penta(k)}"

for n in (13, 12, 4, 1, 50):
    w = want(n)
    if w not in s:
        print(f"PREVPENTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "prevpentagonal: largest pentagonal <= 13 is P(3) = 12" in s
assert "prevpentagonal: largest pentagonal <= 12 is P(3) = 12" in s
assert "prevpentagonal: largest pentagonal <= 4 is P(1) = 1" in s
assert "prevpentagonal: largest pentagonal <= 1 is P(1) = 1" in s
assert "prevpentagonal: largest pentagonal <= 50 is P(5) = 35" in s

print("PREVPENTAGONAL TEST PASSED  (13->P(3)=12, 12->P(3)=12, 4->P(1)=1, 1->P(1)=1, 50->P(5)=35 "
      "vs python largest-k)")
PY
