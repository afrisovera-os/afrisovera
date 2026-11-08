#!/usr/bin/env bash
# Verify `prevoctagonal <n>` reports the LARGEST octagonal Oct(k)=k(3k-2) (k>=1) <= n + index -- the
# mirror of nextoctagonal: 9->Oct(2)=8, 8->Oct(2)=8 (already), 7->Oct(1)=1, 1->Oct(1)=1,
# 50->Oct(4)=40. Distinct from octagonal (membership). Self-checking: python largest-k.
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
OUT="${TMPDIR:-/tmp}/prevoctagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevoctagonal.png" \
    "echo warmup" "prevoctagonal 9" "prevoctagonal 8" "prevoctagonal 7" \
    "prevoctagonal 1" "prevoctagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevoctagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def octa(k): return k * (3 * k - 2)

def want(n):                          # DERIVED reference: largest k>=1 with Oct(k) <= n
    k = 1
    while octa(k + 1) <= n:
        k += 1
    return f"prevoctagonal: largest octagonal <= {n} is Oct({k}) = {octa(k)}"

for n in (9, 8, 7, 1, 50):
    w = want(n)
    if w not in s:
        print(f"PREVOCTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "prevoctagonal: largest octagonal <= 9 is Oct(2) = 8" in s
assert "prevoctagonal: largest octagonal <= 8 is Oct(2) = 8" in s
assert "prevoctagonal: largest octagonal <= 7 is Oct(1) = 1" in s
assert "prevoctagonal: largest octagonal <= 1 is Oct(1) = 1" in s
assert "prevoctagonal: largest octagonal <= 50 is Oct(4) = 40" in s

print("PREVOCTAGONAL TEST PASSED  (9->Oct(2)=8, 8->Oct(2)=8, 7->Oct(1)=1, 1->Oct(1)=1, "
      "50->Oct(4)=40 vs python largest-k)")
PY
