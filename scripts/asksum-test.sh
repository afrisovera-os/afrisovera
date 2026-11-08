#!/usr/bin/env bash
# Verify the Asili assistant's NEW sum/total capability (docs/09 AI pillar): it scans the WHOLE integer
# list and reports the on-device sum. "sum of 3 8 5" -> "SUM = 16 (of 3 values, ON-DEVICE)"; "total of
# 10 20 30" -> "SUM = 60". Also confirms "sum of digits of 12345" still routes to try_digitsum (= 15),
# proving dispatch order. Self-checking: python sum().
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
OUT="${TMPDIR:-/tmp}/asksum.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/asksum.png" \
    "echo warmup" "ask sum of 3 8 5" "ask total of 10 20 30" "ask add up 1 2 3 4" \
    "ask sum of digits of 12345" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(vals):
    return f"[asili] SUM = {sum(vals)} (of {len(vals)} values, ON-DEVICE)"

for vals in [[3, 8, 5], [10, 20, 30], [1, 2, 3, 4]]:
    w = want(vals)
    if w not in s:
        print(f"ASKSUM TEST FAILED (missing '{w}')"); sys.exit(1)

# dispatch-order guard: "sum of digits of 12345" must reach try_digitsum, not try_sum
if "[asili] = 15 (DIGIT SUM, ON-DEVICE)" not in s:
    print("ASKSUM TEST FAILED ('sum of digits' did not route to digitsum)"); sys.exit(1)

assert want([3, 8, 5]) == "[asili] SUM = 16 (of 3 values, ON-DEVICE)"
print("ASKSUM TEST PASSED  (16/60/10 sums vs python; 'sum of digits' still digitsum=15)")
PY
