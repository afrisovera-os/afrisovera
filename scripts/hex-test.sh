#!/usr/bin/env bash
# Verify the `hex` command encodes an inline string to lowercase hex bytes (the
# inverse of unhex): Hi -> 4869, OS! -> 4f5321, AfriSovera -> 41667269536f76657261.
# Also checks the round-trip: unhex(hex(Hi)) == Hi. Test recomputes hex in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4950,server,nowait \
    -qmp tcp:127.0.0.1:4951,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/hex.out"
python3 scripts/shell-demo.py 4950 4951 "${TMPDIR:-/tmp}/hex.png" \
    "hex Hi" "hex OS!" "hex AfriSovera" "unhex 4869" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hex:|unhex:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

for text in ["Hi", "OS!", "AfriSovera"]:
    want = f"hex: {text.encode().hex()}"
    if want not in s:
        print(f"HEX TEST FAILED (expected '{want}' for {text!r})"); sys.exit(1)

# Round-trip via the existing unhex command: unhex(4869) must yield Hi.
if "unhex: Hi" not in s:
    print("HEX TEST FAILED (hex/unhex round-trip broken)"); sys.exit(1)

print("HEX TEST PASSED  (Hi->4869, OS!->4f5321, AfriSovera->...; unhex round-trip -> Hi)")
PY
