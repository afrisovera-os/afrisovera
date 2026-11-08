#!/usr/bin/env bash
# Verify the `wc <file>` command. Writes "hello world" (the shell `write` stores
# the trimmed content verbatim -> 11 bytes, 2 whitespace-delimited words, 0
# newlines), then `wc` must report those exact counts. Single-line assertion.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4658,server,nowait \
    -qmp tcp:127.0.0.1:4659,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wc.out"
python3 scripts/shell-demo.py 4658 4659 "${TMPDIR:-/tmp}/wc.png" \
    "write wctest.txt hello world" "wc wctest.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "wc wctest" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"wc wctest\.txt: (\d+) lines, (\d+) words, (\d+) bytes", s)
if not m:
    print("WC TEST FAILED (no wc output line)"); sys.exit(1)
lines, words, bytes_ = (int(x) for x in m.groups())
if (lines, words, bytes_) != (0, 2, 11):
    print(f"WC TEST FAILED (expected 0 lines, 2 words, 11 bytes; got {lines}, {words}, {bytes_})"); sys.exit(1)
print("WC TEST PASSED  (\"hello world\" = 0 lines, 2 words, 11 bytes)")
PY
