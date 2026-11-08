#!/usr/bin/env bash
# Verify the Asili assistant's NEW power/exponentiation capability (docs/09 AI pillar):
# "ask 2 to the power of 10"->1024, "ask 2 ^ 10"->1024 (the '^' path), "ask 3 to the power of 4"->81,
# "ask 5 ^ 0"->1, "ask what is 2 to the power of 16"->65536. base^exp on the first two non-negative
# ints, exp capped at 63, checked_pow so overflow falls through. Guarded on "power" or '^', dispatched
# before try_math (whose operator set omits '^'). Self-checking: python's ** operator.
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
OUT="${TMPDIR:-/tmp}/askpower.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askpower.png" \
    "echo warmup" "ask 2 to the power of 10" "ask 2 ^ 10" "ask 3 to the power of 4" \
    "ask 5 ^ 0" "ask what is 2 to the power of 16" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# derived expectations: 2^10=1024, 2^10=1024, 3^4=81, 5^0=1, 2^16=65536
for base, exp in ((2, 10), (2, 10), (3, 4), (5, 0), (2, 16)):
    want = f"[asili] = {base ** exp} (POWER, ON-DEVICE)"
    if want not in s:
        print(f"ASKPOWER TEST FAILED (missing '{want}' for {base}^{exp})"); sys.exit(1)

print("ASKPOWER TEST PASSED  (2 to the power of 10->1024, 2 ^ 10->1024, 3 to the power of 4->81, "
      "5 ^ 0->1, 2 to the power of 16->65536; on-device Asili power, dispatched before math)")
PY
