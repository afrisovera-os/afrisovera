#!/usr/bin/env bash
# Verify `touch`: touching a new file creates it empty (0 bytes) and stamps its
# mtime from the RTC, so `stat` shows a real 20xx date (not "unknown"). Proves
# touch both creates and timestamps. Single-line assertion on the stat line.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4668,server,nowait \
    -qmp tcp:127.0.0.1:4669,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/touch.out"
python3 scripts/shell-demo.py 4668 4669 "${TMPDIR:-/tmp}/touch.png" \
    "touch tchtest.txt" "stat tchtest.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "touched|stat tchtest" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"stat tchtest\.txt: (\d+) bytes, modified (20\d\d)-\d\d-\d\d", s)
if not m:
    if "modified unknown" in s:
        print("TOUCH TEST FAILED (touch created the file but did not stamp its mtime)")
    else:
        print("TOUCH TEST FAILED (no stat line with a date for tchtest.txt)")
    sys.exit(1)
size, year = int(m.group(1)), int(m.group(2))
if size != 0:
    print(f"TOUCH TEST FAILED (expected an empty file, got {size} bytes)"); sys.exit(1)
if not (2025 <= year <= 2100):
    print(f"TOUCH TEST FAILED (mtime year {year} out of range)"); sys.exit(1)
print(f"TOUCH TEST PASSED  (touch created tchtest.txt empty and stamped mtime {m.group(2)}-...)")
PY
