#!/usr/bin/env bash
# Verify the `minmax` command reports the min and max of a list of integers.
# Deterministic:
#   minmax 3 1 4 1 5 9 2 6  -> min 1, max 9 (8 values)
#   minmax 42               -> min 42, max 42 (1 values)
#   minmax 100 50 200 25    -> min 25, max 200 (4 values)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4798,server,nowait \
    -qmp tcp:127.0.0.1:4799,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/minmax.out"
python3 scripts/shell-demo.py 4798 4799 "${TMPDIR:-/tmp}/minmax.png" "minmax 3 1 4 1 5 9 2 6" "minmax 42" "minmax 100 50 200 25" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "minmax: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "minmax: min 1, max 9 (8 values)",
    "minmax: min 42, max 42 (1 values)",
    "minmax: min 25, max 200 (4 values)",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"MINMAX TEST FAILED (missing: {missing})"); sys.exit(1)
print("MINMAX TEST PASSED  (1..9/8; 42/42/1; 25..200/4)")
PY
