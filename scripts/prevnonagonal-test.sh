#!/usr/bin/env bash
# Verify `prevnonagonal <n>` reports the LARGEST nonagonal Non(k)=k(7k-5)/2 (k>=1) <= n + index -- the
# mirror of nextnonagonal: 10->Non(2)=9, 9->Non(2)=9 (already), 8->Non(1)=1, 1->Non(1)=1,
# 50->Non(4)=46. Distinct from nonagonal (membership). Self-checking: python largest-k.
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
OUT="${TMPDIR:-/tmp}/prevnonagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevnonagonal.png" \
    "echo warmup" "prevnonagonal 10" "prevnonagonal 9" "prevnonagonal 8" \
    "prevnonagonal 1" "prevnonagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevnonagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nona(k): return k * (7 * k - 5) // 2

def want(n):                          # DERIVED reference: largest k>=1 with Non(k) <= n
    k = 1
    while nona(k + 1) <= n:
        k += 1
    return f"prevnonagonal: largest nonagonal <= {n} is Non({k}) = {nona(k)}"

for n in (10, 9, 8, 1, 50):
    w = want(n)
    if w not in s:
        print(f"PREVNONAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "prevnonagonal: largest nonagonal <= 10 is Non(2) = 9" in s
assert "prevnonagonal: largest nonagonal <= 9 is Non(2) = 9" in s
assert "prevnonagonal: largest nonagonal <= 8 is Non(1) = 1" in s
assert "prevnonagonal: largest nonagonal <= 1 is Non(1) = 1" in s
assert "prevnonagonal: largest nonagonal <= 50 is Non(4) = 46" in s

print("PREVNONAGONAL TEST PASSED  (10->Non(2)=9, 9->Non(2)=9, 8->Non(1)=1, 1->Non(1)=1, "
      "50->Non(4)=46 vs python largest-k)")
PY
