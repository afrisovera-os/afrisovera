#!/usr/bin/env bash
# Verify the WASM dice app: `dice 6` rolls a 1..6 die six times via the
# capability-gated RNG (host random() + the interpreter's new i32.rem_u opcode).
# Asserts six print() results, all within 1..6. RAM-only. Needs python3.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4612,server,nowait \
    -qmp tcp:127.0.0.1:4613,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/dice-test.out"
python3 scripts/shell-demo.py 4612 4613 "${TMPDIR:-/tmp}/dice-test.png" "dice 6" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aiE "dice.wasm|print\(" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
ser = open(sys.argv[1], "rb").read().decode("latin1")
rolls = [int(m) for m in re.findall(r"print\((\d+)\)", ser)]
if len(rolls) < 6:
    print(f"DICE TEST FAILED (expected >=6 rolls, got {len(rolls)}: {rolls})"); sys.exit(1)
bad = [r for r in rolls if r < 1 or r > 6]
if bad:
    print(f"DICE TEST FAILED (rolls out of 1..6 range: {bad})"); sys.exit(1)
print(f"  rolls: {rolls} (all in 1..6)")
print("DICE TEST PASSED  (WASM dice app rolled valid 1..6 values via CAP_RANDOM + i32.rem_u)")
PY
