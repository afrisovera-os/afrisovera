#!/usr/bin/env bash
# Verify `delannoy <m> <n>` computes the Delannoy number D(m,n) (paths (0,0)->(m,n) via right/up/
# diagonal). Tests D(0,5)=1, D(1,1)=3, D(2,2)=13, D(3,3)=63, D(3,4)=129. Self-checking: python DP.
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
OUT="${TMPDIR:-/tmp}/delannoy.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/delannoy.png" \
    "echo warmup" "delannoy 0 5" "delannoy 1 1" "delannoy 2 2" \
    "delannoy 3 3" "delannoy 3 4" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "delannoy:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from functools import lru_cache
s = open(sys.argv[1], "rb").read().decode("latin1")

@lru_cache(maxsize=None)
def D(m, n):
    if m == 0 or n == 0: return 1
    return D(m - 1, n) + D(m, n - 1) + D(m - 1, n - 1)

for m, n in [(0, 5), (1, 1), (2, 2), (3, 3), (3, 4)]:
    want = f"delannoy: D({m}, {n}) = {D(m, n)}"
    if want not in s:
        print(f"DELANNOY TEST FAILED (missing '{want}')"); sys.exit(1)

assert D(1, 1) == 3 and D(2, 2) == 13 and D(3, 3) == 63 and D(3, 4) == 129 and D(0, 5) == 1
print("DELANNOY TEST PASSED  (D(0,5)=1, D(1,1)=3, D(2,2)=13, D(3,3)=63, D(3,4)=129 vs python)")
PY
