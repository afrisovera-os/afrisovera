#!/usr/bin/env bash
# Verify `icosagonal <n>` classifies whether n is a 20-gonal number Icosa(k)=k(9k-8)
# (1,20,57,112,185,276,385,...), reporting index k. Tests members 20/112/185 and non-members 17/50
# (17 is the root-gate case: 9n+16=169=13^2 is a perfect square but 13 != -4 mod 9).
# Self-checking: python derives membership via 9n+16 perfect square with root == -4 (mod 9).
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
OUT="${TMPDIR:-/tmp}/icosagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/icosagonal.png" \
    "echo warmup" "icosagonal 20" "icosagonal 112" "icosagonal 185" \
    "icosagonal 17" "icosagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "icosagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def icosa_k(n):                      # DERIVED reference: index k if icosagonal else None
    m = 9 * n + 16
    r = isqrt(m)
    if r * r == m and (r + 4) % 9 == 0:
        return (r + 4) // 9
    return None

for n in (20, 112, 185):
    k = icosa_k(n)
    want = f"icosagonal: {n} IS icosagonal (Icosa({k}) = {n})"
    if want not in s:
        print(f"ICOSAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (17, 50):
    assert icosa_k(n) is None
    want = f"icosagonal: {n} is NOT icosagonal"
    if want not in s:
        print(f"ICOSAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence + the root-gate (17 passes the perfect-square test, fails the mod-9 gate)
assert [n for n in range(1, 390) if icosa_k(n)] == [1, 20, 57, 112, 185, 276, 385]
assert isqrt(9 * 17 + 16) ** 2 == 9 * 17 + 16 and icosa_k(17) is None
print("ICOSAGONAL TEST PASSED  (20/112/185 members, 17/50 non-members vs python 20-gonal)")
PY
