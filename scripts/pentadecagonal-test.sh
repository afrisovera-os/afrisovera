#!/usr/bin/env bash
# Verify `pentadecagonal <n>` classifies whether n is a 15-gonal number Pentadec(k)=k(13k-11)/2
# (1,15,42,82,135,201,...), reporting index k. Tests members 15/82/135 and non-members 10/50.
# Self-checking: python derives membership via 104n+121 perfect square with root == -11 (mod 26).
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
OUT="${TMPDIR:-/tmp}/pentadecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pentadecagonal.png" \
    "echo warmup" "pentadecagonal 15" "pentadecagonal 82" "pentadecagonal 135" \
    "pentadecagonal 10" "pentadecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pentadecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def pentadec_k(n):                   # DERIVED reference: index k if pentadecagonal else None
    m = 104 * n + 121
    r = isqrt(m)
    if r * r == m and (r + 11) % 26 == 0:
        return (r + 11) // 26
    return None

for n in (15, 82, 135):
    k = pentadec_k(n)
    want = f"pentadecagonal: {n} IS pentadecagonal (Pentadec({k}) = {n})"
    if want not in s:
        print(f"PENTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert pentadec_k(n) is None
    want = f"pentadecagonal: {n} is NOT pentadecagonal"
    if want not in s:
        print(f"PENTADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 260) if pentadec_k(n)] == [1, 15, 42, 82, 135, 201]
print("PENTADECAGONAL TEST PASSED  (15/82/135 members, 10/50 non-members vs python 15-gonal)")
PY
