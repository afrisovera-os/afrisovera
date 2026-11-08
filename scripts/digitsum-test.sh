#!/usr/bin/env bash
# Verify the `digitsum` command computes the decimal digit sum + digital root.
# Deterministic:
#   digitsum 12345 -> sum 1+2+3+4+5 = 15, root 1+5 = 6
#   digitsum 99999 -> sum 45, root 9
#   digitsum 0     -> sum 0, root 0
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4762,server,nowait \
    -qmp tcp:127.0.0.1:4763,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/digitsum.out"
python3 scripts/shell-demo.py 4762 4763 "${TMPDIR:-/tmp}/digitsum.png" "digitsum 12345" "digitsum 99999" "digitsum 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "digitsum: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "digitsum: 12345 -> digit sum 15, digital root 6",
    "digitsum: 99999 -> digit sum 45, digital root 9",
    "digitsum: 0 -> digit sum 0, digital root 0",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"DIGITSUM TEST FAILED (missing: {missing})"); sys.exit(1)
print("DIGITSUM TEST PASSED  (12345->15/6; 99999->45/9; 0->0/0)")
PY
