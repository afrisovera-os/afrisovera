#!/usr/bin/env bash
# Verify `ls -l` long-format listing: writing a uniquely-named file (RTC-stamped),
# then `ls -l` must show a line for that file with its size (2 bytes) and a
# formatted last-modified date (a 20xx year). Greps the full capture for THAT
# file's line (robust vs multi-line truncation).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4664,server,nowait \
    -qmp tcp:127.0.0.1:4665,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/ls-l.out"
python3 scripts/shell-demo.py 4664 4665 "${TMPDIR:-/tmp}/ls-l.png" \
    "write lsltest.txt hi" "ls -l" "uptime" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "lsltest" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
# find the ls -l line for lsltest.txt: "     2  20xx-..-.. ..:..:..  lsltest.txt"
line = None
for ln in s.splitlines():
    if "lsltest.txt" in ln and re.search(r"20\d\d-\d\d-\d\d", ln):
        line = ln.strip(); break
if not line:
    print("LS-L TEST FAILED (no long-listing line for lsltest.txt with a date)"); sys.exit(1)
m = re.match(r"(\d+)\s+(20\d\d)-\d\d-\d\d \d\d:\d\d:\d\d\s+lsltest\.txt", line)
if not m:
    print(f"LS-L TEST FAILED (line not in 'size date name' form: {line!r})"); sys.exit(1)
size, year = int(m.group(1)), int(m.group(2))
if size != 2:
    print(f"LS-L TEST FAILED (expected size 2, got {size})"); sys.exit(1)
if not (2025 <= year <= 2100):
    print(f"LS-L TEST FAILED (mtime year {year} out of range)"); sys.exit(1)
print(f"LS-L TEST PASSED  (ls -l shows lsltest.txt: {size} bytes, mtime {m.group(2)}-... UTC)")
PY
