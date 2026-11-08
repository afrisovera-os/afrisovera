#!/usr/bin/env bash
# WASM parser DoS defense: feed the interpreter malformed modules (truncated
# sections, oversized names, code with no end, etc.) via `wasmfuzz` and confirm
# the kernel SURVIVES - no panic - and the shell is still responsive afterward
# (a follow-up `mem` must run). RAM-only. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4622,server,nowait \
    -qmp tcp:127.0.0.1:4623,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-fuzz.out"
python3 scripts/shell-demo.py 4622 4623 "${TMPDIR:-/tmp}/wasm-fuzz.png" "wasmfuzz" "mem" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "fuzz|parse error|TRAP|no exported|survived|heap:" "$OUT" | sed 's/^/  /' | head -20

grep -aq "KERNEL PANIC" "$OUT" \
    && { echo "WASM-FUZZ TEST FAILED (a malformed module PANICKED the kernel)"; exit 1; }
grep -aq "fuzz] survived" "$OUT" \
    || { echo "WASM-FUZZ TEST FAILED (fuzz run did not complete)"; exit 1; }
grep -aq "heap:" "$OUT" \
    || { echo "WASM-FUZZ TEST FAILED (shell unresponsive after malformed modules)"; exit 1; }
echo "WASM-FUZZ TEST PASSED  (malformed modules rejected; no panic; shell still alive)"
