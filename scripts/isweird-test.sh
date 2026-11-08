#!/usr/bin/env bash
# Verify `isweird <n>` classifies whether n is a WEIRD number: ABUNDANT (sum
# of proper divisors > n) yet NOT SEMIPERFECT (no subset of those divisors
# sums to exactly n). 70 is the smallest weird number -- divisors 1,2,5,7,
# 10,14,35 sum to 74 (abundant) but no subset hits 70 exactly. 12 and 20 are
# abundant BUT semiperfect (so NOT weird); 8 is deficient (so NOT weird
# regardless of semiperfect-ness). Self-checking: python re-derives via an
# independent proper-divisor-sum + subset-sum DP.
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
OUT="${TMPDIR:-/tmp}/isweird.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isweird.png" \
    "echo warmup" "isweird 70" "isweird 12" "isweird 20" \
    "isweird 8" "isweird 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isweird:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def proper_divisors(n):
    divs = []
    d = 1
    while d * d <= n:
        if n % d == 0:
            if d != n:
                divs.append(d)
            e = n // d
            if e != d and e != n:
                divs.append(e)
        d += 1
    return divs

def is_semiperfect(n, divs):
    reachable = {0}
    for d in divs:
        reachable |= {t + d for t in reachable if t + d <= n}
    return n in reachable

def is_weird(n):
    divs = proper_divisors(n)
    dsum = sum(divs)
    abundant = dsum > n
    return abundant and not is_semiperfect(n, divs), dsum

def line(n):
    weird, dsum = is_weird(n)
    if weird:
        return f"isweird: {n} IS weird -- abundant (divisor sum {dsum}) but not semiperfect"
    return f"isweird: {n} is NOT weird"

for n in [70, 12, 20, 8]:
    want = line(n)
    if want not in s:
        print(f"ISWEIRD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(70) == "isweird: 70 IS weird -- abundant (divisor sum 74) but not semiperfect"
assert line(12) == "isweird: 12 is NOT weird"  # abundant (sigma-n=4) but semiperfect (2+4+6)
assert line(20) == "isweird: 20 is NOT weird"  # abundant (sigma-n=2) but semiperfect (1+4+5+10)
assert line(8) == "isweird: 8 is NOT weird"    # deficient, can't be weird regardless of subset-sum

# n=0 must hit the explicit zero branch, not a verdict
if "isweird: 0 is NOT weird (needs n >= 1)" not in s:
    print("ISWEIRD TEST FAILED (n=0 should report the needs-n>=1 message)"); sys.exit(1)

print("ISWEIRD TEST PASSED  (70 weird, 12 & 20 abundant-but-semiperfect, "
      "8 deficient, n=0 -> guard) vs python")
PY
