#!/usr/bin/env bash
# Verify the `hexdump` file inspector: write a file with known bytes and confirm
# hexdump shows the expected hex ("41 42 43") and ASCII gutter ("ABC"). RAM-only.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4650,server,nowait \
    -qmp tcp:127.0.0.1:4651,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/hexdump.out"
python3 scripts/shell-demo.py 4650 4651 "${TMPDIR:-/tmp}/hexdump.png" \
    "write h ABC" "hexdump h" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "0000|hexdump" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
ok_hex = "41 42 43" in s
ok_ascii = "|ABC|" in s
if ok_hex and ok_ascii:
    print("HEXDUMP TEST PASSED  (hex 41 42 43 + ASCII |ABC| shown)")
    sys.exit(0)
print(f"HEXDUMP TEST FAILED (hex={ok_hex} ascii={ok_ascii})")
sys.exit(1)
PY
