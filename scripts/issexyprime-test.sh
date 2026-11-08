#!/usr/bin/env bash
# Verify `issexyprime <n>` classifies sexy primes: n prime AND n-6 or n+6 prime.
# 5 IS (pairs 11), 11 IS (pairs 5 and 17); 3 is an isolated prime (3+6=9 composite) NOT;
# 9 is not prime NOT. The gap-6 analogue of istwinprime/iscousinprime. Self-checking:
# python replicates the primality + n+-6 rule incl. the exact "pairs with" wording.
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
OUT="${TMPDIR:-/tmp}/issexyprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/issexyprime.png" \
    "echo warmup" "issexyprime 5" "issexyprime 11" "issexyprime 3" "issexyprime 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "issexyprime:|usage: issexyprime" "$OUT" | sed 's/^/  /'

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
        return f"issexyprime: {n} is NOT a sexy prime (not prime)"
    lo = n >= 6 and isp(n - 6)
    hi = isp(n + 6)
    if lo and hi:
        return f"issexyprime: {n} IS a sexy prime (pairs with {n - 6} and {n + 6})"
    if lo:
        return f"issexyprime: {n} IS a sexy prime (pairs with {n - 6})"
    if hi:
        return f"issexyprime: {n} IS a sexy prime (pairs with {n + 6})"
    return f"issexyprime: {n} is NOT a sexy prime (isolated prime)"

for n in [5, 11, 3, 9]:
    want = message(n)
    if want not in s:
        print(f"ISSEXYPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches explicitly (derived above)
assert "issexyprime: 5 IS a sexy prime (pairs with 11)" in s
assert "issexyprime: 11 IS a sexy prime (pairs with 5 and 17)" in s
assert "issexyprime: 3 is NOT a sexy prime (isolated prime)" in s
assert "issexyprime: 9 is NOT a sexy prime (not prime)" in s

print("ISSEXYPRIME TEST PASSED  (5 IS (11), 11 IS (5,17), 3 isolated NOT, 9 not-prime NOT "
      "vs python prime + n+-6 rule)")
PY
