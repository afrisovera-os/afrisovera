#!/usr/bin/env bash
# Verify the `df` filesystem-usage summary: at a fresh boot the VFS is seeded with
# 2 files (motd, about.txt) with content, so `df` must report >= 2 files, a
# non-zero bytes-used, and the fixed capacity. Single-line assertion.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4676,server,nowait \
    -qmp tcp:127.0.0.1:4677,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/df.out"
python3 scripts/shell-demo.py 4676 4677 "${TMPDIR:-/tmp}/df.png" "df" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "^df:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")
m = re.search(r"df: (\d+)/24 files, (\d+) bytes used of (\d+) \((\d+) free\), disk (\w+)", s)
if not m:
    print("DF TEST FAILED (no df summary line in the expected form)"); sys.exit(1)
files, used, cap, free, disk = int(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4)), m.group(5)
if files < 2:
    print(f"DF TEST FAILED (expected >=2 seeded files, got {files})"); sys.exit(1)
if used <= 0:
    print(f"DF TEST FAILED (bytes used {used} -- seeded files have content)"); sys.exit(1)
if used + free != cap:
    print(f"DF TEST FAILED (used {used} + free {free} != capacity {cap})"); sys.exit(1)
print(f"DF TEST PASSED  ({files}/24 files, {used} bytes used of {cap}, disk {disk})")
PY
