#!/usr/bin/env bash
# Verify the `bits <text>` command renders each character of the inline text as
# its 8-bit binary, space-separated. e.g. "Hi" -> "01001000 01101001". DISTINCT
# from hex (base-16), base64/base32 (base encodings), base (numeric base of a
# NUMBER), gray (Gray code of a NUMBER), popcount (bit COUNT). Self-checking:
# python builds the expected binary via format(b,'08b') per byte.
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
OUT="${TMPDIR:-/tmp}/bits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bits.png" \
    "echo warmup" "bits Hi" "bits A" "bits AB" "bits" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bits:|usage: bits" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def bits(t):
    return ' '.join(format(b, '08b') for b in t.encode())

for t in ["Hi", "A", "AB"]:
    want = f"bits: {bits(t)}"
    if want not in s:
        print(f"BITS TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the exact 8-bit groups
assert bits("Hi") == "01001000 01101001"   # H=0x48, i=0x69
assert bits("A") == "01000001"             # A=0x41
assert bits("AB") == "01000001 01000010"

if "usage: bits <text>" not in s:
    print("BITS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("BITS TEST PASSED  (Hi/A/AB 8-bit binary exact vs python; empty->usage)")
PY
