#!/usr/bin/env bash
# Verify the `bigomega <n>` command counts the prime factors of n WITH
# multiplicity (Omega(n)): 12=2^2*3 -> 3, 30=2*3*5 -> 3, 8=2^3 -> 3, primes ->
# 1, 1 -> 0 (empty product), 64=2^6 -> 6. Generalizes semiprime (Omega==2).
# n=0 -> usage. Self-checking: python re-derives the multiplicity total.
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
OUT="${TMPDIR:-/tmp}/bigomega.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bigomega.png" \
    "echo warmup" "bigomega 1" "bigomega 7" "bigomega 12" "bigomega 30" \
    "bigomega 8" "bigomega 64" "bigomega 100" "bigomega 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bigomega:|usage: bigomega" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def bigomega(n):
    c = 0
    x = n
    d = 2
    while d * d <= x:
        while x % d == 0:
            x //= d
            c += 1
        d += 1
    if x > 1:
        c += 1
    return c

def line(n):
    return f"bigomega: {n} has {bigomega(n)} prime factors (with multiplicity)"

for n in [1, 7, 12, 30, 8, 64, 100]:
    want = line(n)
    if want not in s:
        print(f"BIGOMEGA TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable values -- multiplicity is counted (12=2^2*3 -> 3, not 2 distinct)
assert line(1) == "bigomega: 1 has 0 prime factors (with multiplicity)"   # empty product
assert line(7) == "bigomega: 7 has 1 prime factors (with multiplicity)"   # prime
assert line(12) == "bigomega: 12 has 3 prime factors (with multiplicity)" # 2*2*3
assert line(64) == "bigomega: 64 has 6 prime factors (with multiplicity)" # 2^6

# n=0 must hit the usage line (n>=1 guard), NOT a verdict
if "usage: bigomega <n>" not in s:
    print("BIGOMEGA TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "bigomega: 0 " in s:
    print("BIGOMEGA TEST FAILED (n=0 should NOT produce a count)"); sys.exit(1)

print("BIGOMEGA TEST PASSED  (1->0, 7->1, 12/30/8->3, 64->6, 100->4 vs python; "
      "multiplicity counted; n=0 -> usage)")
PY
