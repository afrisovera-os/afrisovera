#!/usr/bin/env bash
# Verify the `istwinprime <n>` command classifies twin primes: n is prime AND at
# least one of n-2 or n+2 is also prime (5 pairs with 3 and 7; 3 pairs with 5; the
# isolated prime 23 is NOT; 9 is not prime). Distinct from isprime/emirp/ispalprime.
# Self-checking: python replicates the exact branch message the kernel prints.
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
OUT="${TMPDIR:-/tmp}/istwinprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 5 (both sides), 3 (only n+2), 23 (isolated prime), 9 (not prime).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/istwinprime.png" \
    "echo warmup" "istwinprime 5" "istwinprime 3" "istwinprime 23" "istwinprime 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "istwinprime:|usage: istwinprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_prime(x):
    if x < 2: return False
    if x < 4: return True
    if x % 2 == 0: return False
    d = 3
    while d <= x // d:
        if x % d == 0: return False
        d += 2
    return True

def message(n):
    if not is_prime(n):
        return f"istwinprime: {n} is NOT a twin prime (not prime)"
    lo = n >= 2 and is_prime(n - 2)
    hi = is_prime(n + 2)
    if lo and hi:
        return f"istwinprime: {n} IS a twin prime (pairs with {n-2} and {n+2})"
    if lo:
        return f"istwinprime: {n} IS a twin prime (pairs with {n-2})"
    if hi:
        return f"istwinprime: {n} IS a twin prime (pairs with {n+2})"
    return f"istwinprime: {n} is NOT a twin prime (isolated prime)"

for n in [5, 3, 23, 9]:
    want = message(n)
    if want not in s:
        print(f"ISTWINPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the four notable branches explicitly
assert "istwinprime: 5 IS a twin prime (pairs with 3 and 7)" in s
assert "istwinprime: 3 IS a twin prime (pairs with 5)" in s
assert "istwinprime: 23 is NOT a twin prime (isolated prime)" in s
assert "istwinprime: 9 is NOT a twin prime (not prime)" in s

print("ISTWINPRIME TEST PASSED  (5->twin(3 and 7), 3->twin(5), 23->isolated prime, 9->not prime "
      "vs python; prime with n-2 or n+2 also prime)")
PY
