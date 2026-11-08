#!/usr/bin/env bash
# Verify the `bytesum <file>` command reports the plain sum of all byte values in
# a VFS file, plus the byte count. The simplest additive checksum (Sigma of byte
# values). DISTINCT from crc32 (polynomial CRC), fletcher16 (positional running
# checksum), and histogram (per-byte frequency). Self-checking: python computes
# sum(content.encode()) and len(content.encode()) from the KNOWN seed constants
# (about.txt = line1 + '\n' + line2 = 128 bytes, sum 11376; motd = 63 bytes).
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
OUT="${TMPDIR:-/tmp}/bytesum.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bytesum.png" \
    "bytesum about.txt" "bytesum motd" "bytesum nope.txt" "bytesum" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bytesum:|usage: bytesum" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# The seeded VFS constants (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
motd  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

for name, text in (("about.txt", about), ("motd", motd)):
    b = text.encode()
    want = f"bytesum: {sum(b)} over {len(b)} bytes"
    if want not in s:
        print(f"BYTESUM TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# sanity-pin: about.txt is 128 bytes (matches wc's byte total) and sums to 11376
assert len(about.encode()) == 128 and sum(about.encode()) == 11376
assert len(motd.encode()) == 63 and sum(motd.encode()) == 5638

if "bytesum: no such file: nope.txt" not in s:
    print("BYTESUM TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: bytesum <file>" not in s:
    print("BYTESUM TEST FAILED (missing usage line)"); sys.exit(1)

print("BYTESUM TEST PASSED  (byte sums + counts exact vs seed constants, no-file/usage)")
PY
