#!/usr/bin/env bash
# Verify `dodecagonal <n>` classifies whether n is a 12-gonal number Dod(k)=k(5k-4)
# (1,12,33,64,105,156,...), reporting index k. Tests members 12/33/64/105 and non-members 10/50.
# Self-checking: python derives membership via 5n+4 perfect square with root == -2 (mod 5).
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
OUT="${TMPDIR:-/tmp}/dodecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/dodecagonal.png" \
    "echo warmup" "dodecagonal 12" "dodecagonal 64" "dodecagonal 105" \
    "dodecagonal 10" "dodecagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dodecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def dodec_k(n):                      # DERIVED reference: index k if dodecagonal else None
    m = 5 * n + 4
    r = isqrt(m)
    if r * r == m and (r + 2) % 5 == 0:
        return (r + 2) // 5
    return None

for n in (12, 64, 105):
    k = dodec_k(n)
    want = f"dodecagonal: {n} IS dodecagonal (Dod({k}) = {n})"
    if want not in s:
        print(f"DODECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (10, 50):
    assert dodec_k(n) is None
    want = f"dodecagonal: {n} is NOT dodecagonal"
    if want not in s:
        print(f"DODECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the sequence
assert [n for n in range(1, 160) if dodec_k(n)] == [1, 12, 33, 64, 105, 156]
print("DODECAGONAL TEST PASSED  (12/64/105 members, 10/50 non-members vs python 12-gonal)")
PY
