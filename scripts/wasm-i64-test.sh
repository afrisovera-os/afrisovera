#!/usr/bin/env bash
# Verify the new i64 WASM opcodes: a module computing (2^32 * 3) + 7 via
# i64.const / i64.mul / i64.add must print 12884901895 through the print64 host.
# The result does NOT fit in 32 bits, so a correct answer proves real 64-bit
# integer math in the sandboxed interpreter. Deterministic (pure arithmetic).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4632,server,nowait \
    -qmp tcp:127.0.0.1:4633,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-i64.out"
python3 scripts/shell-demo.py 4632 4633 "${TMPDIR:-/tmp}/wasm-i64.png" "wasmi64" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "i64.wasm|print64|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print64(12884901895)" "$OUT" \
    && echo "WASM-I64 TEST PASSED  ((2^32 * 3) + 7 = 12884901895 via i64.mul + i64.add)" \
    || { echo "WASM-I64 TEST FAILED (expected print64(12884901895))"; exit 1; }
