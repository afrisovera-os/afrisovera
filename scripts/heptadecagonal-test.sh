#!/usr/bin/env bash
# Verify `heptadecagonal <n>` classifies whether n is a 17-gonal number Heptadec(k)=k(15k-13)/2
# (1,17,48,94,155,231,322,...), reporting index k. Tests members 17/94/155 and non-members 10/50
# (10 is the root-gate case: 120n+169=1369=37^2 is a perfect square but 37 != -13 mod 30).
# Self-checking: python derives membership via 120n+169 perfect square with root == -13 (mod 30).
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
OUT="${TMPDIR:-/tmp}/heptadecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/heptadecagonal.png" \
    "echo warmup" "heptadecagonal 17" "heptadecagonal 94" "heptadecagonal 155" \
    "heptadecagonal 10" "heptadecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "heptadecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def heptadec_k(n):                   # DERIVED reference: index k if heptadecagonal else None
    m = 120 * n + 169
    r = isqrt(m)
    if r * r == m and (r + 13) % 30 == 0:
        return (r + 13) // 30
    return None

for n in (17, 94, 155):
    k = heptadec_k(n)
    want = f"heptadecagonal: {n} IS heptadecagonal (Heptadec({k}) = {n})"
    if want not in s:
        print(f"HEPTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert heptadec_k(n) is None
    want = f"heptadecagonal: {n} is NOT heptadecagonal"
    if want not in s:
        print(f"HEPTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence + the root-gate (10 passes the perfect-square test, fails the mod-30 gate)
assert [n for n in range(1, 330) if heptadec_k(n)] == [1, 17, 48, 94, 155, 231, 322]
assert isqrt(120 * 10 + 169) ** 2 == 120 * 10 + 169 and heptadec_k(10) is None
print("HEPTADECAGONAL TEST PASSED  (17/94/155 members, 10/50 non-members vs python 17-gonal)")
PY
