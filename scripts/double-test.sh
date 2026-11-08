#!/usr/bin/env bash
# Verify the `double <text>` command repeats each character of the inline text
# twice. e.g. "abc" -> "aabbcc", "Hi!" -> "HHii!!". DISTINCT from spaceout
# (inserts a space BETWEEN chars, does not repeat), sortchars (sorts), and the
# case/cipher transforms. Self-checking: python's ''.join(c*2 for c in t).
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
OUT="${TMPDIR:-/tmp}/double.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/double.png" \
    "echo warmup" "double abc" "double Hi!" "double x" \
    "double" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "double:|usage: double" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def db(t):
    return ''.join(c * 2 for c in t)

cases = ["abc", "Hi!", "x"]
for text in cases:
    want = f"double: {db(text)}"
    if want not in s:
        print(f"DOUBLE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: each character repeated twice
assert db("abc") == "aabbcc"
assert db("Hi!") == "HHii!!"
assert db("x") == "xx"

if "usage: double <text>" not in s:
    print("DOUBLE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("DOUBLE TEST PASSED  (abc->aabbcc, Hi!->HHii!!, x->xx exact vs python; "
      "empty->usage)")
PY
