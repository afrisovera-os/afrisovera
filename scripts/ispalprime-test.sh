#!/usr/bin/env bash
# Verify the `ispalprime <n>` command classifies palindromic primes: prime AND a
# decimal palindrome (2, 3, 5, 7, 11, 101, 131, 151, 313 IS; 13 is prime but 31 !=
# 13 NOT; 22 is a palindrome but not prime NOT). Combines a trial-division
# primality test with a digit-reverse palindrome check (u128 reverse, no overflow).
# Distinct from isprime (primality only) and revnum (reverse + palindrome only).
# Self-checking: python isprime(n) and str(n)==str(n)[::-1].
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
OUT="${TMPDIR:-/tmp}/ispalprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 13 pins prime-but-not-palindrome; 22 pins palindrome-but-not-prime.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispalprime.png" \
    "echo warmup" "ispalprime 313" "ispalprime 13" "ispalprime 22" "ispalprime 11" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispalprime:|usage: ispalprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isprime(k):
    if k < 2: return False
    i = 2
    while i * i <= k:
        if k % i == 0: return False
        i += 1
    return True

def palindrome(n):
    return str(n) == str(n)[::-1]

# build the exact expected line for each case (three shapes)
for n in [313, 13, 22, 11]:
    p, pal = isprime(n), palindrome(n)
    if p and pal:
        want = f"ispalprime: {n} IS a palindromic prime"
    elif not p:
        want = f"ispalprime: {n} is NOT (not prime)"
    else:
        want = f"ispalprime: {n} is NOT (prime but not a palindrome)"
    if want not in s:
        print(f"ISPALPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "ispalprime: 313 IS a palindromic prime" in s
assert "ispalprime: 13 is NOT (prime but not a palindrome)" in s   # prime, not palindrome
assert "ispalprime: 22 is NOT (not prime)" in s                    # palindrome, not prime
assert "ispalprime: 11 IS a palindromic prime" in s

print("ISPALPRIME TEST PASSED  (313/11 IS, 13 prime-not-palindrome, 22 palindrome-not-"
      "prime vs python; distinct from isprime/revnum)")
PY
