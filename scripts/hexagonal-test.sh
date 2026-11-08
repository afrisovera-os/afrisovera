#!/usr/bin/env bash
# Verify the `hexagonal <n>` command classifies whether n is a HEXAGONAL number
# H(k)=k(2k-1) (k>=1: 1,6,15,28,45,...), reporting the index k. A number is
# hexagonal iff 8n+1 is a perfect square AND its root % 4 == 3; then k=(root+1)/4.
# Rounds out the figurate family (triangular/pronic/issquare/pentagonal).
# Self-checking: python's math.isqrt recomputes the verdict + index.
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
OUT="${TMPDIR:-/tmp}/hexagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hexagonal.png" \
    "echo warmup" "hexagonal 1" "hexagonal 6" "hexagonal 15" "hexagonal 28" \
    "hexagonal 45" "hexagonal 66" "hexagonal 190" "hexagonal 0" \
    "hexagonal 10" "hexagonal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hexagonal:|usage: hexagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    m = 8 * n + 1
    r = math.isqrt(m)
    if r * r == m and r % 4 == 3:
        k = (r + 1) // 4
        return f"hexagonal: {n} IS hexagonal (H({k}) = {n})"
    return f"hexagonal: {n} is NOT hexagonal"

# hexagonal: 1,6,15,28,45,66,190 (with indices) ; not: 0, 10
for n in [1, 6, 15, 28, 45, 66, 190, 0, 10]:
    want = line(n)
    if want not in s:
        print(f"HEXAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the k>=1 convention (0 NOT) and 10 (triangular but not hexagonal)
assert line(15) == "hexagonal: 15 IS hexagonal (H(3) = 15)"
assert line(190) == "hexagonal: 190 IS hexagonal (H(10) = 190)"
assert line(0) == "hexagonal: 0 is NOT hexagonal"
assert line(10) == "hexagonal: 10 is NOT hexagonal"

if "usage: hexagonal <n>" not in s:
    print("HEXAGONAL TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("HEXAGONAL TEST PASSED  (1/6/15/28/45/66/190 IS with exact k, 0/10 NOT vs "
      "python; 10 is triangular but not hexagonal; bad->usage)")
PY
