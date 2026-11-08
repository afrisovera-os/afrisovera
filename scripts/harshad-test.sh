#!/usr/bin/env bash
# Verify the `harshad <n>` command classifies whether n is a HARSHAD (Niven)
# number: a positive integer divisible by its own digit sum. e.g. 18 (digit sum
# 9, 18/9=2) IS; 11 (digit sum 2, remainder 1) is NOT; 0 is undefined (digit sum
# 0). DISTINCT from digitsum (just the sum), armstrong (digit^k sum), happy (sum
# of digit squares), perfect (divisor sum), divisors (lists divisors).
# Self-checking: python recomputes the digit sum, verdict, quotient/remainder.
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
OUT="${TMPDIR:-/tmp}/harshad.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/harshad.png" \
    "echo warmup" "harshad 18" "harshad 12" "harshad 21" "harshad 100" \
    "harshad 11" "harshad 19" "harshad 0" "harshad zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "harshad:|usage: harshad" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ds(n):
    return sum(int(c) for c in str(n))

def line(n):
    d = ds(n)
    if d == 0:
        return "harshad: 0 has digit sum 0 (undefined)"
    if n % d == 0:
        return f"harshad: {n} IS a Harshad number (digit sum {d} divides {n}: {n}/{d} = {n // d})"
    return f"harshad: {n} is NOT a Harshad number (digit sum {d}, remainder {n % d})"

# Harshad: 18, 12, 21, 100 ; not: 11, 19 ; 0 undefined
for n in [18, 12, 21, 100, 11, 19, 0]:
    want = line(n)
    if want not in s:
        print(f"HARSHAD TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the exact digit-sum arithmetic + verdicts
assert line(18) == "harshad: 18 IS a Harshad number (digit sum 9 divides 18: 18/9 = 2)"
assert line(11) == "harshad: 11 is NOT a Harshad number (digit sum 2, remainder 1)"
assert line(0)  == "harshad: 0 has digit sum 0 (undefined)"

if "usage: harshad <n>" not in s:
    print("HARSHAD TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("HARSHAD TEST PASSED  (18/12/21/100 IS, 11/19 NOT, 0 undefined; exact digit "
      "sums/quotients/remainders vs python; bad->usage)")
PY
