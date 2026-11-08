#!/usr/bin/env bash
# Verify `replaceword <old> <new> <text>` swaps only WHOLE words: "cat dog cat category" ->
# dog category (category untouched), "the a the cat the dog" -> a cat a dog, "x y a b c" ->
# a b c (no match), "a A a b a c" -> A b A c, "foo bar foofoo foo" -> foofoo bar. Distinct
# from replace (substring). Self-checking: python new if w==old else w over split().
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
OUT="${TMPDIR:-/tmp}/replaceword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/replaceword.png" \
    "echo warmup" "replaceword cat dog cat category" "replaceword the a the cat the dog" \
    "replaceword x y a b c" "replaceword a A a b a c" \
    "replaceword foo bar foofoo foo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "replaceword:|usage: replaceword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(old, new, text):
    return "replaceword: " + " ".join(new if w == old else w for w in text.split())   # DERIVED

cases = [("cat", "dog", "cat category"), ("the", "a", "the cat the dog"),
         ("x", "y", "a b c"), ("a", "A", "a b a c"), ("foo", "bar", "foofoo foo")]
for old, new, text in cases:
    w = want(old, new, text)
    if w not in s:
        print(f"REPLACEWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: whole-word only, multiple swaps, no-match, repeated, substring-safe
assert "replaceword: dog category" in s      # "category" NOT swapped
assert "replaceword: a cat a dog" in s
assert "replaceword: a b c" in s             # no match -> unchanged
assert "replaceword: A b A c" in s
assert "replaceword: foofoo bar" in s        # "foofoo" NOT swapped, only whole "foo"

print("REPLACEWORD TEST PASSED  (cat dog cat category->dog category, the a...->a cat a dog, "
      "x y a b c->a b c, a A...->A b A c, foo bar foofoo foo->foofoo bar vs python whole-word swap)")
PY
