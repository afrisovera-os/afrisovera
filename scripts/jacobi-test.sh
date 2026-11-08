#!/usr/bin/env bash
# Verify `jacobi <a> <n>` computes the Jacobi symbol (a/n) for odd n via quadratic reciprocity:
# (2/3)=-1, (2/7)=1, (3/5)=-1, (5/21)=1, (6/9)=0 (gcd>1). Result is -1/0/1. Even n -> usage.
# Self-checking: python computes the SAME symbol INDEPENDENTLY via Euler's criterion on each
# prime factor (product of Legendre symbols) -- not the kernel's reciprocity algorithm.
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
OUT="${TMPDIR:-/tmp}/jacobi.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/jacobi.png" \
    "echo warmup" "jacobi 2 3" "jacobi 2 7" "jacobi 3 5" \
    "jacobi 5 21" "jacobi 6 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "jacobi:|usage: jacobi" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def factorize(n):
    f, d = {}, 2
    while d * d <= n:
        while n % d == 0:
            f[d] = f.get(d, 0) + 1; n //= d
        d += 1
    if n > 1:
        f[n] = f.get(n, 0) + 1
    return f

def legendre(a, p):                 # Euler's criterion (INDEPENDENT of reciprocity)
    r = pow(a % p, (p - 1) // 2, p)
    return -1 if r == p - 1 else r  # r is 0, 1, or p-1

def jacobi(a, n):
    res = 1
    for p, k in factorize(n).items():
        res *= legendre(a, p) ** k
    return res

for a, n in [(2, 3), (2, 7), (3, 5), (5, 21), (6, 9)]:
    want = f"jacobi: ({a}/{n}) = {jacobi(a, n)}"   # DERIVED via Euler per prime factor
    if want not in s:
        print(f"JACOBI TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: -1, +1, -1, composite +1, zero (gcd>1)
assert "jacobi: (2/3) = -1" in s
assert "jacobi: (2/7) = 1" in s
assert "jacobi: (3/5) = -1" in s
assert "jacobi: (5/21) = 1" in s
assert "jacobi: (6/9) = 0" in s

print("JACOBI TEST PASSED  ((2/3)=-1, (2/7)=1, (3/5)=-1, (5/21)=1, (6/9)=0 "
      "vs python Euler-criterion product over prime factors)")
PY
