#!/usr/bin/env bash
# Verify `bitops <a> <b>` prints bitwise AND, OR, XOR of two ints: 12 10->and 8 or 14 xor 6,
# 255 15->and 15 or 255 xor 240, 0 0->and 0 or 0 xor 0, 6 6->and 6 or 6 xor 0 (self), 1 2->
# and 0 or 3 xor 3. Fundamental bit primitive vs popcount/hammingdist. Self-checking: python a&b,a|b,a^b.
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
OUT="${TMPDIR:-/tmp}/bitops.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bitops.png" \
    "echo warmup" "bitops 12 10" "bitops 255 15" "bitops 0 0" \
    "bitops 6 6" "bitops 1 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bitops:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b):                       # DERIVED reference
    return f"bitops: {a} and {b} -> and {a & b} or {a | b} xor {a ^ b}"

for a, b in [(12, 10), (255, 15), (0, 0), (6, 6), (1, 2)]:
    w = want(a, b)
    if w not in s:
        print(f"BITOPS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mixed, mask, zeros, self (xor 0), disjoint
assert "bitops: 12 and 10 -> and 8 or 14 xor 6" in s
assert "bitops: 255 and 15 -> and 15 or 255 xor 240" in s
assert "bitops: 0 and 0 -> and 0 or 0 xor 0" in s
assert "bitops: 6 and 6 -> and 6 or 6 xor 0" in s     # a^a == 0
assert "bitops: 1 and 2 -> and 0 or 3 xor 3" in s     # disjoint bits

print("BITOPS TEST PASSED  (12 10->8/14/6, 255 15->15/255/240, 0 0->0/0/0, 6 6->6/6/0, 1 2->0/3/3 "
      "vs python a&b,a|b,a^b)")
PY
