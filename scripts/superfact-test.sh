#!/usr/bin/env bash
# Verify the `superfact <n>` command computes the SUPERFACTORIAL -- the product of
# the first n factorials 1!*2!*3!*...*n! -- in u64 with an overflow guard.
# sf(0)=1 (empty product), sf(1)=1, sf(2)=2, sf(3)=12, sf(4)=288, sf(5)=34560,
# sf(6)=24883200, sf(7)=125411328000, sf(8)=5056584744960000. DISTINCT from
# factorial (a single n!), doublefact (n!!), and primorial (product of primes):
# superfact multiplies whole factorials together, so it overflows u64 early
# (sf(8) fits, sf(9) overflows). Self-checking: python recomputes with
# math.factorial and asserts each value + the observed overflow boundary.
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
OUT="${TMPDIR:-/tmp}/superfact.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/superfact.png" \
    "superfact 0" "superfact 1" "superfact 2" "superfact 3" "superfact 4" \
    "superfact 5" "superfact 6" "superfact 7" "superfact 8" "superfact 9" \
    "factorial 4" "superfact zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "superfact:|usage: superfact|factorial:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import factorial
s = open(sys.argv[1], "rb").read().decode("latin1")

def sf(n):
    r = 1
    for k in range(1, n + 1):
        r *= factorial(k)
    return r

for n in range(0, 9):   # sf(0)..sf(8) all fit u64
    want = f"superfact: sf({n}) = {sf(n)}"
    if want not in s:
        print(f"SUPERFACT TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: sf(8) fits, sf(9) overflows.
M = 2**64 - 1
assert sf(8) <= M and sf(9) > M, "boundary assumption wrong"
if "superfact: sf(9) overflows u64" not in s:
    print("SUPERFACT TEST FAILED (missing sf(9) overflow line)"); sys.exit(1)

# Cross-check: superfact(4)=288 grows far faster than a single factorial(4)=24.
assert sf(4) == 288 and factorial(4) == 24
if "factorial: 4! = 24" not in s:
    print("SUPERFACT TEST FAILED (missing factorial 4 cross-check)"); sys.exit(1)

if "usage: superfact <n>" not in s:
    print("SUPERFACT TEST FAILED (missing usage line)"); sys.exit(1)

print("SUPERFACT TEST PASSED  (sf(0..8) exact vs python, sf(9) overflow-guarded, "
      "factorial(4)=24 cross-check, bad->usage)")
PY
