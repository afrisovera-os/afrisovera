#!/usr/bin/env bash
# Verify the `tail` command prints the last N bytes of a VFS file. The test seeds
# a known file first, so the output is deterministic:
#   write tf.txt abcdefgh  (8 bytes)
#   tail 3 tf.txt   -> "fgh"       (last 3 bytes)
#   tail 100 tf.txt -> "abcdefgh"  (N > size -> whole file)
#   tail 3 nope.txt -> "not found"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4780,server,nowait \
    -qmp tcp:127.0.0.1:4781,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/tail.out"
python3 scripts/shell-demo.py 4780 4781 "${TMPDIR:-/tmp}/tail.png" "write tf.txt abcdefgh" "tail 3 tf.txt" "tail 100 tf.txt" "tail 3 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tail: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["tail: fgh", "tail: abcdefgh", "tail: nope.txt not found"]
missing = [w for w in want if w not in s]
if missing:
    print(f"TAIL TEST FAILED (missing: {missing})"); sys.exit(1)
print("TAIL TEST PASSED  (last 3 -> fgh; N>size -> abcdefgh; missing file handled)")
PY
