#!/usr/bin/env bash
# Verify WASM memory.size: a module declaring (memory 1) calls memory.size and
# prints the result -- which must be 1 (the interpreter's linear memory is one
# 64KiB page). Proves the memory.size opcode + page accounting. Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4672,server,nowait \
    -qmp tcp:127.0.0.1:4673,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-memsz.out"
python3 scripts/shell-demo.py 4672 4673 "${TMPDIR:-/tmp}/wasm-memsz.png" "wasmmemsz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "memsz.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print(1)" "$OUT" \
    && echo "WASM-MEMSZ TEST PASSED  (memory.size = 1 page via the memory.size opcode)" \
    || { echo "WASM-MEMSZ TEST FAILED (expected print(1))"; exit 1; }
