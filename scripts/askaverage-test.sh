#!/usr/bin/env bash
# Verify the Asili assistant's NEW average/mean capability (docs/09 AI pillar): it scans the WHOLE
# integer list and reports the on-device arithmetic mean. "average of 10 20 30" -> "MEAN = 20 (of 3
# values, sum 60, ON-DEVICE)"; non-divisible sums add a remainder ("average of 1 2 4" -> "2 r 1").
# Guarded on average/mean, needs >=2 ints, dispatched before try_math. Self-checking: python sum//n.
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
OUT="${TMPDIR:-/tmp}/askaverage.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askaverage.png" \
    "echo warmup" "ask average of 10 20 30" "ask mean of 100 200" \
    "ask average of 1 2 4" "ask mean of 2 3 4 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(vals):                      # DERIVED reference matching kernel formatting
    n = len(vals); sm = sum(vals); q = sm // n; r = sm % n
    core = f"MEAN = {q}" + (f" r {r}" if r else "")
    return f"[asili] {core} (of {n} values, sum {sm}, ON-DEVICE)"

cases = [[10, 20, 30], [100, 200], [1, 2, 4], [2, 3, 4, 5]]
for vals in cases:
    w = want(vals)
    if w not in s:
        print(f"ASKAVERAGE TEST FAILED (missing '{w}')"); sys.exit(1)

# sanity on the two flavors
assert want([10, 20, 30]) == "[asili] MEAN = 20 (of 3 values, sum 60, ON-DEVICE)"
assert want([1, 2, 4]) == "[asili] MEAN = 2 r 1 (of 3 values, sum 7, ON-DEVICE)"
print("ASKAVERAGE TEST PASSED  (20 exact / 150 exact / 2 r 1 / 3 r 2 vs python; on-device Asili mean)")
PY
