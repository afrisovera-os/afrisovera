#!/usr/bin/env bash
# Verify the `piglatin <text>` command converts each space-separated word to Pig
# Latin (joined by single spaces). Rules (lowercase first; y is a CONSONANT;
# vowels a,e,i,o,u): no vowel -> word+"ay"; first vowel at 0 -> word+"way"; else
# (leading consonant cluster of length i) -> word[i:]+word[:i]+"ay". Self-checking:
# python replicates the same algorithm and asserts the exact output per input.
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
OUT="${TMPDIR:-/tmp}/piglatin.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/piglatin.png" \
    "piglatin hello" "piglatin own" "piglatin string" "piglatin pig" "piglatin my" \
    "piglatin own your device" "piglatin" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "piglatin:|usage: piglatin" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pig(w):
    w = w.lower()
    vs = "aeiou"
    idx = next((i for i, c in enumerate(w) if c in vs), None)
    if idx is None:
        return w + "ay"
    if idx == 0:
        return w + "way"
    return w[idx:] + w[:idx] + "ay"

def phrase(text):
    return " ".join(pig(w) for w in text.split())

cases = ["hello", "own", "string", "pig", "my", "own your device"]
for text in cases:
    want = f"piglatin: {phrase(text)}"
    if want not in s:
        print(f"PIGLATIN TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin a few
assert pig("hello") == "ellohay"
assert pig("own") == "ownway"
assert pig("string") == "ingstray"
assert pig("my") == "myay"  # no vowel (y is a consonant)

if "usage: piglatin <text>" not in s:
    print("PIGLATIN TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("PIGLATIN TEST PASSED  (Pig Latin exact vs python; vowel/consonant/no-vowel cases, multi-word, empty->usage)")
PY
