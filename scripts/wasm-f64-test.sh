#!/usr/bin/env bash
# Verify f64 (double) WASM ops: a module computes (10.0 / 4.0) * 2.0 = 5.0 with
# f64.const/f64.div/f64.mul, truncates to i32 via i32.trunc_f64_s, and prints 5.
# The 2.5 intermediate from 10.0/4.0 cannot come from integer division, so a
# correct 5 proves real IEEE-754 floating-point arithmetic. Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4644,server,nowait \
    -qmp tcp:127.0.0.1:4645,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-f64.out"
python3 scripts/shell-demo.py 4644 4645 "${TMPDIR:-/tmp}/wasm-f64.png" "wasmf64" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "f64.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print(5)" "$OUT" \
    && echo "WASM-F64 TEST PASSED  ((10.0 / 4.0) * 2.0 = 5.0 via IEEE-754 f64.div + f64.mul)" \
    || { echo "WASM-F64 TEST FAILED (expected print(5))"; exit 1; }
