#!/usr/bin/env bash
# Verify the Asili assistant's NEW max/min capability (docs/09 AI pillar): it scans the WHOLE integer
# list and reports the on-device maximum or minimum. "max of 3 8 5" -> "MAX = 8 (of 3 values,
# ON-DEVICE)"; "smallest of 10 42 7" -> "MIN = 7 (of 3 values, ON-DEVICE)". Mode chosen by keyword,
# needs >=2 ints, dispatched before try_math. Self-checking: python max()/min().
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
OUT="${TMPDIR:-/tmp}/askmaxmin.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askmaxmin.png" \
    "echo warmup" "ask max of 3 8 5" "ask min of 3 8 5" "ask largest of 10 42 7" \
    "ask smallest of 10 42 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(kind, vals):
    v = max(vals) if kind == "MAX" else min(vals)
    return f"[asili] {kind} = {v} (of {len(vals)} values, ON-DEVICE)"

checks = [
    ("MAX", [3, 8, 5]),
    ("MIN", [3, 8, 5]),
    ("MAX", [10, 42, 7]),
    ("MIN", [10, 42, 7]),
]
for kind, vals in checks:
    w = want(kind, vals)
    if w not in s:
        print(f"ASKMAXMIN TEST FAILED (missing '{w}')"); sys.exit(1)

assert want("MAX", [3, 8, 5]) == "[asili] MAX = 8 (of 3 values, ON-DEVICE)"
assert want("MIN", [10, 42, 7]) == "[asili] MIN = 7 (of 3 values, ON-DEVICE)"
print("ASKMAXMIN TEST PASSED  (max 8 / min 3 / largest 42 / smallest 7 vs python; on-device Asili)")
PY
