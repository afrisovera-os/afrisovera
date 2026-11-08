#!/usr/bin/env bash
# Verify i64 locals: a module stashes 0x2_0000_0005 (8589934597) in an i64 local
# via local.set, reads it back via local.get, and prints it through print64. The
# value exceeds 32 bits, so a correct result proves locals no longer truncate to
# 32 bits. Deterministic.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4636,server,nowait \
    -qmp tcp:127.0.0.1:4637,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/wasm-i64loc.out"
python3 scripts/shell-demo.py 4636 4637 "${TMPDIR:-/tmp}/wasm-i64loc.png" "wasmi64loc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "i64loc.wasm|print64|returned cleanly" "$OUT" | sed 's/^/  /'

grep -aq "print64(8589934597)" "$OUT" \
    && echo "WASM-I64LOC TEST PASSED  (8589934597 survived in an i64 local via local.set + local.get)" \
    || { echo "WASM-I64LOC TEST FAILED (expected print64(8589934597))"; exit 1; }
