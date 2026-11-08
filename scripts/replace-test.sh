#!/usr/bin/env bash
# Verify the `replace` command substitutes all non-overlapping occurrences of one
# substring for another: cat dog "my cat and cat" -> my dog and dog; o 0 "hello
# world" -> hell0 w0rld; ab X ababab -> XXX. Test recomputes text.replace() in python.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4988,server,nowait \
    -qmp tcp:127.0.0.1:4989,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/replace.out"
python3 scripts/shell-demo.py 4988 4989 "${TMPDIR:-/tmp}/replace.png" \
    "replace cat dog my cat and cat" "replace o 0 hello world" "replace ab X ababab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "replace:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

cases = [("cat", "dog", "my cat and cat"), ("o", "0", "hello world"), ("ab", "X", "ababab")]
for frm, to, text in cases:
    want = f"replace: {text.replace(frm, to)}"
    if want not in s:
        print(f"REPLACE TEST FAILED (expected '{want}' for {frm}->{to} in {text!r})"); sys.exit(1)

print("REPLACE TEST PASSED  (cat/dog->my dog and dog, o/0->hell0 w0rld, ab/X->XXX)")
PY
