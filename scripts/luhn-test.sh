#!/usr/bin/env bash
# Verify the `luhn` command validates a digit string with the Luhn mod-10 check-digit
# algorithm: 79927398713 -> VALID (the canonical Wikipedia example, not a real card),
# 79927398710 -> INVALID, 18 -> VALID (sum 10). The test recomputes Luhn in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4926,server,nowait \
    -qmp tcp:127.0.0.1:4927,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/luhn.out"
python3 scripts/shell-demo.py 4926 4927 "${TMPDIR:-/tmp}/luhn.png" \
    "luhn 79927398713" "luhn 79927398710" "luhn 18" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "luhn:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def luhn(d):
    total = 0
    for i, ch in enumerate(reversed(d)):
        v = int(ch)
        if i % 2 == 1:
            v *= 2
            if v > 9:
                v -= 9
        total += v
    return total, total % 10 == 0

for digits in ["79927398713", "79927398710", "18"]:
    total, ok = luhn(digits)
    want = f"luhn: {digits} is {'VALID' if ok else 'INVALID'} (checksum sum {total}, mod 10 = {total % 10})"
    if want not in s:
        print(f"LUHN TEST FAILED (expected '{want}')"); sys.exit(1)

print("LUHN TEST PASSED  (79927398713 VALID, 79927398710 INVALID, 18 VALID)")
PY
