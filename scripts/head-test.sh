#!/usr/bin/env bash
# Verify the `head` command prints the first N bytes of a VFS file (mirror of
# tail). Deterministic via a seeded file:
#   write hf.txt abcdefgh  (8 bytes)
#   head 3 hf.txt   -> "abc"       (first 3 bytes)
#   head 100 hf.txt -> "abcdefgh"  (N > size -> whole file)
#   head 3 nope.txt -> "not found"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4788,server,nowait \
    -qmp tcp:127.0.0.1:4789,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/head.out"
python3 scripts/shell-demo.py 4788 4789 "${TMPDIR:-/tmp}/head.png" "write hf.txt abcdefgh" "head 3 hf.txt" "head 100 hf.txt" "head 3 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "head: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["head: abc", "head: abcdefgh", "head: nope.txt not found"]
missing = [w for w in want if w not in s]
if missing:
    print(f"HEAD TEST FAILED (missing: {missing})"); sys.exit(1)
print("HEAD TEST PASSED  (first 3 -> abc; N>size -> abcdefgh; missing file handled)")
PY
