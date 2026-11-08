#!/usr/bin/env bash
# Verify the `sort` command lexicographically (byte-order) sorts its arguments.
# Deterministic, so exact-line assertions prove it. The second case deliberately
# demonstrates that it is a STRING sort, not numeric ("10" < "2"):
#   sort banana apple cherry -> "sorted: apple banana cherry"
#   sort 3 1 2 10            -> "sorted: 1 10 2 3"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4698,server,nowait \
    -qmp tcp:127.0.0.1:4699,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/sort.out"
python3 scripts/shell-demo.py 4698 4699 "${TMPDIR:-/tmp}/sort.png" "sort banana apple cherry" "sort 3 1 2 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sorted: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["sorted: apple banana cherry", "sorted: 1 10 2 3"]
missing = [w for w in want if w not in s]
if missing:
    print(f"SORT TEST FAILED (missing exact line(s): {missing})"); sys.exit(1)
print("SORT TEST PASSED  (lexicographic: apple banana cherry; 1 10 2 3)")
PY
