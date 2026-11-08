#!/usr/bin/env bash
# Verify the `capitalize` command uppercases ONLY the first alphabetic character of
# the string, leaving everything else unchanged -- distinct from title (every word)
# and swapcase (all letters). The harness applies the SAME rule in python (find the
# first alpha, uppercase it if lowercase, keep the rest VERBATIM -- NOT python
# str.capitalize(), which also lowercases the tail), so it is self-checking:
# "hello world"->"Hello world" (w stays lowercase), "HELLO" unchanged, "123abc"->"123Abc".
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
OUT="${TMPDIR:-/tmp}/capitalize.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/capitalize.png" \
    "capitalize hello world" "capitalize HELLO" "capitalize 123abc" \
    "capitalize hi there" "capitalize" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "capitalize:|usage: capitalize" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def cap(t):
    # Uppercase only the first alphabetic char; keep the rest verbatim.
    out = list(t)
    for i, ch in enumerate(out):
        if ch.isalpha():
            if ch.islower():
                out[i] = ch.upper()
            break
    return "".join(out)

for t in ["hello world", "HELLO", "123abc", "hi there"]:
    want = f"capitalize: {cap(t)}"
    if want not in s:
        print(f"CAPITALIZE TEST FAILED (missing '{want}' for {t!r})"); sys.exit(1)

if "usage: capitalize <text>" not in s:
    print("CAPITALIZE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("CAPITALIZE TEST PASSED  (hello world->Hello world (w stays lower), HELLO unchanged, 123abc->123Abc; empty->usage)")
PY
