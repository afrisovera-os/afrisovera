#!/usr/bin/env bash
# Verify file timestamps: writing a file stamps it with the RTC epoch, and `stat`
# reports the size + a formatted UTC last-modified time. Asserts the single stat
# line shows a non-zero size and a sane year (2025..2100), proving the RTC stamp
# on the write path + the epoch->calendar formatting. Deterministic-ish (wall clock).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4654,server,nowait \
    -qmp tcp:127.0.0.1:4655,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/stat.out"
python3 scripts/shell-demo.py 4654 4655 "${TMPDIR:-/tmp}/stat.png" \
    "write stamped.txt hello" "stat stamped.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "stat stamped|modified" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"stat stamped\.txt: (\d+) bytes, modified (\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2}) UTC", s)
if not m:
    print("STAT TEST FAILED (no stat line with a modified timestamp)"); sys.exit(1)
size = int(m.group(1)); year = int(m.group(2))
if size < 1:
    print(f"STAT TEST FAILED (size {size} < 1)"); sys.exit(1)
if not (2025 <= year <= 2100):
    print(f"STAT TEST FAILED (mtime year {year} out of sane range)"); sys.exit(1)
print(f"STAT TEST PASSED  (stamped.txt = {size} bytes, RTC-stamped mtime {m.group(0).split('modified ')[1]})")
PY
