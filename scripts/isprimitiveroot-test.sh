#!/usr/bin/env bash
# Verify `isprimitiveroot <g> <p>` tests whether g is a primitive root modulo prime p (mult order of
# g mod p == p-1). Tests 3 7 -> YES, 2 7 -> NO, 2 11 -> YES, 5 23 -> YES, plus composite modulus 9.
# The membership companion of primitiveroot. Self-checking: python order of g mod p == p-1.
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
OUT="${TMPDIR:-/tmp}/isprimitiveroot.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isprimitiveroot.png" \
    "echo warmup" "isprimitiveroot 3 7" "isprimitiveroot 2 7" "isprimitiveroot 2 11" \
    "isprimitiveroot 5 23" "isprimitiveroot 3 9" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isprimitiveroot:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_primroot(g, p):               # DERIVED reference: order of g mod p == p-1
    g %= p
    if g == 0: return False
    seen, x, order = set(), 1, 0
    for k in range(1, p):
        x = (x * g) % p
        if x == 1:
            order = k; break
    return order == p - 1

checks = [(3, 7, True), (2, 7, False), (2, 11, True), (5, 23, True)]
for g, p, exp in checks:
    assert is_primroot(g, p) == exp, (g, p)
    kind = "IS" if exp else "is NOT"
    want = f"isprimitiveroot: {g} {kind} a primitive root mod {p}"
    if want not in s:
        print(f"ISPRIMITIVEROOT TEST FAILED (missing '{want}')"); sys.exit(1)

assert "isprimitiveroot: 9 is not prime (need a prime modulus)" in s
print("ISPRIMITIVEROOT TEST PASSED  (3/7 YES, 2/7 NO, 2/11 YES, 5/23 YES vs python; composite 9 rejected)")
PY
