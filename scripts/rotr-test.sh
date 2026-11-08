#!/usr/bin/env bash
# Verify `rotr <n> <k>` rotates the 64 bits of n RIGHT by k (bits off the bottom wrap to the top):
# rotr 2 1 -> 1, rotr 1 1 -> 2^63, rotr 1 63 -> 2, rotr 65280 8 -> 255. k taken mod 64. The inverse
# of rotl. Self-checking: python replicates a u64 rotate_right for every reference (value + hex).
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
OUT="${TMPDIR:-/tmp}/rotr.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rotr.png" \
    "echo warmup" "rotr 2 1" "rotr 1 1" "rotr 1 63" "rotr 65280 8" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rotr:|usage: rotr" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
M = (1 << 64) - 1

def rotr(n, k):  # mirror u64::rotate_right: k mod 64, off-bottom bits wrap to the top
    k %= 64
    return n & M if k == 0 else ((n >> k) | (n << (64 - k))) & M

def message(n, k):
    r = rotr(n, k)
    return f"rotr: {n} rotr {k} = {r} (0x{r:016x})"

cases = [(2, 1), (1, 1), (1, 63), (65280, 8)]
for n, k in cases:
    want = message(n, k)   # value + hex DERIVED by python, never hand-typed
    if want not in s:
        print(f"ROTR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: >>1, low bit wraps to the high bit, wrap-back, byte shift
assert message(2, 1) in s and "= 1 " in message(2, 1)
assert message(1, 1) in s and "= 9223372036854775808 " in message(1, 1)  # wrap to top
assert message(65280, 8) in s and "= 255 " in message(65280, 8)

print("ROTR TEST PASSED  (2 rotr 1=1, 1 rotr 1=2^63 wrap, 1 rotr 63=2, 65280 rotr 8=255 "
      "vs python u64 rotate_right)")
PY
