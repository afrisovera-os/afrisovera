#!/usr/bin/env bash
# Verify the `unbits <binary>` command decodes space-separated 8-bit binary
# groups back to text -- the INVERSE of `bits` (2.278). e.g.
# "01001000 01101001" -> "Hi". A bad group (non-binary digit) reports an error
# and stops. Completes the encode/decode pair like unhex/hex, demorse/morse.
# Self-checking: python decodes via int(t,2) and also checks the bits/unbits
# round-trip (unbits(bits(x)) == x).
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
OUT="${TMPDIR:-/tmp}/unbits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/unbits.png" \
    "echo warmup" "unbits 01001000 01101001" "unbits 01000001" "bits Hi" \
    "unbits 012" "unbits" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unbits:|usage: unbits|bits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def unbits(t):
    return ''.join(chr(int(g, 2)) for g in t.split())
def bits(t):
    return ' '.join(format(b, '08b') for b in t.encode())

# decode cases
for enc, dec in [("01001000 01101001", "Hi"), ("01000001", "A")]:
    want = f"unbits: {dec}"
    if want not in s:
        print(f"UNBITS TEST FAILED (missing '{want}')"); sys.exit(1)
    assert unbits(enc) == dec

# round-trip: bits Hi == "01001000 01101001" and unbits of that == "Hi"
assert bits("Hi") == "01001000 01101001"
assert unbits(bits("Hi")) == "Hi"
if "bits: 01001000 01101001" not in s:
    print("UNBITS TEST FAILED (bits Hi control missing for round-trip)"); sys.exit(1)

# invalid group + usage
if "unbits: invalid binary group '012'" not in s:
    print("UNBITS TEST FAILED (missing invalid-group message)"); sys.exit(1)
if "usage: unbits <binary>" not in s:
    print("UNBITS TEST FAILED (missing usage line)"); sys.exit(1)

print("UNBITS TEST PASSED  (decode Hi/A, bits/unbits round-trip, invalid group "
      "rejected, empty->usage)")
PY
