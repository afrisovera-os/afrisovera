#!/usr/bin/env bash
# Verify `nextnonagonal <n>` reports the SMALLEST nonagonal Non(k)=k(7k-5)/2 (k>=1) >= n + index -- the
# nonagonal member of the nextsquare/.../nextoctagonal family: 2->Non(2)=9, 9->Non(2)=9 (already),
# 10->Non(3)=24, 0->Non(1)=1, 50->Non(5)=75. Distinct from nonagonal (membership). Python smallest-k.
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
OUT="${TMPDIR:-/tmp}/nextnonagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextnonagonal.png" \
    "echo warmup" "nextnonagonal 2" "nextnonagonal 9" "nextnonagonal 10" \
    "nextnonagonal 0" "nextnonagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextnonagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nona(k): return k * (7 * k - 5) // 2

def want(n):                          # DERIVED reference: smallest k>=1 with Non(k) >= n
    k = 1
    while nona(k) < n:
        k += 1
    return f"nextnonagonal: smallest nonagonal >= {n} is Non({k}) = {nona(k)}"

for n in (2, 9, 10, 0, 50):
    w = want(n)
    if w not in s:
        print(f"NEXTNONAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "nextnonagonal: smallest nonagonal >= 2 is Non(2) = 9" in s
assert "nextnonagonal: smallest nonagonal >= 9 is Non(2) = 9" in s
assert "nextnonagonal: smallest nonagonal >= 10 is Non(3) = 24" in s
assert "nextnonagonal: smallest nonagonal >= 0 is Non(1) = 1" in s
assert "nextnonagonal: smallest nonagonal >= 50 is Non(5) = 75" in s

print("NEXTNONAGONAL TEST PASSED  (2->Non(2)=9, 9->Non(2)=9, 10->Non(3)=24, 0->Non(1)=1, "
      "50->Non(5)=75 vs python smallest-k)")
PY
