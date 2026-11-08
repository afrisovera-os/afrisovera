#!/usr/bin/env bash
# Verify `tridecagonal <n>` classifies whether n is a 13-gonal number Tridec(k)=k(11k-9)/2
# (1,13,36,70,115,171,...), reporting index k. Tests members 13/70/115 and non-members 10/50.
# Self-checking: python derives membership via 88n+81 perfect square with root == -9 (mod 22).
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
OUT="${TMPDIR:-/tmp}/tridecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tridecagonal.png" \
    "echo warmup" "tridecagonal 13" "tridecagonal 70" "tridecagonal 115" \
    "tridecagonal 10" "tridecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tridecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def tridec_k(n):                     # DERIVED reference: index k if tridecagonal else None
    m = 88 * n + 81
    r = isqrt(m)
    if r * r == m and (r + 9) % 22 == 0:
        return (r + 9) // 22
    return None

for n in (13, 70, 115):
    k = tridec_k(n)
    want = f"tridecagonal: {n} IS tridecagonal (Tridec({k}) = {n})"
    if want not in s:
        print(f"TRIDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert tridec_k(n) is None
    want = f"tridecagonal: {n} is NOT tridecagonal"
    if want not in s:
        print(f"TRIDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 240) if tridec_k(n)] == [1, 13, 36, 70, 115, 171, 238]
print("TRIDECAGONAL TEST PASSED  (13/70/115 members, 10/50 non-members vs python 13-gonal)")
PY
