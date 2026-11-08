#!/usr/bin/env bash
# Verify the `count` command counts non-overlapping needle occurrences.
# Deterministic:
#   count na banana  -> "na" at index 2 and 4 (non-overlapping) = 2
#   count a banana   -> "a" at 1,3,5 = 3
#   count xyz banana -> 0
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4740,server,nowait \
    -qmp tcp:127.0.0.1:4741,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/count.out"
python3 scripts/shell-demo.py 4740 4741 "${TMPDIR:-/tmp}/count.png" "count na banana" "count a banana" "count xyz banana" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "count: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["count: 2 occurrence", "count: 3 occurrence", "count: 0 occurrence"]
missing = [w for w in want if w not in s]
if missing:
    print(f"COUNT TEST FAILED (missing: {missing})"); sys.exit(1)
print("COUNT TEST PASSED  (na/banana=2; a/banana=3; xyz/banana=0)")
PY
