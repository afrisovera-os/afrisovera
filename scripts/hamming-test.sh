#!/usr/bin/env bash
# Verify the `hamming` command computes the Hamming distance (count of differing
# positions) between two equal-length strings: karolin/kathrin -> 3 (the classic
# example), 1011101/1001001 -> 2, abc/abc -> 0; unequal lengths report a mismatch.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4866,server,nowait \
    -qmp tcp:127.0.0.1:4867,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/hamming.out"
python3 scripts/shell-demo.py 4866 4867 "${TMPDIR:-/tmp}/hamming.png" \
    "hamming karolin kathrin" "hamming 1011101 1001001" "hamming abc abc" "hamming abc abcd" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hamming:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "hamming: karolin vs kathrin = 3",
    "hamming: 1011101 vs 1001001 = 2",
    "hamming: abc vs abc = 0",
    "hamming: length mismatch (3 vs 4)",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"HAMMING TEST FAILED (missing: {missing})"); sys.exit(1)
print("HAMMING TEST PASSED  (karolin/kathrin=3, bitstrings=2, identical=0, length mismatch reported)")
PY
