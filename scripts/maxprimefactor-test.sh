#!/usr/bin/env bash
# Verify `maxprimefactor <n>` gives the largest prime factor (A006530, gpf): 60->5, 17->17,
# 100->5, 91->13 (7*13), 2->2. n<=1 reported. Distinct from factor(all)/radical(product)/isprime.
# Self-checking: python trial-division greatest prime factor.
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
OUT="${TMPDIR:-/tmp}/maxprimefactor.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/maxprimefactor.png" \
    "echo warmup" "maxprimefactor 60" "maxprimefactor 17" "maxprimefactor 100" \
    "maxprimefactor 91" "maxprimefactor 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "maxprimefactor:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def gpf(n):                           # DERIVED reference
    largest = 1
    d = 2
    while d * d <= n:
        while n % d == 0:
            largest = d
            n //= d
        d += 1
    if n > 1:
        largest = n
    return largest

for n in [60, 17, 100, 91, 2]:
    w = f"maxprimefactor: {n} -> {gpf(n)}"
    if w not in s:
        print(f"MAXPRIMEFACTOR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: composite, prime (self), power+prime, two odd primes, smallest prime
assert "maxprimefactor: 60 -> 5" in s
assert "maxprimefactor: 17 -> 17" in s       # prime -> itself
assert "maxprimefactor: 100 -> 5" in s
assert "maxprimefactor: 91 -> 13" in s        # 7*13 -> 13
assert "maxprimefactor: 2 -> 2" in s

print("MAXPRIMEFACTOR TEST PASSED  (60->5, 17->17, 100->5, 91->13, 2->2 vs python greatest prime factor)")
PY
