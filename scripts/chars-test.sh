#!/usr/bin/env bash
# Verify the `chars` command tallies the character-class breakdown of an inline
# string (total/letters/digits/spaces/other). "Hello 42!" -> 9/5/2/1/1; "abc" ->
# 3/3/0/0/0; "12 34" -> 5/0/4/1/0. The test recomputes each tally in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4920,server,nowait \
    -qmp tcp:127.0.0.1:4921,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/chars.out"
python3 scripts/shell-demo.py 4920 4921 "${TMPDIR:-/tmp}/chars.png" \
    "chars Hello 42!" "chars abc" "chars 12 34" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "chars:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def cls(t):
    total = len(t)
    letters = sum(c.isascii() and c.isalpha() for c in t)
    digits = sum(c.isascii() and c.isdigit() for c in t)
    spaces = t.count(" ")
    other = total - letters - digits - spaces
    return f"chars: total={total} letters={letters} digits={digits} spaces={spaces} other={other}"

for text in ["Hello 42!", "abc", "12 34"]:
    want = cls(text)
    if want not in s:
        print(f"CHARS TEST FAILED (expected '{want}' for {text!r})"); sys.exit(1)

print("CHARS TEST PASSED  (Hello 42! -> 9/5/2/1/1, abc -> 3/3/0/0/0, 12 34 -> 5/0/4/1/0)")
PY
