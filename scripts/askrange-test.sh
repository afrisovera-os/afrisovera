#!/usr/bin/env bash
# Verify the Asili assistant's NEW range/spread capability (docs/09 AI pillar): it scans the WHOLE
# integer list and reports the on-device range (max - min). "range of 3 8 5" -> "RANGE = 5 (max 8 -
# min 3, of 3 values, ON-DEVICE)"; "spread of 10 42 7" -> "RANGE = 35 (max 42 - min 7, ...)". Guarded
# on range/spread, needs >=2 ints, dispatched before try_math. Self-checking: python max-min.
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
OUT="${TMPDIR:-/tmp}/askrange.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askrange.png" \
    "echo warmup" "ask range of 3 8 5" "ask spread of 10 42 7" \
    "ask range of 100 100 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(vals):
    hi, lo = max(vals), min(vals)
    return f"[asili] RANGE = {hi - lo} (max {hi} - min {lo}, of {len(vals)} values, ON-DEVICE)"

cases = [[3, 8, 5], [10, 42, 7], [100, 100, 100]]
for vals in cases:
    w = want(vals)
    if w not in s:
        print(f"ASKRANGE TEST FAILED (missing '{w}')"); sys.exit(1)

assert want([3, 8, 5]) == "[asili] RANGE = 5 (max 8 - min 3, of 3 values, ON-DEVICE)"
assert want([100, 100, 100]) == "[asili] RANGE = 0 (max 100 - min 100, of 3 values, ON-DEVICE)"
print("ASKRANGE TEST PASSED  (range 5 / spread 35 / range 0 vs python max-min; on-device Asili)")
PY
