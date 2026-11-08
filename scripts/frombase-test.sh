#!/usr/bin/env bash
# Verify `frombase <digits> <base>` parses digits in base 2..36 to decimal. Tests 1010 2 -> 10,
# ff 16 -> 255, 777 8 -> 511, z 36 -> 35, and an invalid digit (129 8) rejected. Self-checking:
# python int(digits, base).
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
OUT="${TMPDIR:-/tmp}/frombase.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/frombase.png" \
    "echo warmup" "frombase 1010 2" "frombase ff 16" "frombase 777 8" \
    "frombase z 36" "frombase 129 8" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "frombase:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for d, b in [("1010", 2), ("ff", 16), ("777", 8), ("z", 36)]:
    want = f"frombase: {d} (base {b}) = {int(d, b)}"
    if want not in s:
        print(f"FROMBASE TEST FAILED (missing '{want}')"); sys.exit(1)

assert int("1010", 2) == 10 and int("ff", 16) == 255 and int("777", 8) == 511 and int("z", 36) == 35
# 129 is not a valid base-8 number (digit 9 out of range)
if "frombase: '129' is not a valid base-8 number" not in s:
    print("FROMBASE TEST FAILED (missing invalid-digit report for 129 base 8)"); sys.exit(1)
print("FROMBASE TEST PASSED  (1010/2=10, ff/16=255, 777/8=511, z/36=35; 129/8 rejected vs python)")
PY
