#!/usr/bin/env bash
# Verify the `factorial` command computes n! in u128 with an explicit overflow
# guard. The harness recomputes each value with python math.factorial, so it is
# self-checking: 0!=1, 1!=1, 5!=120, 10!=3628800, 20!, 34! (the largest that fits
# u128), and 35! -> overflow (35! ~1.03e40 > u128 max ~3.4e38); bad arg -> usage.
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
OUT="${TMPDIR:-/tmp}/factorial.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/factorial.png" \
    "factorial 0" "factorial 1" "factorial 5" "factorial 10" "factorial 20" \
    "factorial 34" "factorial 35" "factorial abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "factorial:|usage: factorial" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

for n in [0, 1, 5, 10, 20, 34]:
    want = f"factorial: {n}! = {math.factorial(n)}"
    if want not in s:
        print(f"FACTORIAL TEST FAILED (missing '{want}')"); sys.exit(1)

# 34! fits u128, 35! does not.
assert math.factorial(34) <= 2**128 - 1 and math.factorial(35) > 2**128 - 1
if "factorial: 35! overflows u128" not in s:
    print("FACTORIAL TEST FAILED (missing 35! overflow line)"); sys.exit(1)
if "usage: factorial <n>" not in s:
    print("FACTORIAL TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("FACTORIAL TEST PASSED  (0..20 & 34! exact vs python, 35! overflow-guarded, bad->usage)")
PY
