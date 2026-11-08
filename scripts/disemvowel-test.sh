#!/usr/bin/env bash
# Verify the `disemvowel <text>` command removes ALL ASCII vowels (a,e,i,o,u,
# both cases) from the inline text, keeping consonants, spaces, and punctuation.
# DISTINCT from `vowels` (counts vowels/consonants), `squeeze` (collapses adjacent
# repeats), `dedup` (removes duplicate lines), `mock` (alternates case) -- none of
# them REMOVE vowels. Self-checking: python replicates the same filter and asserts
# each output line plus the empty-input usage line.
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
OUT="${TMPDIR:-/tmp}/disemvowel.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/disemvowel.png" \
    "disemvowel hello world" "disemvowel AfriSovera" "disemvowel aeiou AEIOU" \
    "disemvowel bcd" "disemvowel" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "disemvowel:|usage: disemvowel" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

VOWELS = set("aeiouAEIOU")
def disemvowel(t):
    return ''.join(c for c in t if c not in VOWELS)

cases = ["hello world", "AfriSovera", "aeiou AEIOU", "bcd"]
for text in cases:
    want = f"disemvowel: {disemvowel(text)}"
    if want not in s:
        print(f"DISEMVOWEL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the transform: consonants/spaces stay, vowels vanish (both cases)
assert disemvowel("hello world") == "hll wrld"
assert disemvowel("AfriSovera")  == "frSvr"    # ignores case: A/i/o/e/a all removed
assert disemvowel("aeiou AEIOU") == " "        # every vowel gone, the space remains
assert disemvowel("bcd")         == "bcd"      # no vowels -> unchanged

if "usage: disemvowel <text>" not in s:
    print("DISEMVOWEL TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("DISEMVOWEL TEST PASSED  (vowels removed exact vs python both cases, "
      "consonants/spaces kept, empty->usage)")
PY
