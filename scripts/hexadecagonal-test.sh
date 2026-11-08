#!/usr/bin/env bash
# Verify `hexadecagonal <n>` classifies whether n is a 16-gonal number Hexadec(k)=k(7k-6)
# (1,16,45,88,145,216,...), reporting index k. Tests members 16/88/145 and non-members 10/50.
# Self-checking: python derives membership via 7n+9 perfect square with root == -3 (mod 7).
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
OUT="${TMPDIR:-/tmp}/hexadecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hexadecagonal.png" \
    "echo warmup" "hexadecagonal 16" "hexadecagonal 88" "hexadecagonal 145" \
    "hexadecagonal 10" "hexadecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hexadecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def hexadec_k(n):                    # DERIVED reference: index k if hexadecagonal else None
    m = 7 * n + 9
    r = isqrt(m)
    if r * r == m and (r + 3) % 7 == 0:
        return (r + 3) // 7
    return None

for n in (16, 88, 145):
    k = hexadec_k(n)
    want = f"hexadecagonal: {n} IS hexadecagonal (Hexadec({k}) = {n})"
    if want not in s:
        print(f"HEXADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert hexadec_k(n) is None
    want = f"hexadecagonal: {n} is NOT hexadecagonal"
    if want not in s:
        print(f"HEXADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 310) if hexadec_k(n)] == [1, 16, 45, 88, 145, 216, 301]
print("HEXADECAGONAL TEST PASSED  (16/88/145 members, 10/50 non-members vs python 16-gonal)")
PY
