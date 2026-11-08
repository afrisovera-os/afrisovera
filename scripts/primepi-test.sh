#!/usr/bin/env bash
# Verify the `primepi` command computes the prime-counting function pi(n) -- how
# many primes are <= n -- via trial division. Distinct from primes (lists them),
# nthprime (the nth), isprime (predicate). The harness recomputes pi(n) the same
# way in python, so it is self-checking: pi(0)=0, pi(1)=0, pi(2)=1, pi(10)=4,
# pi(100)=25, pi(1000)=168; n>100000 -> too-large; bad arg -> usage. Inputs are
# kept small (<=1000) so the suite stays fast.
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
OUT="${TMPDIR:-/tmp}/primepi.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/primepi.png" \
    "primepi 0" "primepi 1" "primepi 2" "primepi 10" "primepi 100" "primepi 1000" \
    "primepi 200000" "primepi abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "primepi:|usage: primepi" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(x):
    if x < 2:
        return False
    if x % 2 == 0:
        return x == 2
    d = 3
    while d * d <= x:
        if x % d == 0:
            return False
        d += 2
    return True

def pi(n):
    return sum(1 for k in range(2, n + 1) if isp(k))

for n in [0, 1, 2, 10, 100, 1000]:
    want = f"primepi: pi({n}) = {pi(n)}"
    if want not in s:
        print(f"PRIMEPI TEST FAILED (missing '{want}')"); sys.exit(1)

if "primepi: n too large (max 100000)" not in s:
    print("PRIMEPI TEST FAILED (missing too-large line)"); sys.exit(1)
if "usage: primepi <n>" not in s:
    print("PRIMEPI TEST FAILED (missing usage line)"); sys.exit(1)

print("PRIMEPI TEST PASSED  (pi(0)=0, pi(2)=1, pi(10)=4, pi(100)=25, pi(1000)=168; too-large + usage handled)")
PY
