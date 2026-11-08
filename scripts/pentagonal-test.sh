#!/usr/bin/env bash
# Verify the `pentagonal <n>` command classifies whether n is a PENTAGONAL number
# P(k)=k(3k-1)/2 (k>=1: 1,5,12,22,35,...), reporting the index k. A number is
# pentagonal iff 24n+1 is a perfect square AND its root % 6 == 5; then
# k=(root+1)/6. DISTINCT from triangular (k(k+1)/2), pronic (k(k+1)), issquare
# (k^2), sumto -- a different figurate family. n=0 is NOT pentagonal (k>=1 conv).
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
OUT="${TMPDIR:-/tmp}/pentagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pentagonal.png" \
    "echo warmup" "pentagonal 1" "pentagonal 5" "pentagonal 12" "pentagonal 22" \
    "pentagonal 35" "pentagonal 51" "pentagonal 145" "pentagonal 0" \
    "pentagonal 6" "pentagonal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pentagonal:|usage: pentagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    m = 24 * n + 1
    r = math.isqrt(m)
    if r * r == m and r % 6 == 5:
        k = (r + 1) // 6
        return f"pentagonal: {n} IS pentagonal (P({k}) = {n})"
    return f"pentagonal: {n} is NOT pentagonal"

# pentagonal: 1,5,12,22,35,51,145 (with indices) ; not: 0, 6
for n in [1, 5, 12, 22, 35, 51, 145, 0, 6]:
    want = line(n)
    if want not in s:
        print(f"PENTAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the k>=1 convention (0 is NOT)
assert line(12) == "pentagonal: 12 IS pentagonal (P(3) = 12)"
assert line(145) == "pentagonal: 145 IS pentagonal (P(10) = 145)"
assert line(0) == "pentagonal: 0 is NOT pentagonal"

if "usage: pentagonal <n>" not in s:
    print("PENTAGONAL TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("PENTAGONAL TEST PASSED  (1/5/12/22/35/51/145 IS with exact k, 0/6 NOT vs "
      "python; 0 not pentagonal per k>=1; bad->usage)")
PY
