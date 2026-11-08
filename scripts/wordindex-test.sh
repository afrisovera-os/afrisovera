#!/usr/bin/env bash
# Verify `wordindex <word> <text>` gives the 1-based index of the first whole-word match, or
# "not found": "cat the cat sat" -> 2, "the the cat sat" -> 1, "sat the cat sat" -> 3, "dog
# the cat sat" -> not found, "cat cat cat cat" -> 1. Inverse of wordat. Self-checking: python
# text.split().index.
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
OUT="${TMPDIR:-/tmp}/wordindex.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordindex.png" \
    "echo warmup" "wordindex cat the cat sat" "wordindex the the cat sat" "wordindex sat the cat sat" \
    "wordindex dog the cat sat" "wordindex cat cat cat cat" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordindex:|usage: wordindex" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(word, text):
    words = text.split()
    for i, w in enumerate(words):
        if w == word:
            return f"wordindex: {i + 1}"           # DERIVED first whole-word index
    return "wordindex: not found"

cases = [("cat", "the cat sat"), ("the", "the cat sat"), ("sat", "the cat sat"),
         ("dog", "the cat sat"), ("cat", "cat cat cat")]
for word, text in cases:
    w = want(word, text)
    if w not in s:
        print(f"WORDINDEX TEST FAILED (word '{word}' text '{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: 2nd word, 1st word, 3rd word, absent, first of repeats
assert "wordindex: 2" in s
assert "wordindex: 1" in s
assert "wordindex: 3" in s
assert "wordindex: not found" in s

print("WORDINDEX TEST PASSED  (cat->2, the->1, sat->3, dog->not found, cat cat cat->1 "
      "vs python word index)")
PY
