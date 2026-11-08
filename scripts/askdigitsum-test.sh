#!/usr/bin/env bash
# Verify the Asili assistant's NEW digit-sum capability (docs/09 AI pillar): "ask digit sum of
# 12345"->15, "ask sum of digits of 9999"->36, "ask digitsum 0"->0, "ask what is the digit sum of
# 100"->1, "ask sum of the digits of 2024"->8. Sums decimal digits of the first integer (>=0),
# guarded on digit sum / digitsum / sum of digits, dispatched before try_math (no operator present).
# Self-checking: python sum of digits.
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
OUT="${TMPDIR:-/tmp}/askdigitsum.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askdigitsum.png" \
    "echo warmup" "ask digit sum of 12345" "ask sum of digits of 9999" "ask digitsum 0" \
    "ask what is the digit sum of 100" "ask sum of the digits of 2024" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# derived expectations: digit sums of 12345, 9999, 0, 100, 2024
for n in (12345, 9999, 0, 100, 2024):
    want = f"[asili] = {sum(int(c) for c in str(n))} (DIGIT SUM, ON-DEVICE)"
    if want not in s:
        print(f"ASKDIGITSUM TEST FAILED (missing '{want}' for n={n})"); sys.exit(1)

print("ASKDIGITSUM TEST PASSED  (digit sum of 12345->15, 9999->36, 0->0, 100->1, 2024->8; "
      "on-device Asili digit sum, dispatched before math)")
PY
