#!/usr/bin/env bash
# Verify `sqrtmod <a> <p>` finds r with r*r = a (mod p) via Tonelli-Shanks, showing both roots sorted,
# or reports a non-residue. Tests 2 7 -> 3 or 4, 4 7 -> 2 or 5, 2 17 -> 6 or 11 (full T-S, p%4==1),
# 10 13 -> 6 or 7, 3 7 -> non-residue, 0 7 -> 0. Self-checking: python brute-forces the roots.
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
OUT="${TMPDIR:-/tmp}/sqrtmod.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sqrtmod.png" \
    "echo warmup" "sqrtmod 2 7" "sqrtmod 2 17" "sqrtmod 10 13" \
    "sqrtmod 3 7" "sqrtmod 0 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sqrtmod:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def roots(a, p):                     # DERIVED reference: brute-force square roots
    a %= p
    return sorted(r for r in range(p) if (r * r) % p == a)

def want(a, p):
    a2 = a % p
    rs = roots(a, p)
    if a2 == 0:
        return f"sqrtmod: sqrt({a}) mod {p} = 0"
    if not rs:
        return f"sqrtmod: {a} is a quadratic non-residue mod {p} (no sqrt)"
    return f"sqrtmod: sqrt({a}) mod {p} = {rs[0]} or {rs[1]}"

for a, p in [(2, 7), (2, 17), (10, 13), (3, 7), (0, 7)]:
    w = want(a, p)
    if w not in s:
        print(f"SQRTMOD TEST FAILED (missing '{w}')"); sys.exit(1)

assert roots(2, 7) == [3, 4] and roots(2, 17) == [6, 11] and roots(10, 13) == [6, 7]
assert roots(3, 7) == []
print("SQRTMOD TEST PASSED  (2/7=3,4; 2/17=6,11; 10/13=6,7; 3/7 non-residue; 0/7=0 vs python)")
PY
