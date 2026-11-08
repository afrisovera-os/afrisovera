#!/usr/bin/env bash
# Verify the `iszuckerman <n>` command classifies Zuckerman numbers: a positive
# integer divisible by the PRODUCT of its own digits (24 = product 2*4=8, 24/8=3
# IS; 13 product 3 remainder 1 NOT; any number with a 0 digit has product 0 and is
# excluded). Distinct from harshad (divisible by digit SUM) and digitprod (which
# only PRINTS the product). Self-checking: python p=prod(digits); p!=0 and n%p==0.
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
OUT="${TMPDIR:-/tmp}/iszuckerman.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "iszuckerman 10" pins the 0-digit -> product-0 exclusion.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iszuckerman.png" \
    "echo warmup" "iszuckerman 24" "iszuckerman 13" "iszuckerman 10" "iszuckerman 128" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iszuckerman:|usage: iszuckerman" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import prod
s = open(sys.argv[1], "rb").read().decode("latin1")

def dp(n):
    return prod(int(d) for d in str(n))

# build the exact expected line for each case (three shapes: zero-digit, IS, NOT)
for n in [24, 13, 10, 128]:
    p = dp(n)
    if p == 0:
        want = f"iszuckerman: {n} is NOT a Zuckerman number (digit product 0 -- has a 0 digit)"
    elif n % p == 0:
        want = f"iszuckerman: {n} IS a Zuckerman number (digit product {p} divides {n})"
    else:
        want = f"iszuckerman: {n} is NOT a Zuckerman number (digit product {p}, remainder {n % p})"
    if want not in s:
        print(f"ISZUCKERMAN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "iszuckerman: 24 IS a Zuckerman number (digit product 8 divides 24)" in s
assert "iszuckerman: 13 is NOT a Zuckerman number (digit product 3, remainder 1)" in s
assert "iszuckerman: 10 is NOT a Zuckerman number (digit product 0 -- has a 0 digit)" in s
assert "iszuckerman: 128 IS a Zuckerman number (digit product 16 divides 128)" in s

print("ISZUCKERMAN TEST PASSED  (24/128 IS, 13 NOT (rem 1), 10 excluded (0 digit) via "
      "digit-product divisibility vs python; distinct from harshad's digit-sum test)")
PY
