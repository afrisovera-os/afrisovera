#!/usr/bin/env bash
# Verify the `fletcher16` command computes the Fletcher-16 checksum of a VFS file
# (a checksum distinct from crc32's IEEE CRC-32). The seeded about.txt (fixed bytes
# compiled into vfs.rs) has a deterministic Fletcher-16; the test recomputes the
# expected value in python from the exact seed bytes for a self-checking assert.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4838,server,nowait \
    -qmp tcp:127.0.0.1:4839,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/fletcher16.out"
python3 scripts/shell-demo.py 4838 4839 "${TMPDIR:-/tmp}/fletcher16.png" \
    "fletcher16 about.txt" "fletcher16 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fletcher16:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def f16(data):
    s1 = s2 = 0
    for b in data:
        s1 = (s1 + b) % 255
        s2 = (s2 + s1) % 255
    return (s2 << 8) | s1

# The exact about.txt seed bytes from kernel/src/vfs.rs (one embedded '\n', no trailing NL).
about = b"AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\nAI as an OS service. You own the device and the software on it."
expect = f"fletcher16: about.txt = 0x{f16(about):04x} ({len(about)} bytes)"

if expect not in s:
    print(f"FLETCHER16 TEST FAILED (expected '{expect}')"); sys.exit(1)
if "fletcher16: nope.txt not found" not in s:
    print("FLETCHER16 TEST FAILED (missing not-found path)"); sys.exit(1)

print(f"FLETCHER16 TEST PASSED  ({expect}; missing file reported)")
PY
