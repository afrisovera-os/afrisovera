#!/usr/bin/env bash
# Verify `nextpentagonal <n>` reports the SMALLEST pentagonal P(k)=k(3k-1)/2 (k>=1) >= n + its index:
# 2->P(2)=5, 12->P(3)=12 (already), 13->P(4)=22, 0->P(1)=1, 36->P(6)=51. Figurate sibling of
# nextsquare/nextcube/nexttriangular; distinct from pentagonal (membership). Self-checking: python.
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
OUT="${TMPDIR:-/tmp}/nextpentagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextpentagonal.png" \
    "echo warmup" "nextpentagonal 2" "nextpentagonal 12" "nextpentagonal 13" \
    "nextpentagonal 0" "nextpentagonal 36" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextpentagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def penta(k): return k * (3 * k - 1) // 2

def want(n):                          # DERIVED reference: smallest k>=1 with P(k) >= n
    k = 1
    while penta(k) < n:
        k += 1
    return f"nextpentagonal: smallest pentagonal >= {n} is P({k}) = {penta(k)}"

cases = [2, 12, 13, 0, 36]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"NEXTPENTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

# pin notable branches: round up, already pentagonal, round up, zero->P(1), mid-range
assert "nextpentagonal: smallest pentagonal >= 2 is P(2) = 5" in s
assert "nextpentagonal: smallest pentagonal >= 12 is P(3) = 12" in s
assert "nextpentagonal: smallest pentagonal >= 13 is P(4) = 22" in s
assert "nextpentagonal: smallest pentagonal >= 0 is P(1) = 1" in s
assert "nextpentagonal: smallest pentagonal >= 36 is P(6) = 51" in s

print("NEXTPENTAGONAL TEST PASSED  (2->P(2)=5, 12->P(3)=12, 13->P(4)=22, 0->P(1)=1, 36->P(6)=51 "
      "vs python smallest-k)")
PY
