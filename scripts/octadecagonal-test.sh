#!/usr/bin/env bash
# Verify `octadecagonal <n>` classifies whether n is an 18-gonal number Oct18(k)=k(8k-7)
# (1,18,51,100,165,246,343,...), reporting index k. Tests members 18/100/165 and non-members 15/50
# (15 is the root-gate case: 32n+49=529=23^2 is a perfect square but 23 != -7 mod 16).
# Self-checking: python derives membership via 32n+49 perfect square with root == -7 (mod 16).
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
OUT="${TMPDIR:-/tmp}/octadecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/octadecagonal.png" \
    "echo warmup" "octadecagonal 18" "octadecagonal 100" "octadecagonal 165" \
    "octadecagonal 15" "octadecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "octadecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def oct18_k(n):                      # DERIVED reference: index k if octadecagonal else None
    m = 32 * n + 49
    r = isqrt(m)
    if r * r == m and (r + 7) % 16 == 0:
        return (r + 7) // 16
    return None

for n in (18, 100, 165):
    k = oct18_k(n)
    want = f"octadecagonal: {n} IS octadecagonal (Oct18({k}) = {n})"
    if want not in s:
        print(f"OCTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (15, 50):
    assert oct18_k(n) is None
    want = f"octadecagonal: {n} is NOT octadecagonal"
    if want not in s:
        print(f"OCTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence + the root-gate (15 passes the perfect-square test, fails the mod-16 gate)
assert [n for n in range(1, 350) if oct18_k(n)] == [1, 18, 51, 100, 165, 246, 343]
assert isqrt(32 * 15 + 49) ** 2 == 32 * 15 + 49 and oct18_k(15) is None
print("OCTADECAGONAL TEST PASSED  (18/100/165 members, 15/50 non-members vs python 18-gonal)")
PY
