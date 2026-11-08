#!/usr/bin/env bash
# Verify `henicosagonal <n>` classifies whether n is a 21-gonal number Heni21(k)=k(19k-17)/2
# (1,21,60,118,195,291,406,...), reporting index k. Tests members 21/118/195 and non-members 18/50
# (18 is the root-gate case: 152n+289=3025=55^2 is a perfect square but 55 != -17 mod 38).
# Self-checking: python derives membership via 152n+289 perfect square with root == -17 (mod 38).
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
OUT="${TMPDIR:-/tmp}/henicosagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/henicosagonal.png" \
    "echo warmup" "henicosagonal 21" "henicosagonal 118" "henicosagonal 195" \
    "henicosagonal 18" "henicosagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "henicosagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def heni_k(n):                       # DERIVED reference: index k if henicosagonal else None
    m = 152 * n + 289
    r = isqrt(m)
    if r * r == m and (r + 17) % 38 == 0:
        return (r + 17) // 38
    return None

for n in (21, 118, 195):
    k = heni_k(n)
    want = f"henicosagonal: {n} IS henicosagonal (Heni21({k}) = {n})"
    if want not in s:
        print(f"HENICOSAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (18, 50):
    assert heni_k(n) is None
    want = f"henicosagonal: {n} is NOT henicosagonal"
    if want not in s:
        print(f"HENICOSAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence + the root-gate (18 passes the perfect-square test, fails the mod-38 gate)
assert [n for n in range(1, 407) if heni_k(n)] == [1, 21, 60, 118, 195, 291, 406]
assert isqrt(152 * 18 + 289) ** 2 == 152 * 18 + 289 and heni_k(18) is None
print("HENICOSAGONAL TEST PASSED  (21/118/195 members, 18/50 non-members vs python 21-gonal)")
PY
