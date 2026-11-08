#!/usr/bin/env bash
# Verify `carmichael <n>` computes the Carmichael function lambda(n): lambda(1)=1, lambda(15)=4
# (vs phi=8), lambda(16)=4, lambda(12)=2, lambda(21)=6. lcm over prime-power factors, special
# 2^k rule. Companion of totient/order/modinv. Self-checking: python computes lambda(n)
# INDEPENDENTLY (factorize + lcm of phi(p^k) / 2^k rule).
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
OUT="${TMPDIR:-/tmp}/carmichael.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/carmichael.png" \
    "echo warmup" "carmichael 1" "carmichael 15" "carmichael 16" \
    "carmichael 12" "carmichael 21" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "carmichael:|usage: carmichael" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import gcd
s = open(sys.argv[1], "rb").read().decode("latin1")

def lcm(a, b):
    return a // gcd(a, b) * b if a and b else 0

def factorize(n):
    f, d = {}, 2
    while d * d <= n:
        while n % d == 0:
            f[d] = f.get(d, 0) + 1; n //= d
        d += 1
    if n > 1:
        f[n] = f.get(n, 0) + 1
    return f

def carmichael(n):                       # INDEPENDENT reference
    if n == 1:
        return 1
    res = 1
    for p, k in factorize(n).items():
        pk = p ** k
        if p == 2:
            lam = 1 if k == 1 else 2 if k == 2 else pk // 4
        else:
            lam = pk // p * (p - 1)
        res = lcm(res, lam)
    return res

for n in [1, 15, 16, 12, 21]:
    w = f"carmichael: lambda({n}) = {carmichael(n)}"
    if w not in s:
        print(f"CARMICHAEL TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: trivial, composite (< phi), power of 2, even composite, odd product
assert "carmichael: lambda(1) = 1" in s
assert "carmichael: lambda(15) = 4" in s     # phi(15)=8, so lambda != phi
assert "carmichael: lambda(16) = 4" in s     # 2^(4-2)
assert "carmichael: lambda(12) = 2" in s
assert "carmichael: lambda(21) = 6" in s     # lcm(lambda 3, lambda 7)=lcm(2,6)

print("CARMICHAEL TEST PASSED  (1->1, 15->4, 16->4, 12->2, 21->6 "
      "vs python lcm over prime-power factors)")
PY
