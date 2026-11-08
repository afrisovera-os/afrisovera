#!/usr/bin/env bash
# Verify the `nextprime <n>` command returns the SMALLEST PRIME STRICTLY GREATER
# THAN n (the successor-prime function). e.g. 10->11, 13->17, 0->2, 100->101.
# DISTINCT from nthprime (nth prime by index), primepi (count of primes <= n),
# isprime (a primality test), and primes (list up to a bound). Self-checking:
# python recomputes the next prime and matches each output line.
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
OUT="${TMPDIR:-/tmp}/nextprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextprime.png" \
    "echo warmup" "nextprime 0" "nextprime 1" "nextprime 2" "nextprime 10" \
    "nextprime 13" "nextprime 20" "nextprime 100" "nextprime 1000" \
    "nextprime zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextprime:|usage: nextprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(x):
    return x >= 2 and all(x % d for d in range(2, int(x ** 0.5) + 1))

def nextp(n):
    c = n + 1
    while not isp(c):
        c += 1
    return c

for n in [0, 1, 2, 10, 13, 20, 100, 1000]:
    want = f"nextprime: next prime after {n} is {nextp(n)}"
    if want not in s:
        print(f"NEXTPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin a few exact successors + the strictly-greater property (2 -> 3, not 2)
assert nextp(0) == 2 and nextp(1) == 2 and nextp(2) == 3
assert nextp(13) == 17 and nextp(100) == 101 and nextp(1000) == 1009

if "usage: nextprime <n>" not in s:
    print("NEXTPRIME TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("NEXTPRIME TEST PASSED  (0/1/2/10/13/20/100/1000 successors exact vs python, "
      "strictly greater, bad->usage)")
PY
