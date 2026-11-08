#!/usr/bin/env bash
# Verify the `rot13 <text>` command applies the classic ROT13 letter cipher (shift
# each ASCII letter by 13, wrapping within its case; non-letters pass through). It
# is SELF-INVERSE: rot13(rot13(x)) == x. e.g. "Hello" -> "Uryyb". Unlike caesar
# (which takes a numeric shift argument), rot13 is the fixed argument-free form.
# Self-checking: python's codecs 'rot_13' encoder is the reference.
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
OUT="${TMPDIR:-/tmp}/rot13.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot13.png" \
    "echo warmup" "rot13 Hello" "rot13 AfriSovera" "rot13 Uryyb" \
    "rot13 abc123" "rot13" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot13:|usage: rot13" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, codecs
s = open(sys.argv[1], "rb").read().decode("latin1")

def rot13(t):
    return codecs.encode(t, "rot_13")

for t in ["Hello", "AfriSovera", "Uryyb", "abc123"]:
    want = f"rot13: {rot13(t)}"
    if want not in s:
        print(f"ROT13 TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the classic cases + the SELF-INVERSE property
assert rot13("Hello") == "Uryyb"
assert rot13("AfriSovera") == "NsevFbiren"
assert rot13("abc123") == "nop123"        # digits/punct pass through
assert rot13(rot13("Hello")) == "Hello"   # self-inverse
assert rot13("Uryyb") == "Hello"          # round-trip observed in the capture

if "usage: rot13 <text>" not in s:
    print("ROT13 TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("ROT13 TEST PASSED  (Hello/AfriSovera/abc123 vs python, self-inverse "
      "round-trip Uryyb->Hello, empty->usage)")
PY
