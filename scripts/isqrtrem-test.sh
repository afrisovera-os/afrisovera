#!/usr/bin/env bash
# Verify `isqrtrem <n>` reports floor root r AND remainder n-r*r (n = r^2 + rem): 10->root 3 rem 1,
# 16->root 4 rem 0, 99->root 9 rem 18, 1->root 1 rem 0, 0->root 0 rem 0. Number-theory primitive
# (cf GMP isqrtrem) vs isqrt (root only). Self-checking: python r=isqrt(n); rem=n-r*r.
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
OUT="${TMPDIR:-/tmp}/isqrtrem.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isqrtrem.png" \
    "echo warmup" "isqrtrem 10" "isqrtrem 16" "isqrtrem 99" \
    "isqrtrem 1" "isqrtrem 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isqrtrem:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from math import isqrt
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    r = isqrt(n)
    return f"isqrtrem: isqrtrem({n}) = root {r} rem {n - r * r}"

for n in [10, 16, 99, 1, 0]:
    w = want(n)
    if w not in s:
        print(f"ISQRTREM TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: non-square, perfect square (rem 0), large remainder, one, zero
assert "isqrtrem: isqrtrem(10) = root 3 rem 1" in s
assert "isqrtrem: isqrtrem(16) = root 4 rem 0" in s     # perfect square
assert "isqrtrem: isqrtrem(99) = root 9 rem 18" in s    # rem = 2r
assert "isqrtrem: isqrtrem(1) = root 1 rem 0" in s
assert "isqrtrem: isqrtrem(0) = root 0 rem 0" in s

print("ISQRTREM TEST PASSED  (10->3 r1, 16->4 r0, 99->9 r18, 1->1 r0, 0->0 r0 vs python isqrt+rem)")
PY
