#!/usr/bin/env bash
# Verify the `vowelcase <text>` command uppercases every ASCII vowel and lowercases
# every other letter, leaving non-letters unchanged: "hello world" -> "hEllO wOrld".
# Distinct from mock (ALTERNATING case), swapcase (FLIP each), shout/whisper (ALL
# up/down), title/capitalize (word-start/first). Self-checking: python replicates.
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
OUT="${TMPDIR:-/tmp}/vowelcase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# "AfriSovera OS" mixes cases + a consonant-heavy word; "sky" has no vowels.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vowelcase.png" \
    "echo warmup" "vowelcase hello world" "vowelcase AfriSovera OS" "vowelcase SKY" "vowelcase a1e2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vowelcase:|usage: vowelcase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def vowelcase(t):
    out = []
    for c in t:
        low = c.lower()
        if c.isalpha() and low in "aeiou":
            out.append(c.upper())
        elif c.isalpha():
            out.append(c.lower())
        else:
            out.append(c)
    return "".join(out)

for t in ["hello world", "AfriSovera OS", "SKY", "a1e2"]:
    want = f"vowelcase: {vowelcase(t)}"
    if want not in s:
        print(f"VOWELCASE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "vowelcase: hEllO wOrld" in s
assert "vowelcase: AfrIsOvErA Os" in s      # mixed case normalised: vowels up, cons down (incl S->s)
assert "vowelcase: sky" in s                # no vowels -> all lowercased
assert "vowelcase: A1E2" in s               # digits pass through, vowels uppercased

print("VOWELCASE TEST PASSED  (hello world->hEllO wOrld, AfriSovera OS->AfrISovErA Os, "
      "SKY->sky, a1e2->A1E2 vs python; uppercase-vowels/lowercase-rest)")
PY
