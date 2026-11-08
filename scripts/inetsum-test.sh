#!/usr/bin/env bash
# Verify the `inetsum <file>` command computes the 16-bit one's-complement
# Internet checksum (RFC 1071) of a VFS file's bytes -- the checksum used in
# IP/TCP/UDP headers. DISTINCT from bytesum (plain u64 sum), lrc (XOR),
# fletcher16 (positional), crc32 (polynomial). Self-checking: python implements
# the SAME RFC-1071 algorithm from the KNOWN seed constants -- about.txt (128
# bytes, EVEN -> 0x80e2) and motd (63 bytes, ODD -> 0x41a2, exercises padding).
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
OUT="${TMPDIR:-/tmp}/inetsum.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/inetsum.png" \
    "inetsum about.txt" "inetsum motd" "inetsum nope.txt" "inetsum" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "inetsum:|usage: inetsum" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys

# The seeded VFS constants (must match kernel/src/vfs.rs).
about = ("AfriSovera: one codebase, WASM-native apps, no gatekeeper store,\n"
         "AI as an OS service. You own the device and the software on it.")
motd  = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def inet(bs):  # RFC 1071
    s = 0
    for i in range(0, len(bs) - 1, 2):
        s += (bs[i] << 8) | bs[i + 1]
    if len(bs) & 1:
        s += bs[-1] << 8       # odd trailing byte high-padded
    while s >> 16:
        s = (s & 0xffff) + (s >> 16)
    return (~s) & 0xffff

s = open(sys.argv[1], "rb").read().decode("latin1")

for name, text in (("about.txt", about), ("motd", motd)):
    b = text.encode()
    # match the kernel's {:#06x}: lowercase, 0x prefix, 4 hex digits
    want = f"inetsum: 0x{inet(b):04x} over {len(b)} bytes"
    if want not in s:
        print(f"INETSUM TEST FAILED ({name}: missing '{want}')"); sys.exit(1)

# sanity-pin: about (even, 128 bytes) and motd (odd, 63 bytes -> padding path)
assert inet(about.encode()) == 0x80e2 and len(about.encode()) == 128
assert inet(motd.encode()) == 0x41a2 and len(motd.encode()) == 63

if "inetsum: no such file: nope.txt" not in s:
    print("INETSUM TEST FAILED (missing no-such-file line)"); sys.exit(1)
if "usage: inetsum <file>" not in s:
    print("INETSUM TEST FAILED (missing usage line)"); sys.exit(1)

print("INETSUM TEST PASSED  (RFC-1071 checksums exact vs seed constants incl. odd-byte padding, no-file/usage)")
PY
