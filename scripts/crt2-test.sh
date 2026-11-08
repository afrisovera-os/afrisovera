#!/usr/bin/env bash
# Verify `crt2 <a1> <n1> <a2> <n2>` solves x = a1 (mod n1), x = a2 (mod n2) for COPRIME moduli,
# reported mod n1*n2: "crt2 2 3 3 5"->8, "crt2 1 4 2 5"->17, "crt2 0 2 0 3"->0, "crt2 2 3 2 5"->2;
# non-coprime "crt2 1 4 2 6" rejected (gcd 2). Self-checking: python CRT (pow(n1,-1,n2)).
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
OUT="${TMPDIR:-/tmp}/crt2.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/crt2.png" \
    "echo warmup" "crt2 2 3 3 5" "crt2 1 4 2 5" "crt2 0 2 0 3" \
    "crt2 2 3 2 5" "crt2 1 4 2 6" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "crt2:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import gcd
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a1, n1, a2, n2):             # DERIVED reference: CRT for two coprime moduli
    if gcd(n1, n2) != 1:
        return f"crt2: moduli {n1} and {n2} are not coprime (gcd {gcd(n1, n2)})"
    inv = pow(n1, -1, n2)
    t = ((a2 - a1) % n2) * inv % n2
    L = n1 * n2
    return f"crt2: x = {(a1 + n1 * t) % L} (mod {L})"

for (a1, n1, a2, n2) in [(2,3,3,5), (1,4,2,5), (0,2,0,3), (2,3,2,5), (1,4,2,6)]:
    w = want(a1, n1, a2, n2)
    if w not in s:
        print(f"CRT2 TEST FAILED (missing '{w}')"); sys.exit(1)

assert "crt2: x = 8 (mod 15)" in s
assert "crt2: x = 17 (mod 20)" in s
assert "crt2: x = 0 (mod 6)" in s
assert "crt2: x = 2 (mod 15)" in s
assert "crt2: moduli 4 and 6 are not coprime (gcd 2)" in s

print("CRT2 TEST PASSED  (2,3,3,5->8 mod15; 1,4,2,5->17 mod20; 0,2,0,3->0 mod6; 2,3,2,5->2 mod15; "
      "1,4,2,6->non-coprime vs python CRT)")
PY
