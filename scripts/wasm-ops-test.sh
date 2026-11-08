#!/usr/bin/env bash
# Verify the new WASM shift opcodes: a module computing (5 << 2) + (100 >> 3)
# via i32.shl + i32.shr_u must print 32. Deterministic (pure arithmetic).
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4630,server,nowait \
    -qmp tcp:127.0.0.1:4631,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-ops.out"
python3 scripts/shell-demo.py 4630 4631 "${TMPDIR:-/tmp}/wasm-ops.png" "wasmops" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "ops.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print(32)" "$OUT" \
    && echo "WASM-OPS TEST PASSED  ((5<<2)+(100>>3) = 32 via i32.shl + i32.shr_u)" \
    || { echo "WASM-OPS TEST FAILED (expected print(32))"; exit 1; }
