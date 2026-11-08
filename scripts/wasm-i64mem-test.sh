#!/usr/bin/env bash
# Verify the new i64 memory ops: a module stores 0x1_0000_002A (4294967338) to
# linear memory via i64.store and loads it back via i64.load, then prints it
# through print64. A correct result proves 64-bit values round-trip through the
# sandbox's linear memory (the value exceeds 32 bits). Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4634,server,nowait \
    -qmp tcp:127.0.0.1:4635,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-i64mem.out"
python3 scripts/shell-demo.py 4634 4635 "${TMPDIR:-/tmp}/wasm-i64mem.png" "wasmi64mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "i64mem.wasm|print64|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print64(4294967338)" "$OUT" \
    && echo "WASM-I64MEM TEST PASSED  (4294967338 round-tripped through linear memory via i64.store + i64.load)" \
    || { echo "WASM-I64MEM TEST FAILED (expected print64(4294967338))"; exit 1; }
