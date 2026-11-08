#!/usr/bin/env bash
# Verify the CRC-32 implementation against the canonical check vector: the CRC-32
# (IEEE, reflected, poly 0xEDB88320) of the ASCII string "123456789" is exactly
# 0xCBF43926. Writing that string and running `crc32` must produce it -- a
# deterministic, standard-defined correctness check.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4670,server,nowait \
    -qmp tcp:127.0.0.1:4671,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/crc32.out"
python3 scripts/shell-demo.py 4670 4671 "${TMPDIR:-/tmp}/crc32.png" \
    "write crctest.txt 123456789" "crc32 crctest.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "crc32 crctest" "$OUT" | sed 's/^/  /'

if grep -aiq "crc32 crctest.txt: 0xcbf43926" "$OUT"; then
    echo "CRC32 TEST PASSED  (CRC-32 of \"123456789\" = 0xcbf43926, the canonical check value)"
else
    echo "CRC32 TEST FAILED (expected 0xcbf43926)"; exit 1
fi
