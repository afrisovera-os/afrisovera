#!/usr/bin/env bash
# Verify the `histogram` command computes the byte-frequency profile of a VFS
# file -- a filesystem-backed milestone. The test seeds a known file first, so
# the profile is deterministic:
#   write histg.txt aaabbc  (6 bytes: a x3, b x2, c x1)
#   histogram histg.txt -> "6 bytes, 3 distinct values", most common 0x61 ('a') x 3
#   histogram nope.txt  -> "not found"
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
OUT="${TMPDIR:-/tmp}/histogram.out"
python3 scripts/shell-demo.py 4766 4767 "${TMPDIR:-/tmp}/histogram.png" "write histg.txt aaabbc" "histogram histg.txt" "histogram nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "histogram: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "histogram: histg.txt = 6 bytes, 3 distinct values",
    "histogram: most common byte 0x61 ('a') x 3",
    "histogram: nope.txt not found",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"HISTOGRAM TEST FAILED (missing: {missing})"); sys.exit(1)
print("HISTOGRAM TEST PASSED  (aaabbc -> 6 bytes/3 distinct/most-common 'a' x3; missing file handled)")
PY
