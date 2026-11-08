#!/usr/bin/env bash
# Verify `iscousinprime <n>` classifies cousin primes: n prime AND n-4 or n+4 prime.
# 7 IS (pairs 3 and 11), 13 IS (pairs 17); 5 is an isolated prime (1,9 both composite) NOT;
# 9 is not prime NOT. The gap-4 analogue of istwinprime. Self-checking: python replicates
# the primality + n+-4 rule incl. the exact "pairs with" wording.
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
OUT="${TMPDIR:-/tmp}/iscousinprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iscousinprime.png" \
    "echo warmup" "iscousinprime 7" "iscousinprime 13" "iscousinprime 5" "iscousinprime 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iscousinprime:|usage: iscousinprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(m):
    if m < 2: return False
    if m < 4: return True
    if m % 2 == 0: return False
    d = 3
    while d * d <= m:
        if m % d == 0: return False
        d += 2
    return True

def message(n):  # mirror the kernel wording exactly
    if not isp(n):
        return f"iscousinprime: {n} is NOT a cousin prime (not prime)"
    lo = n >= 4 and isp(n - 4)
    hi = isp(n + 4)
    if lo and hi:
        return f"iscousinprime: {n} IS a cousin prime (pairs with {n - 4} and {n + 4})"
    if lo:
        return f"iscousinprime: {n} IS a cousin prime (pairs with {n - 4})"
    if hi:
        return f"iscousinprime: {n} IS a cousin prime (pairs with {n + 4})"
    return f"iscousinprime: {n} is NOT a cousin prime (isolated prime)"

for n in [7, 13, 5, 9]:
    want = message(n)
    if want not in s:
        print(f"ISCOUSINPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches explicitly (derived above)
assert "iscousinprime: 7 IS a cousin prime (pairs with 3 and 11)" in s
assert "iscousinprime: 13 IS a cousin prime (pairs with 17)" in s
assert "iscousinprime: 5 is NOT a cousin prime (isolated prime)" in s
assert "iscousinprime: 9 is NOT a cousin prime (not prime)" in s

print("ISCOUSINPRIME TEST PASSED  (7 IS (3,11), 13 IS (17), 5 isolated NOT, 9 not-prime NOT "
      "vs python prime + n+-4 rule)")
PY
