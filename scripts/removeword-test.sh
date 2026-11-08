#!/usr/bin/env bash
# Verify `removeword <word> <text>` removes all WHOLE-WORD occurrences: "the the cat the dog"->cat dog,
# "a a b a c a"->b c, "x a b c"->a b c (no match), "cat cat category cat"->category (substring-safe),
# "only only"->"" (empty). Distinct from deleteword (by index)/replaceword (swap)/delete (chars).
# Self-checking: python drop words == word.
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
OUT="${TMPDIR:-/tmp}/removeword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/removeword.png" \
    "echo warmup" "removeword the the cat the dog" "removeword a a b a c a" \
    "removeword x a b c" "removeword cat cat category cat" "removeword only only" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "removeword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(word, text):                 # DERIVED reference
    return "removeword: " + " ".join(w for w in text.split() if w != word)

cases = [("the", "the cat the dog"), ("a", "a b a c a"), ("x", "a b c"),
         ("cat", "cat category cat"), ("only", "only")]
for word, text in cases:
    w = want(word, text)
    if w not in s:
        print(f"REMOVEWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: repeated removal, interleaved, no-match, substring-safe, empty result
assert "removeword: cat dog" in s
assert "removeword: b c" in s
assert "removeword: a b c" in s        # no match -> unchanged
assert "removeword: category" in s     # "category" NOT removed (whole-word only)
# "only only" -> "" : the line is "removeword: " with empty tail
assert "removeword:" in s

print("REMOVEWORD TEST PASSED  (the...->cat dog, a...->b c, x a b c->a b c, cat cat category cat->category, "
      "only only->empty vs python whole-word drop)")
PY
