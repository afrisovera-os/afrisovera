#!/usr/bin/env bash
# Verify the `product` command multiplies a list of integers in u128 with an
# overflow guard: 2 3 4 -> 24; 1 2 3 4 5 -> 120; 10 0 7 -> 0 (zero in list); a
# product exceeding u128 -> overflow message. Test recomputes the product in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4994,server,nowait \
    -qmp tcp:127.0.0.1:4995,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/product.out"
python3 scripts/shell-demo.py 4994 4995 "${TMPDIR:-/tmp}/product.png" \
    "product 2 3 4" "product 1 2 3 4 5" "product 10 0 7" \
    "product 4294967296 4294967296 4294967296 4294967296 4294967296" "product" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "product:|usage: product" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def prod(lst):
    p = 1
    for v in lst:
        p *= v
    return p

for lst in [[2, 3, 4], [1, 2, 3, 4, 5], [10, 0, 7]]:
    want = f"product: {prod(lst)} (of {len(lst)} values)"
    if want not in s:
        print(f"PRODUCT TEST FAILED (expected '{want}' for {lst})"); sys.exit(1)

if "product: overflows u128" not in s:
    print("PRODUCT TEST FAILED (missing overflow guard line)"); sys.exit(1)
if "usage: product <n1> <n2> ..." not in s:
    print("PRODUCT TEST FAILED (missing usage line for empty)"); sys.exit(1)

print("PRODUCT TEST PASSED  (2 3 4->24, 1..5->120, 10 0 7->0, overflow guarded, empty->usage)")
PY
