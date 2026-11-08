#!/usr/bin/env bash
# Verify WASM br_table (the jump-table branch / switch): a module switches on
# index 1 across two nested blocks. Index 1 must break to the OUTER block, skipping
# print(10) and reaching print(20). Asserts print(20) AND not print(10) -- proving
# the index selected the correct branch target. Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4688,server,nowait \
    -qmp tcp:127.0.0.1:4689,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-brtable.out"
python3 scripts/shell-demo.py 4688 4689 "${TMPDIR:-/tmp}/wasm-brtable.png" "wasmbrtable" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "brtable.wasm|print\(|returned cleanly" "$OUT" | sed 's/^/  /'

if grep -aq "print(20)" "$OUT" && ! grep -aq "print(10)" "$OUT"; then
    echo "WASM-BRTABLE TEST PASSED  (br_table index 1 -> outer block -> print(20), not print(10))"
else
    echo "WASM-BRTABLE TEST FAILED (expected print(20) and NOT print(10))"; exit 1
fi
