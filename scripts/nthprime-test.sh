#!/usr/bin/env bash
# Verify the `nthprime` command returns the nth prime (1-indexed): #6=13, #100=541,
# #1000=7919; n=0 falls through to usage. Distinct from primes (sieve up to N),
# isprime (predicate), factor. Test recomputes the nth prime in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4980,server,nowait \
    -qmp tcp:127.0.0.1:4981,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/nthprime.out"
python3 scripts/shell-demo.py 4980 4981 "${TMPDIR:-/tmp}/nthprime.png" \
    "nthprime 6" "nthprime 100" "nthprime 1000" "nthprime 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthprime:|usage: nthprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k == 2: return True
    if k % 2 == 0: return False
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

def nth(n):
    c, cand = 0, 1
    while c < n:
        cand += 1
        if isp(cand): c += 1
    return cand

for n in [6, 100, 1000]:
    want = f"nthprime: prime #{n} = {nth(n)}"
    if want not in s:
        print(f"NTHPRIME TEST FAILED (expected '{want}')"); sys.exit(1)

if "usage: nthprime <n>" not in s:
    print("NTHPRIME TEST FAILED (missing usage line for 0)"); sys.exit(1)

print("NTHPRIME TEST PASSED  (#6=13, #100=541, #1000=7919; 0 rejected)")
PY
