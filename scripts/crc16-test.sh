#!/usr/bin/env bash
# Verify the `crc16` command computes CRC-16/CCITT-FALSE. The canonical check
# value for this variant (poly 0x1021, init 0xFFFF, non-reflected) is the ASCII
# string "123456789" -> 0x29B1 -- a definitive, standard test vector.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4722,server,nowait \
    -qmp tcp:127.0.0.1:4723,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/crc16.out"
python3 scripts/shell-demo.py 4722 4723 "${TMPDIR:-/tmp}/crc16.png" "crc16 123456789" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "crc16: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
if "crc16: 0x29b1" not in s:
    print("CRC16 TEST FAILED (expected the canonical CCITT-FALSE vector crc16: 0x29b1 for '123456789')"); sys.exit(1)
print("CRC16 TEST PASSED  (CCITT-FALSE '123456789' -> 0x29b1)")
PY
