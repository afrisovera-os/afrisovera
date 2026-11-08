#!/usr/bin/env bash
# Verify `nextoctagonal <n>` reports the SMALLEST octagonal Oct(k)=k(3k-2) (k>=1) >= n + index -- the
# octagonal member of the nextsquare/.../nextheptagonal family: 2->Oct(2)=8, 8->Oct(2)=8 (already),
# 9->Oct(3)=21, 0->Oct(1)=1, 50->Oct(5)=65. Distinct from octagonal (membership). Python smallest-k.
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
OUT="${TMPDIR:-/tmp}/nextoctagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextoctagonal.png" \
    "echo warmup" "nextoctagonal 2" "nextoctagonal 8" "nextoctagonal 9" \
    "nextoctagonal 0" "nextoctagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextoctagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def octa(k): return k * (3 * k - 2)

def want(n):                          # DERIVED reference: smallest k>=1 with Oct(k) >= n
    k = 1
    while octa(k) < n:
        k += 1
    return f"nextoctagonal: smallest octagonal >= {n} is Oct({k}) = {octa(k)}"

for n in (2, 8, 9, 0, 50):
    w = want(n)
    if w not in s:
        print(f"NEXTOCTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "nextoctagonal: smallest octagonal >= 2 is Oct(2) = 8" in s
assert "nextoctagonal: smallest octagonal >= 8 is Oct(2) = 8" in s
assert "nextoctagonal: smallest octagonal >= 9 is Oct(3) = 21" in s
assert "nextoctagonal: smallest octagonal >= 0 is Oct(1) = 1" in s
assert "nextoctagonal: smallest octagonal >= 50 is Oct(5) = 65" in s

print("NEXTOCTAGONAL TEST PASSED  (2->Oct(2)=8, 8->Oct(2)=8, 9->Oct(3)=21, 0->Oct(1)=1, "
      "50->Oct(5)=65 vs python smallest-k)")
PY
