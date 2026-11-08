#!/usr/bin/env bash
# Verify the `heptagonal <n>` command classifies whether n is a HEPTAGONAL
# number Hep(k)=k(5k-3)/2 (k>=1: 1,7,18,34,55,...), reporting the index k. A
# number is heptagonal iff 40n+9 is a perfect square AND its root % 10 == 7;
# then k=(root+3)/10. Extends the figurate family (triangular/pronic/issquare/
# pentagonal/hexagonal). Self-checking: python's math.isqrt recomputes the
# verdict + index.
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
OUT="${TMPDIR:-/tmp}/heptagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/heptagonal.png" \
    "echo warmup" "heptagonal 1" "heptagonal 7" "heptagonal 18" "heptagonal 34" \
    "heptagonal 55" "heptagonal 112" "heptagonal 0" "heptagonal 6" \
    "heptagonal 17" "heptagonal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "heptagonal:|usage: heptagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    m = 40 * n + 9
    r = math.isqrt(m)
    if r * r == m and r % 10 == 7:
        k = (r + 3) // 10
        return f"heptagonal: {n} IS heptagonal (Hep({k}) = {n})"
    return f"heptagonal: {n} is NOT heptagonal"

# heptagonal: 1,7,18,34,55,112 (with indices); not: 0,6,17
for n in [1, 7, 18, 34, 55, 112, 0, 6, 17]:
    want = line(n)
    if want not in s:
        print(f"HEPTAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the k>=1 convention (0 NOT)
assert line(18) == "heptagonal: 18 IS heptagonal (Hep(3) = 18)"
assert line(112) == "heptagonal: 112 IS heptagonal (Hep(7) = 112)"
assert line(0) == "heptagonal: 0 is NOT heptagonal"
assert line(6) == "heptagonal: 6 is NOT heptagonal"

if "usage: heptagonal <n>" not in s:
    print("HEPTAGONAL TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("HEPTAGONAL TEST PASSED  (1/7/18/34/55/112 IS with exact k, 0/6/17 NOT vs "
      "python; bad->usage)")
PY
