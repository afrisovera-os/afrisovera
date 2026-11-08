#!/usr/bin/env bash
# Verify the `collatz` command iterates the Collatz (3n+1) map to 1 and reports the
# step count + peak value (deterministic): 27 is the famous case (111 steps, peak
# 9232), 6 -> 8 steps peak 16, 1 -> 0 steps peak 1; n=0 falls through to usage.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4766,server,nowait \
    -qmp tcp:127.0.0.1:4767,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/collatz.out"
python3 scripts/shell-demo.py 4766 4767 "${TMPDIR:-/tmp}/collatz.png" \
    "collatz 1" "collatz 6" "collatz 27" "collatz 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "collatz:|usage: collatz" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "collatz: 1 -> 0 steps, peak 1",
    "collatz: 6 -> 8 steps, peak 16",
    "collatz: 27 -> 111 steps, peak 9232",
    "usage: collatz <n>",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"COLLATZ TEST FAILED (missing: {missing})"); sys.exit(1)
print("COLLATZ TEST PASSED  (27->111 steps peak 9232; 6->8 peak 16; 1->0 peak 1; 0 rejected)")
PY
