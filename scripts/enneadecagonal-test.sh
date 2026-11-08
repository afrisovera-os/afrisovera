#!/usr/bin/env bash
# Verify `enneadecagonal <n>` classifies whether n is a 19-gonal number Ennea19(k)=k(17k-15)/2
# (1,19,54,106,175,261,364,...), reporting index k. Tests members 19/106/175 and non-members 16/50
# (16 is the root-gate case: 136n+225=2401=49^2 is a perfect square but 49 != -15 mod 34).
# Self-checking: python derives membership via 136n+225 perfect square with root == -15 (mod 34).
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
OUT="${TMPDIR:-/tmp}/enneadecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/enneadecagonal.png" \
    "echo warmup" "enneadecagonal 19" "enneadecagonal 106" "enneadecagonal 175" \
    "enneadecagonal 16" "enneadecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "enneadecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def ennea19_k(n):                    # DERIVED reference: index k if enneadecagonal else None
    m = 136 * n + 225
    r = isqrt(m)
    if r * r == m and (r + 15) % 34 == 0:
        return (r + 15) // 34
    return None

for n in (19, 106, 175):
    k = ennea19_k(n)
    want = f"enneadecagonal: {n} IS enneadecagonal (Ennea19({k}) = {n})"
    if want not in s:
        print(f"ENNEADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (16, 50):
    assert ennea19_k(n) is None
    want = f"enneadecagonal: {n} is NOT enneadecagonal"
    if want not in s:
        print(f"ENNEADECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence + the root-gate (16 passes the perfect-square test, fails the mod-34 gate)
assert [n for n in range(1, 370) if ennea19_k(n)] == [1, 19, 54, 106, 175, 261, 364]
assert isqrt(136 * 16 + 225) ** 2 == 136 * 16 + 225 and ennea19_k(16) is None
print("ENNEADECAGONAL TEST PASSED  (19/106/175 members, 16/50 non-members vs python 19-gonal)")
PY
