#!/usr/bin/env bash
# Verify the `primorial` command computes the product of the first n primes
# (2*3*5*7*11*...) in u64 with an overflow guard. primorial(0)=1 (empty product),
# 2,6,30,210,2310,30030,... DISTINCT from factorial (product of first n INTEGERS)
# and primepi/nthprime (count/return primes, not multiply). Self-checking: python
# recomputes the product of the first n primes; the u64 boundary is primorial(15)
# fits / primorial(16) overflows.
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
OUT="${TMPDIR:-/tmp}/primorial.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/primorial.png" \
    "primorial 0" "primorial 1" "primorial 2" "primorial 3" "primorial 4" \
    "primorial 5" "primorial 6" "primorial 10" "primorial 15" "primorial 16" \
    "primorial zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "primorial:|usage: primorial" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isprime(x):
    if x < 2:
        return False
    d = 2
    while d * d <= x:
        if x % d == 0:
            return False
        d += 1
    return True

def primorial(n):
    r, found, c = 1, 0, 2
    while found < n:
        if isprime(c):
            r *= c
            found += 1
        c += 1
    return r

for n in [0, 1, 2, 3, 4, 5, 6, 10, 15]:
    want = f"primorial: primorial({n}) = {primorial(n)}"
    if want not in s:
        print(f"PRIMORIAL TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: primorial(15) fits, primorial(16) overflows.
M = 2**64 - 1
assert primorial(15) <= M and primorial(16) > M, "boundary assumption wrong"
if "primorial: primorial(16) overflows u64" not in s:
    print("PRIMORIAL TEST FAILED (missing primorial(16) overflow line)"); sys.exit(1)
if "usage: primorial <n>" not in s:
    print("PRIMORIAL TEST FAILED (missing usage line)"); sys.exit(1)

print("PRIMORIAL TEST PASSED  (primorial(0..10) & (15) exact vs python, (16) overflow-guarded, bad->usage)")
PY
