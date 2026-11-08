#!/usr/bin/env bash
# Verify WASM globals: a module declares a mutable i32 global initialized to 10,
# then does global.get + i32.const 5 + i32.add + global.set + global.get + print,
# yielding 15. Proves the global section (init expr) plus global.get/global.set --
# mutable module state that real compiled WASM relies on. Deterministic.
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
OUT="${TMPDIR:-/tmp}/wasm-glob.out"
python3 scripts/shell-demo.py 4650 4651 "${TMPDIR:-/tmp}/wasm-glob.png" "wasmglob" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "glob.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print(15)" "$OUT" \
    && echo "WASM-GLOB TEST PASSED  (mutable global 10 += 5 = 15 via global.get/global.set)" \
    || { echo "WASM-GLOB TEST FAILED (expected print(15))"; exit 1; }
