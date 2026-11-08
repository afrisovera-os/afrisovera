#!/usr/bin/env bash
# Verify `hendecagonal <n>` classifies whether n is an 11-gonal number Hen(k)=k(9k-7)/2
# (1,11,30,58,95,141,...), reporting index k. Tests members 11/30/58/95 and non-members 10/50.
# Self-checking: python derives membership via 49+72n perfect square with root == -7 (mod 18).
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
OUT="${TMPDIR:-/tmp}/hendecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hendecagonal.png" \
    "echo warmup" "hendecagonal 30" "hendecagonal 95" "hendecagonal 58" \
    "hendecagonal 10" "hendecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hendecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def hendec_k(n):                     # DERIVED reference: index k if hendecagonal else None
    m = 72 * n + 49
    r = isqrt(m)
    if r * r == m and (r + 7) % 18 == 0:
        return (r + 7) // 18
    return None

for n in (30, 95, 58):
    k = hendec_k(n)
    want = f"hendecagonal: {n} IS hendecagonal (Hen({k}) = {n})"
    if want not in s:
        print(f"HENDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert hendec_k(n) is None
    want = f"hendecagonal: {n} is NOT hendecagonal"
    if want not in s:
        print(f"HENDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 150) if hendec_k(n)] == [1, 11, 30, 58, 95, 141]
print("HENDECAGONAL TEST PASSED  (30/95/58 members, 10/50 non-members vs python 11-gonal)")
PY
