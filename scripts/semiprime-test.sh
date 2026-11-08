#!/usr/bin/env bash
# Verify the `semiprime <n>` command classifies whether n (n>=2) is a SEMIPRIME:
# a product of exactly two primes counted WITH multiplicity (4=2*2, 6, 9=3*3,
# 10, 14, 15, 21, 25, 49 yes; 30=2*3*5, 7 (prime), 12=2^2*3 no; n<2 -> usage).
# Self-checking: python re-derives the prime-factor multiplicity total.
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
OUT="${TMPDIR:-/tmp}/semiprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/semiprime.png" \
    "echo warmup" "semiprime 4" "semiprime 6" "semiprime 9" "semiprime 25" \
    "semiprime 49" "semiprime 30" "semiprime 7" "semiprime 12" \
    "semiprime 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "semiprime:|usage: semiprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def omega_with_mult(n):
    cnt = 0
    x = n
    d = 2
    while d * d <= x:
        while x % d == 0:
            x //= d
            cnt += 1
        d += 1
    if x > 1:
        cnt += 1
    return cnt

def line(n):
    return f"semiprime: {n} IS semiprime" if omega_with_mult(n) == 2 else f"semiprime: {n} is NOT semiprime"

# semiprime: 4,6,9,25,49 ; not: 30,7,12
for n in [4, 6, 9, 25, 49, 30, 7, 12]:
    want = line(n)
    if want not in s:
        print(f"SEMIPRIME TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable values -- 4=2*2 counts multiplicity (IS semiprime, contrast squarefree)
assert line(4) == "semiprime: 4 IS semiprime"
assert line(49) == "semiprime: 49 IS semiprime"    # 7*7
assert line(30) == "semiprime: 30 is NOT semiprime"  # 2*3*5 (three primes)
assert line(7) == "semiprime: 7 is NOT semiprime"    # prime

# n=1 must hit the usage line (n>=2 guard), NOT a verdict
if "usage: semiprime <n>" not in s:
    print("SEMIPRIME TEST FAILED (n=1 should report usage)"); sys.exit(1)
if "semiprime: 1 " in s:
    print("SEMIPRIME TEST FAILED (n=1 should NOT produce a verdict)"); sys.exit(1)

print("SEMIPRIME TEST PASSED  (4/6/9/25/49 yes, 30/7/12 no vs python; 4=2*2 counts "
      "multiplicity; n=1 -> usage)")
PY
