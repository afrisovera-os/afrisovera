#!/usr/bin/env bash
# Verify f32 (single-precision) WASM ops: a module computes (7.0 / 2.0) * 4.0 =
# 14.0 with f32.const/f32.div/f32.mul, truncates to i32 via i32.trunc_f32_s, and
# prints 14. The 3.5 intermediate from 7.0/2.0 cannot come from integer division,
# so a correct 14 proves real IEEE-754 single-precision arithmetic. Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4648,server,nowait \
    -qmp tcp:127.0.0.1:4649,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-f32.out"
python3 scripts/shell-demo.py 4648 4649 "${TMPDIR:-/tmp}/wasm-f32.png" "wasmf32" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "f32.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print(14)" "$OUT" \
    && echo "WASM-F32 TEST PASSED  ((7.0 / 2.0) * 4.0 = 14.0 via IEEE-754 f32.div + f32.mul)" \
    || { echo "WASM-F32 TEST FAILED (expected print(14))"; exit 1; }
