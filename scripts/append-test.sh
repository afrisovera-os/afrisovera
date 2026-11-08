#!/usr/bin/env bash
# Verify the `append` command: write "AB" then append "CD" -> the file holds
# "ABCD" (append concatenates, unlike write which overwrites). Confirms via cat.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4690,server,nowait \
    -qmp tcp:127.0.0.1:4691,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/append.out"
python3 scripts/shell-demo.py 4690 4691 "${TMPDIR:-/tmp}/append.png" \
    "write aptest.txt AB" "append aptest.txt CD" "cat aptest.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "appended|aptest|ABCD" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
if "appended 2 bytes to aptest.txt (4 total)" not in s:
    print("APPEND TEST FAILED (append did not report 2 bytes / 4 total)"); sys.exit(1)
# cat output after the append must show ABCD on its own line
if "ABCD" not in s.split("cat aptest.txt", 1)[-1]:
    print("APPEND TEST FAILED (cat did not read back ABCD)"); sys.exit(1)
print("APPEND TEST PASSED  (write AB + append CD -> ABCD)")
PY
