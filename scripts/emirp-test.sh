#!/usr/bin/env bash
# Verify the `emirp <n>` command classifies emirps: a prime whose decimal digit-
# reversal is a DIFFERENT prime (13<->31, 79<->97). Palindromic primes (11, 101)
# are NOT emirps (reverse == n); composites are NOT. Distinct from ispalprime
# (prime that reads the same reversed). Self-checking: python replicates the exact
# three-branch message the kernel prints.
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
OUT="${TMPDIR:-/tmp}/emirp.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 13 (emirp), 11 (palindromic prime), 23 (prime but 32 composite), 12 (not prime).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/emirp.png" \
    "echo warmup" "emirp 13" "emirp 11" "emirp 23" "emirp 12" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "emirp:|usage: emirp" "$OUT" | sed 's/^/  /'

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
    r = int(str(n)[::-1])
    if is_prime(n) and r != n and is_prime(r):
        return f"emirp: {n} IS an emirp (reverse {r} is a distinct prime)"
    elif is_prime(n) and r == n:
        return f"emirp: {n} is NOT an emirp (palindromic prime)"
    else:
        return f"emirp: {n} is NOT an emirp"

for n in [13, 11, 23, 12]:
    want = message(n)
    if want not in s:
        print(f"EMIRP TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the four notable branches explicitly
assert "emirp: 13 IS an emirp (reverse 31 is a distinct prime)" in s
assert "emirp: 11 is NOT an emirp (palindromic prime)" in s   # reverse == n
assert "emirp: 23 is NOT an emirp" in s                        # 32 is composite
assert "emirp: 12 is NOT an emirp" in s                        # 12 not prime

print("EMIRP TEST PASSED  (13->emirp(rev 31), 11->palindromic prime, 23->no(32 composite), "
      "12->no(not prime) vs python; prime whose reverse is a distinct prime)")
PY
