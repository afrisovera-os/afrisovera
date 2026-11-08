#!/usr/bin/env bash
# Verify `tetradecagonal <n>` classifies whether n is a 14-gonal number Tetradec(k)=k(6k-5)
# (1,14,39,76,125,186,...), reporting index k. Tests members 14/76/125 and non-members 10/50.
# Self-checking: python derives membership via 24n+25 perfect square with root == -5 (mod 12).
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
OUT="${TMPDIR:-/tmp}/tetradecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tetradecagonal.png" \
    "echo warmup" "tetradecagonal 14" "tetradecagonal 76" "tetradecagonal 125" \
    "tetradecagonal 10" "tetradecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tetradecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def tetradec_k(n):                   # DERIVED reference: index k if tetradecagonal else None
    m = 24 * n + 25
    r = isqrt(m)
    if r * r == m and (r + 5) % 12 == 0:
        return (r + 5) // 12
    return None

for n in (14, 76, 125):
    k = tetradec_k(n)
    want = f"tetradecagonal: {n} IS tetradecagonal (Tetradec({k}) = {n})"
    if want not in s:
        print(f"TETRADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert tetradec_k(n) is None
    want = f"tetradecagonal: {n} is NOT tetradecagonal"
    if want not in s:
        print(f"TETRADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 260) if tetradec_k(n)] == [1, 14, 39, 76, 125, 186, 259]
print("TETRADECAGONAL TEST PASSED  (14/76/125 members, 10/50 non-members vs python 14-gonal)")
PY
