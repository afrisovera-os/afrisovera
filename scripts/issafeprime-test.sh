#!/usr/bin/env bash
# Verify the `issafeprime <n>` command classifies safe primes: n is prime AND (n-1)/2 is
# also prime. 5 ((5-1)/2=2) and 11 ((11-1)/2=5) IS; 13 ((13-1)/2=6 composite) and 4 (not
# prime) NOT. Distinct from istwinprime/emirp/iscircularprime. Self-checking: python
# replicates the primality + (n-1)/2 rule (with the IS branch's reported half-value).
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
OUT="${TMPDIR:-/tmp}/issafeprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/issafeprime.png" \
    "echo warmup" "issafeprime 5" "issafeprime 11" "issafeprime 13" "issafeprime 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "issafeprime:|usage: issafeprime" "$OUT" | sed 's/^/  /'

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

def message(n):  # mirror the kernel rule + IS-branch wording
    if isp(n) and isp((n - 1) // 2):
        return f"issafeprime: {n} IS a safe prime ((n-1)/2 = {(n - 1) // 2} is prime)"
    return f"issafeprime: {n} is NOT a safe prime"

for n in [5, 11, 13, 4]:
    want = message(n)
    if want not in s:
        print(f"ISSAFEPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable branches explicitly (derived above)
assert "issafeprime: 5 IS a safe prime ((n-1)/2 = 2 is prime)" in s
assert "issafeprime: 11 IS a safe prime ((n-1)/2 = 5 is prime)" in s
assert "issafeprime: 13 is NOT a safe prime" in s     # (13-1)/2 = 6 composite
assert "issafeprime: 4 is NOT a safe prime" in s      # not prime

print("ISSAFEPRIME TEST PASSED  (5 IS (2), 11 IS (5), 13 NOT (6), 4 NOT vs python "
      "prime + (n-1)/2 prime rule)")
PY
