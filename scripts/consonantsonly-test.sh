#!/usr/bin/env bash
# Verify the `consonantsonly <text>` command keeps ONLY the ASCII consonants
# (letters that are NOT vowels), dropping vowels/digits/punct/spaces. Rounds out
# the vowel/consonant/letter/digit filter family; distinct from disemvowel
# (keeps consonants BUT ALSO punct+spaces), lettersonly (keeps all letters), and
# vowelsonly. No consonants -> "(none)". Self-checking: python keeps only the
# non-vowel letters.
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
OUT="${TMPDIR:-/tmp}/consonantsonly.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/consonantsonly.png" \
    "echo warmup" "consonantsonly AfriSovera OS" "consonantsonly Hello World 42" \
    "consonantsonly aeiou AEIOU" "consonantsonly" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "consonantsonly:|usage: consonantsonly" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def consonantsonly(t):
    r = "".join(c for c in t if c.isalpha() and c.lower() not in "aeiou")
    return r if r else "(none)"

# shell-demo joins argv with single spaces, so the inline text matches exactly.
cases = ["AfriSovera OS", "Hello World 42", "aeiou AEIOU"]
for t in cases:
    want = f"consonantsonly: {consonantsonly(t)}"
    if want not in s:
        print(f"CONSONANTSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms (case preserved; vowels/digits/spaces dropped)
assert "consonantsonly: frSvrS" in s   # AfriSovera OS -> f,r,S,v,r,S
assert "consonantsonly: HllWrld" in s   # Hello World 42 -> H,l,l,W,r,l,d
assert "consonantsonly: (none)" in s    # aeiou AEIOU -> all vowels -> nothing

# empty input -> usage
if "usage: consonantsonly <text>" not in s:
    print("CONSONANTSONLY TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("CONSONANTSONLY TEST PASSED  (AfriSovera OS->frSvrS, Hello World 42->HllWrld, "
      "aeiou AEIOU->(none) vs python; empty -> usage)")
PY
