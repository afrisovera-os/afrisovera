#!/usr/bin/env bash
# Verify the `sphenic <n>` command classifies whether n (n>=2) is a SPHENIC
# number: a product of exactly THREE DISTINCT primes, each to the first power
# (30=2*3*5, 42, 66, 70, 78, 105=3*5*7, 110 yes; NOT 8=2^3, 12=2^2*3,
# 60=2^2*3*5 [3 distinct but 2 repeats], primes, semiprimes). n<2 -> usage.
# Extends the squarefree/semiprime family. Self-checking: python re-derives the
# factorization and checks 3 distinct primes all with exponent 1.
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
OUT="${TMPDIR:-/tmp}/sphenic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sphenic.png" \
    "echo warmup" "sphenic 30" "sphenic 42" "sphenic 105" "sphenic 110" \
    "sphenic 60" "sphenic 8" "sphenic 12" "sphenic 7" "sphenic 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sphenic:|usage: sphenic" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def factor(n):
    f = {}
    x = n
    d = 2
    while d * d <= x:
        while x % d == 0:
            f[d] = f.get(d, 0) + 1
            x //= d
        d += 1
    if x > 1:
        f[x] = f.get(x, 0) + 1
    return f

def sphenic(n):
    f = factor(n)
    return len(f) == 3 and all(e == 1 for e in f.values())

def line(n):
    return f"sphenic: {n} IS sphenic" if sphenic(n) else f"sphenic: {n} is NOT sphenic"

# sphenic: 30,42,105,110 ; not: 60,8,12,7
for n in [30, 42, 105, 110, 60, 8, 12, 7]:
    want = line(n)
    if want not in s:
        print(f"SPHENIC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the tricky cases explicitly
assert line(30) == "sphenic: 30 IS sphenic"       # 2*3*5
assert line(105) == "sphenic: 105 IS sphenic"      # 3*5*7 (no factor of 2)
assert line(60) == "sphenic: 60 is NOT sphenic"    # 2^2*3*5: 3 distinct but 2 repeats
assert line(8) == "sphenic: 8 is NOT sphenic"      # 2^3: only 1 distinct prime

# n=1 must hit the usage line (n>=2 guard), NOT a verdict
if "usage: sphenic <n>" not in s:
    print("SPHENIC TEST FAILED (n=1 should report usage)"); sys.exit(1)
if "sphenic: 1 " in s:
    print("SPHENIC TEST FAILED (n=1 should NOT produce a verdict)"); sys.exit(1)

print("SPHENIC TEST PASSED  (30/42/105/110 yes, 60/8/12/7 no vs python; 60=2^2*3*5 "
      "correctly NOT despite 3 distinct primes; n=1 -> usage)")
PY
