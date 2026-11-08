#!/usr/bin/env bash
# Verify `nextcube <n>` reports the SMALLEST perfect cube >= n and its root: 9->2^3=8? no: 9->3^3=27,
# 8->2^3=8 (already a cube), 28->4^3=64, 0->0^3=0, 1->1^3=1. Cube analogue of nextsquare; distinct
# from icbrt (FLOOR root) / iscube. Self-checking: python ceil-cube-root reference.
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
OUT="${TMPDIR:-/tmp}/nextcube.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextcube.png" \
    "echo warmup" "nextcube 9" "nextcube 8" "nextcube 28" "nextcube 0" \
    "nextcube 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextcube:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def icbrt(n):
    k = 0
    while (k + 1) ** 3 <= n:
        k += 1
    return k

def want(n):                          # DERIVED reference: ceil cube root
    r = icbrt(n)
    k = r if r ** 3 == n else r + 1
    return f"nextcube: smallest perfect cube >= {n} is {k}^3 = {k**3}"

cases = [9, 8, 28, 0, 1]
for n in cases:
    w = want(n)
    if w not in s:
        print(f"NEXTCUBE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin notable branches: round up, already a cube, mid-range round up, zero, one
assert "nextcube: smallest perfect cube >= 9 is 3^3 = 27" in s
assert "nextcube: smallest perfect cube >= 8 is 2^3 = 8" in s
assert "nextcube: smallest perfect cube >= 28 is 4^3 = 64" in s
assert "nextcube: smallest perfect cube >= 0 is 0^3 = 0" in s
assert "nextcube: smallest perfect cube >= 1 is 1^3 = 1" in s

print("NEXTCUBE TEST PASSED  (9->3^3=27, 8->2^3=8, 28->4^3=64, 0->0, 1->1 vs python ceil-cube-root)")
PY
