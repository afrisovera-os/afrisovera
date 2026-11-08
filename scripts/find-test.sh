#!/usr/bin/env bash
# Verify the `find <substr>` filename search: after writing a uniquely-named file,
# `find zqfind` must list it (and NOT report "no files matching"). The unique name
# avoids collisions with seeded files. Single distinctive-line assertion.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4666,server,nowait \
    -qmp tcp:127.0.0.1:4667,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/find.out"
python3 scripts/shell-demo.py 4666 4667 "${TMPDIR:-/tmp}/find.png" \
    "write zqfind.txt x" "find zqfind" "uptime" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "zqfind|no files matching" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
if "no files matching" in s:
    print("FIND TEST FAILED (find reported no match for zqfind)"); sys.exit(1)
# zqfind.txt should appear at least twice: the write echo + the find result line
if s.count("zqfind.txt") < 2:
    print(f"FIND TEST FAILED (zqfind.txt seen {s.count('zqfind.txt')}x; find did not list it)"); sys.exit(1)
print("FIND TEST PASSED  (find zqfind listed zqfind.txt)")
PY
