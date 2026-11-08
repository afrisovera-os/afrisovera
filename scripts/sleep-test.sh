#!/usr/bin/env bash
# Verify the `sleep` timekeeping primitive: `sleep 300` should block ~30 ticks
# (100 Hz timer = 10 ms/tick). Assert the reported tick count is ~30 (tolerant
# band for scheduler jitter/rounding). RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4690,server,nowait \
    -qmp tcp:127.0.0.1:4691,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/sleep.out"
python3 scripts/shell-demo.py 4690 4691 "${TMPDIR:-/tmp}/sleep.png" "sleep 300" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "sleep" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"\[sleep\] slept (\d+) ticks", s)
if not m:
    print("SLEEP TEST FAILED (no sleep result line)"); sys.exit(1)
n = int(m.group(1))
if 27 <= n <= 34:
    print(f"SLEEP TEST PASSED  (sleep 300 blocked {n} ticks, ~300 ms as expected)")
    sys.exit(0)
print(f"SLEEP TEST FAILED (expected ~30 ticks, got {n})")
sys.exit(1)
PY
