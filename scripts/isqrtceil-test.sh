#!/usr/bin/env bash
# Verify `isqrtceil <n>` gives the ceiling integer square root (smallest k, k^2>=n): 10->4, 16->4,
# 17->5, 1->1, 0->0. Ceil complement of isqrt (floor). Self-checking: python r=isqrt(n); r if r*r==n else r+1.
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
OUT="${TMPDIR:-/tmp}/isqrtceil.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isqrtceil.png" \
    "echo warmup" "isqrtceil 10" "isqrtceil 16" "isqrtceil 17" \
    "isqrtceil 1" "isqrtceil 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isqrtceil:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    r = isqrt(n)
    return f"isqrtceil: isqrtceil({n}) = {r if r * r == n else r + 1}"

for n in [10, 16, 17, 1, 0]:
    w = want(n)
    if w not in s:
        print(f"ISQRTCEIL TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: non-square, perfect square, just-over, one, zero
assert "isqrtceil: isqrtceil(10) = 4" in s
assert "isqrtceil: isqrtceil(16) = 4" in s      # perfect square -> floor==ceil
assert "isqrtceil: isqrtceil(17) = 5" in s
assert "isqrtceil: isqrtceil(1) = 1" in s
assert "isqrtceil: isqrtceil(0) = 0" in s

print("ISQRTCEIL TEST PASSED  (10->4, 16->4, 17->5, 1->1, 0->0 vs python ceil integer sqrt)")
PY
