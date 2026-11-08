#!/usr/bin/env bash
# Verify the `atbash` command applies the Atbash substitution cipher (A<->Z,
# case-preserving, self-inverse): HELLO -> SVOOL; abc -> zyx; and SVOOL -> HELLO
# (the round-trip proves self-inverse). Non-letters pass through unchanged.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4832,server,nowait \
    -qmp tcp:127.0.0.1:4833,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/atbash.out"
python3 scripts/shell-demo.py 4832 4833 "${TMPDIR:-/tmp}/atbash.png" \
    "atbash HELLO" "atbash abc" "atbash SVOOL" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "atbash:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "atbash: SVOOL",
    "atbash: zyx",
    "atbash: HELLO",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"ATBASH TEST FAILED (missing: {missing})"); sys.exit(1)
print("ATBASH TEST PASSED  (HELLO<->SVOOL round-trip, abc->zyx; self-inverse)")
PY
