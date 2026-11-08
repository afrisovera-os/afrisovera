#!/usr/bin/env bash
# Verify the `lrc <file>` command reports the XOR of all byte values in a VFS
# file (an 8-bit Longitudinal Redundancy Check), plus the byte count. DISTINCT
# from bytesum (additive SUM), crc32 (polynomial CRC), and fletcher16 (positional
# checksum) -- lrc XORs the bytes. Self-checking: python computes
# reduce(xor, content.encode(), 0) from the KNOWN seed constants (about.txt = 128
# bytes -> 0x12; motd = 63 bytes -> 0x14) and asserts the exact `0x%02x` output.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4998,server,nowait \
    -qmp tcp:127.0.0.1:4999,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/lrc.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lrc.png" \
    "lrc about.txt" "lrc motd" "lrc nope.txt" "lrc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lrc:|usage: lrc" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, operator
from functools import reduce

# The seeded VFS constants (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
motd  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

s = open(sys.argv[1], "rb").read().decode("latin1")

for name, text in (("about.txt", about), ("motd", motd)):
    b = text.encode()
    x = reduce(operator.xor, b, 0)
    # match the kernel's {:#04x}: lowercase, 0x prefix, 2 hex digits (0x0a..0xff)
    want = f"lrc: 0x{x:02x} over {len(b)} bytes"
    if want not in s:
        print(f"LRC TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# sanity-pin the XOR checksums (distinct from bytesum's additive sums)
assert reduce(operator.xor, about.encode(), 0) == 0x12 and len(about.encode()) == 128
assert reduce(operator.xor, motd.encode(), 0) == 0x14 and len(motd.encode()) == 63

if "lrc: no such file: nope.txt" not in s:
    print("LRC TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: lrc <file>" not in s:
    print("LRC TEST FAILED (missing usage line)"); sys.exit(1)

print("LRC TEST PASSED  (XOR checksums + counts exact vs seed constants, no-file/usage)")
PY
