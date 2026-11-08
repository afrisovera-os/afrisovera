#!/usr/bin/env bash
# Verify the `leet <text>` command converts text to leetspeak via a fixed
# letter->digit substitution: a->4, e->3, i->1, o->0, s->5, t->7 (case-insensitive
# match); every other byte (other letters with case preserved, spaces, punctuation)
# passes through unchanged. DISTINCT from `mock` (alternates case), `swapcase`
# (inverts case), `disemvowel` (removes vowels), and the letter ciphers
# `caesar`/`atbash`/`rot47`. Self-checking: python replicates the same table.
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
OUT="${TMPDIR:-/tmp}/leet.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/leet.png" \
    "leet hello world" "leet AfriSovera" "leet test" "leet bcd" "leet" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "leet:|usage: leet" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

TABLE = {'a': '4', 'e': '3', 'i': '1', 'o': '0', 's': '5', 't': '7'}
def leet(t):
    # map on the lowercased char, but keep the ORIGINAL char when not in the table
    return ''.join(TABLE.get(c.lower(), c) for c in t)

cases = ["hello world", "AfriSovera", "test", "bcd"]
for text in cases:
    want = f"leet: {leet(text)}"
    if want not in s:
        print(f"LEET TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the substitution (both cases map; non-mapped letters keep case)
assert leet("hello world") == "h3ll0 w0rld"
assert leet("AfriSovera")  == "4fr150v3r4"   # A->4, i->1, S->5, o->0, e->3, a->4
assert leet("test")        == "7357"
assert leet("bcd")         == "bcd"          # no mapped letters -> unchanged

if "usage: leet <text>" not in s:
    print("LEET TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("LEET TEST PASSED  (a4 e3 i1 o0 s5 t7 exact vs python both cases, "
      "non-mapped bytes passthrough, empty->usage)")
PY
