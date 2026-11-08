#!/usr/bin/env bash
# Verify the `triangular <n>` command classifies whether n is a TRIANGULAR number
# (n = k*(k+1)/2), reporting the index k. A number is triangular iff 8n+1 is a
# perfect square; then k = (sqrt(8n+1)-1)/2. Triangular: 0,1,3,6,10,15,21,...
# DISTINCT from sumto (which COMPUTES the nth triangular) and the digit-based
# classifiers -- triangular tests figurate-number MEMBERSHIP. Self-checking:
# python recomputes the verdict + index via math.isqrt.
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
OUT="${TMPDIR:-/tmp}/triangular.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/triangular.png" \
    "echo warmup" "triangular 0" "triangular 1" "triangular 3" "triangular 6" \
    "triangular 10" "triangular 55" "triangular 2" "triangular 11" \
    "triangular" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "triangular:|usage: triangular" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    r = math.isqrt(8 * n + 1)
    if r * r == 8 * n + 1:
        k = (r - 1) // 2
        return f"triangular: {n} IS triangular (T({k}) = {k}*({k}+1)/2)"
    return f"triangular: {n} is NOT triangular"

# triangular: 0,1,3,6,10,55 (with indices) ; not: 2, 11
for n in [0, 1, 3, 6, 10, 55, 2, 11]:
    want = line(n)
    if want not in s:
        print(f"TRIANGULAR TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the perfect-square membership property
assert line(10) == "triangular: 10 IS triangular (T(4) = 4*(4+1)/2)"
assert line(55) == "triangular: 55 IS triangular (T(10) = 10*(10+1)/2)"
assert line(11) == "triangular: 11 is NOT triangular"

if "usage: triangular <n>" not in s:
    print("TRIANGULAR TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("TRIANGULAR TEST PASSED  (0/1/3/6/10/55 IS with exact k, 2/11 NOT vs python; "
      "bad->usage)")
PY
