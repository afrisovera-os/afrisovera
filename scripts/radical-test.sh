#!/usr/bin/env bash
# Verify the `radical <n>` command computes rad(n): the product of the DISTINCT
# primes dividing n (rad(12)=2*3=6, rad(8)=2, rad(30)=30, rad(360)=30,
# rad(64)=2, rad(100)=10, rad(1)=1, rad(prime p)=p). Where omega COUNTS the
# distinct primes, radical MULTIPLIES them. n=0 -> usage. Self-checking: python
# multiplies the set of prime factors.
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
OUT="${TMPDIR:-/tmp}/radical.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/radical.png" \
    "echo warmup" "radical 8" "radical 12" "radical 60" "radical 100" \
    "radical 1" "radical 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "radical:|usage: radical" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def radical(n):
    r = 1
    x = n
    d = 2
    while d * d <= x:
        if x % d == 0:
            r *= d
            while x % d == 0:
                x //= d
        d += 1
    if x > 1:
        r *= x
    return r

def line(n):
    return f"radical: rad({n}) = {radical(n)}"

for n in [8, 12, 60, 100, 1]:
    want = line(n)
    if want not in s:
        print(f"RADICAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable values -- radical MULTIPLIES distinct primes (contrast omega's COUNT)
assert line(8) == "radical: rad(8) = 2"      # 2^3 -> just 2 (omega: 1)
assert line(60) == "radical: rad(60) = 30"    # 2^2*3*5 -> 2*3*5=30 (omega: 3)
assert line(100) == "radical: rad(100) = 10"  # 2^2*5^2 -> 2*5=10 (omega: 2)
assert line(1) == "radical: rad(1) = 1"       # empty product

# n=0 must hit the usage line (n>=1 guard), NOT a value
if "usage: radical <n>" not in s:
    print("RADICAL TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "radical: rad(0)" in s:
    print("RADICAL TEST FAILED (n=0 should NOT produce a value)"); sys.exit(1)

print("RADICAL TEST PASSED  (rad 8->2, 12->6, 60->30, 100->10, 1->1 vs python; radical "
      "multiplies distinct primes (omega counts them); n=0 -> usage)")
PY
