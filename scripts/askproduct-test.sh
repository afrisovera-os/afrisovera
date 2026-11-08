#!/usr/bin/env bash
# Verify the Asili assistant's NEW product/multiply capability (docs/09 AI pillar), the
# multiplicative sibling of asksum: it scans the WHOLE integer list and reports the on-device
# product. "product of 3 4 5" -> "PRODUCT = 60 (of 3 values, ON-DEVICE)"; "multiply 6 and 7" ->
# "PRODUCT = 42 (of 2 values, ON-DEVICE)". Also confirms "6 x 7" (an operator, not a product-list
# phrasing) still routes to try_math, proving dispatch order. Self-checking: python math.prod().
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
OUT="${TMPDIR:-/tmp}/askproduct.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askproduct.png" \
    "echo warmup" "ask product of 3 4 5" "ask multiply 6 and 7" "ask multiplied 2 3 4" \
    "ask 6 x 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(vals):
    return f"[asili] PRODUCT = {math.prod(vals)} (of {len(vals)} values, ON-DEVICE)"

for vals in [[3, 4, 5], [6, 7], [2, 3, 4]]:
    w = want(vals)
    if w not in s:
        print(f"ASKPRODUCT TEST FAILED (missing '{w}')"); sys.exit(1)

# dispatch-order guard: "6 x 7" (operator phrasing, no product/multiply keyword) must still
# reach try_math, not try_product.
if "[asili] ANSWER: 42   (COMPUTED ON-DEVICE, PRIVATELY)" not in s:
    print("ASKPRODUCT TEST FAILED ('6 x 7' did not route to try_math)"); sys.exit(1)

assert want([3, 4, 5]) == "[asili] PRODUCT = 60 (of 3 values, ON-DEVICE)"
print("ASKPRODUCT TEST PASSED  (60/42/24 products vs python math.prod; '6 x 7' still try_math=42)")
PY
