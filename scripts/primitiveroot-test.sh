#!/usr/bin/env bash
# Verify `primitiveroot <p>` reports the SMALLEST primitive root modulo a prime p -- the least g whose
# multiplicative order mod p is p-1. Tests 7->3, 11->2, 13->2, 23->5, plus a composite (9) rejected.
# Self-checking: python computes the smallest primitive root by checking g^((p-1)/q) != 1 for each
# prime q | p-1.
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
OUT="${TMPDIR:-/tmp}/primitiveroot.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/primitiveroot.png" \
    "echo warmup" "primitiveroot 7" "primitiveroot 11" "primitiveroot 13" \
    "primitiveroot 23" "primitiveroot 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "primitiveroot:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def prime_factors(n):
    f, d = set(), 2
    while d * d <= n:
        while n % d == 0:
            f.add(d); n //= d
        d += 1
    if n > 1: f.add(n)
    return f

def smallest_primitive_root(p):      # DERIVED reference
    phi = p - 1
    qs = prime_factors(phi)
    g = 2
    while g < p:
        if all(pow(g, phi // q, p) != 1 for q in qs):
            return g
        g += 1
    return None

for p in (7, 11, 13, 23):
    g = smallest_primitive_root(p)
    want = f"primitiveroot: smallest primitive root mod {p} is {g}"
    if want not in s:
        print(f"PRIMITIVEROOT TEST FAILED (missing '{want}')"); sys.exit(1)

assert smallest_primitive_root(7) == 3
assert smallest_primitive_root(11) == 2
assert smallest_primitive_root(13) == 2
assert smallest_primitive_root(23) == 5
assert "primitiveroot: 9 is not prime (need a prime modulus)" in s
print("PRIMITIVEROOT TEST PASSED  (7->3, 11->2, 13->2, 23->5 vs python; composite 9 rejected)")
PY
