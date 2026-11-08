#!/usr/bin/env bash
# Verify the `rle` command run-length encodes text as <char><count> pairs:
# aaabbc -> a3b2c1; hello -> h1e1l2o1; and aaaaaaaaaabbb -> a10b3 (exercises the
# multi-digit run count). Distinct from `squeeze` (which just collapses runs).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4860,server,nowait \
    -qmp tcp:127.0.0.1:4861,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/rle.out"
python3 scripts/shell-demo.py 4860 4861 "${TMPDIR:-/tmp}/rle.png" \
    "rle aaabbc" "rle hello" "rle aaaaaaaaaabbb" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rle:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "rle: a3b2c1",
    "rle: h1e1l2o1",
    "rle: a10b3",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"RLE TEST FAILED (missing: {missing})"); sys.exit(1)
print("RLE TEST PASSED  (aaabbc->a3b2c1, hello->h1e1l2o1, a*10 b*3 -> a10b3 multi-digit)")
PY
