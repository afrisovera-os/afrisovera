#!/usr/bin/env bash
# Verify the `octagonal <n>` command classifies whether n is an OCTAGONAL
# number Oct(k)=k(3k-2) (k>=1: 1,8,21,40,65,...), reporting the index k. A
# number is octagonal iff 3n+1 is a perfect square AND its root % 3 == 2; then
# k=(root+1)/3. Extends the figurate family (triangular/pronic/issquare/
# pentagonal/hexagonal/heptagonal). Self-checking: python's math.isqrt
# recomputes the verdict + index.
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
OUT="${TMPDIR:-/tmp}/octagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/octagonal.png" \
    "echo warmup" "octagonal 1" "octagonal 8" "octagonal 21" "octagonal 40" \
    "octagonal 65" "octagonal 133" "octagonal 0" "octagonal 7" \
    "octagonal 20" "octagonal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "octagonal:|usage: octagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    m = 3 * n + 1
    r = math.isqrt(m)
    if r * r == m and r % 3 == 2:
        k = (r + 1) // 3
        return f"octagonal: {n} IS octagonal (Oct({k}) = {n})"
    return f"octagonal: {n} is NOT octagonal"

# octagonal: 1,8,21,40,65,133 (with indices); not: 0,7,20
for n in [1, 8, 21, 40, 65, 133, 0, 7, 20]:
    want = line(n)
    if want not in s:
        print(f"OCTAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the k>=1 convention (0 NOT)
assert line(21) == "octagonal: 21 IS octagonal (Oct(3) = 21)"
assert line(133) == "octagonal: 133 IS octagonal (Oct(7) = 133)"
assert line(0) == "octagonal: 0 is NOT octagonal"
assert line(7) == "octagonal: 7 is NOT octagonal"

if "usage: octagonal <n>" not in s:
    print("OCTAGONAL TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("OCTAGONAL TEST PASSED  (1/8/21/40/65/133 IS with exact k, 0/7/20 NOT vs "
      "python; bad->usage)")
PY
