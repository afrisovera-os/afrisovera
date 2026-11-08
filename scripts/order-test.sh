#!/usr/bin/env bash
# Verify `order <a> <n>` computes the multiplicative order of a mod n (least k with a^k=1 mod
# n): ord(2 mod 7)=3, ord(3 mod 7)=6, ord(2 mod 5)=4, ord(4 mod 15)=2, and "no order" when
# gcd(a,n)>1 (2 mod 4 -> gcd 2). Companion of modinv/jacobi. Self-checking: python iterates
# a^k mod n (and math.gcd for the no-order branch) INDEPENDENTLY.
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
OUT="${TMPDIR:-/tmp}/order.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/order.png" \
    "echo warmup" "order 2 7" "order 3 7" "order 2 5" \
    "order 4 15" "order 2 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "order:|usage: order" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(a, n):
    g = math.gcd(a % n, n)
    if g != 1:
        return f"order: no order (gcd({a}, {n}) = {g})"
    x, k = a % n, 1                     # iterate a^k mod n INDEPENDENTLY
    while x != 1:
        x = (x * a) % n; k += 1
    return f"order: ord({a} mod {n}) = {k}"

for a, n in [(2, 7), (3, 7), (2, 5), (4, 15), (2, 4)]:
    w = line(a, n)
    if w not in s:
        print(f"ORDER TEST FAILED (a={a} n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: order 3, full order 6, order 4, small composite modulus, no-order
assert "order: ord(2 mod 7) = 3" in s
assert "order: ord(3 mod 7) = 6" in s
assert "order: ord(2 mod 5) = 4" in s
assert "order: ord(4 mod 15) = 2" in s
assert "order: no order (gcd(2, 4) = 2)" in s

print("ORDER TEST PASSED  (2/7->3, 3/7->6, 2/5->4, 4/15->2, 2/4->no order "
      "vs python a^k mod n iteration)")
PY
