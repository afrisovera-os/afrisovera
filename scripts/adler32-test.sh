#!/usr/bin/env bash
# Verify the `adler32` command computes the Adler-32 checksum (zlib). Two
# definitive, reference-checkable vectors:
#   "Wikipedia" -> 0x11E60398   (the classic Adler-32 example)
#   "a"         -> 0x00620062   (a=1+97=98=0x62, b=98)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4724,server,nowait \
    -qmp tcp:127.0.0.1:4725,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/adler32.out"
python3 scripts/shell-demo.py 4724 4725 "${TMPDIR:-/tmp}/adler32.png" "adler32 Wikipedia" "adler32 a" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "adler32: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["adler32: 0x11e60398", "adler32: 0x00620062"]
missing = [w for w in want if w not in s]
if missing:
    print(f"ADLER32 TEST FAILED (missing: {missing})"); sys.exit(1)
print("ADLER32 TEST PASSED  (Wikipedia -> 0x11e60398; a -> 0x00620062)")
PY
