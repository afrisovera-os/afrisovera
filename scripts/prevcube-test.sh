#!/usr/bin/env bash
# Verify `prevcube <n>` reports the LARGEST perfect cube <= n and its root -- the mirror of nextcube:
# 30->27 (3^3), 27->27 (already a cube), 8->8, 7->1, 0->0. Distinct from icbrt (FLOOR root VALUE) /
# iscube / prevsquare. Self-checking: python floor-cube-root reference.
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
OUT="${TMPDIR:-/tmp}/prevcube.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevcube.png" \
    "echo warmup" "prevcube 30" "prevcube 27" "prevcube 8" "prevcube 7" \
    "prevcube 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevcube:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def icbrt(n):
    k = 0
    while (k + 1) ** 3 <= n:
        k += 1
    return k

def want(n):                          # DERIVED reference: floor cube root
    k = icbrt(n)
    return f"prevcube: largest cube <= {n} is {k**3} ({k}^3)"

cases = [30, 27, 8, 7, 0]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"PREVCUBE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin notable branches: round down, already a cube, small cube, below 8->1, zero
assert "prevcube: largest cube <= 30 is 27 (3^3)" in s
assert "prevcube: largest cube <= 27 is 27 (3^3)" in s
assert "prevcube: largest cube <= 8 is 8 (2^3)" in s
assert "prevcube: largest cube <= 7 is 1 (1^3)" in s
assert "prevcube: largest cube <= 0 is 0 (0^3)" in s

print("PREVCUBE TEST PASSED  (30->27, 27->27, 8->8, 7->1, 0->0 vs python floor-cube-root)")
PY
