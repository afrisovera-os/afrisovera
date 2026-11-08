#!/usr/bin/env bash
# Verify `prevheptagonal <n>` reports the LARGEST heptagonal Hep(k)=k(5k-3)/2 (k>=1) <= n + index --
# the mirror of nextheptagonal: 8->Hep(2)=7, 7->Hep(2)=7 (already), 6->Hep(1)=1, 1->Hep(1)=1,
# 50->Hep(4)=34. Distinct from heptagonal (membership). Self-checking: python largest-k.
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
OUT="${TMPDIR:-/tmp}/prevheptagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevheptagonal.png" \
    "echo warmup" "prevheptagonal 8" "prevheptagonal 7" "prevheptagonal 6" \
    "prevheptagonal 1" "prevheptagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevheptagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hepta(k): return k * (5 * k - 3) // 2

def want(n):                          # DERIVED reference: largest k>=1 with Hep(k) <= n
    k = 1
    while hepta(k + 1) <= n:
        k += 1
    return f"prevheptagonal: largest heptagonal <= {n} is Hep({k}) = {hepta(k)}"

for n in (8, 7, 6, 1, 50):
    w = want(n)
    if w not in s:
        print(f"PREVHEPTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "prevheptagonal: largest heptagonal <= 8 is Hep(2) = 7" in s
assert "prevheptagonal: largest heptagonal <= 7 is Hep(2) = 7" in s
assert "prevheptagonal: largest heptagonal <= 6 is Hep(1) = 1" in s
assert "prevheptagonal: largest heptagonal <= 1 is Hep(1) = 1" in s
assert "prevheptagonal: largest heptagonal <= 50 is Hep(4) = 34" in s

print("PREVHEPTAGONAL TEST PASSED  (8->Hep(2)=7, 7->Hep(2)=7, 6->Hep(1)=1, 1->Hep(1)=1, "
      "50->Hep(4)=34 vs python largest-k)")
PY
