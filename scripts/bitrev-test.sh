#!/usr/bin/env bash
# Verify the `bitrev` command reverses the 32 bits of an integer (MSB<->LSB):
# 1 -> 0x80000000, and its round-trip 2147483648 -> 1, plus 0xffffffff -> itself.
# The test recomputes each reversal in python for a self-checking assert.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4914,server,nowait \
    -qmp tcp:127.0.0.1:4915,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/bitrev.out"
python3 scripts/shell-demo.py 4914 4915 "${TMPDIR:-/tmp}/bitrev.png" \
    "bitrev 1" "bitrev 2147483648" "bitrev 4294967295" "bitrev 43261596" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bitrev:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def br(n):
    return int(format(n, "032b")[::-1], 2)

for n in [1, 2147483648, 4294967295, 43261596]:
    r = br(n)
    want = f"bitrev: {n} = {r} (0x{r:08x})"
    if want not in s:
        print(f"BITREV TEST FAILED (expected '{want}')"); sys.exit(1)

print("BITREV TEST PASSED  (1<->0x80000000 round-trip, 0xffffffff->itself, 43261596->0x39782940)")
PY
