#!/usr/bin/env bash
# Verify the `swapcase` command inverts the case of each ASCII letter, passing
# non-letters through unchanged. Deterministic:
#   swapcase AfriSovera OS       -> "swapcase: aFRIsOVERA os"
#   swapcase Hello, World! 123   -> "swapcase: hELLO, wORLD! 123"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4728,server,nowait \
    -qmp tcp:127.0.0.1:4729,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/swapcase.out"
python3 scripts/shell-demo.py 4728 4729 "${TMPDIR:-/tmp}/swapcase.png" "swapcase AfriSovera OS" "swapcase Hello, World! 123" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapcase: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["swapcase: aFRIsOVERA os", "swapcase: hELLO, wORLD! 123"]
missing = [w for w in want if w not in s]
if missing:
    print(f"SWAPCASE TEST FAILED (missing: {missing})"); sys.exit(1)
print("SWAPCASE TEST PASSED  (AfriSovera OS -> aFRIsOVERA os; punctuation/digits pass through)")
PY
