#!/usr/bin/env bash
# Verify `insertword <n> <word> <text>` inserts a word before the nth position: "2 X a b c"
# -> a X b c, "1 X a b c" -> X a b c, "4 X a b c" -> a b c X (append), "10 X a b c" -> a b c X
# (clamp), "2 hello one two three" -> one hello two three. Word-level insert. Self-checking:
# python words.insert(min(n-1,len)).
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
OUT="${TMPDIR:-/tmp}/insertword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/insertword.png" \
    "echo warmup" "insertword 2 X a b c" "insertword 1 X a b c" "insertword 4 X a b c" \
    "insertword 10 X a b c" "insertword 2 hello one two three" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "insertword:|usage: insertword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, word, text):
    words = text.split()
    words.insert(min(n - 1, len(words)), word)       # DERIVED clamped insert
    return "insertword: " + " ".join(words)

cases = [(2, "X", "a b c"), (1, "X", "a b c"), (4, "X", "a b c"),
         (10, "X", "a b c"), (2, "hello", "one two three")]
for n, word, text in cases:
    w = want(n, word, text)
    if w not in s:
        print(f"INSERTWORD TEST FAILED (n={n} word={word} missing '{w}')"); sys.exit(1)

# pin the notable branches: mid insert, prepend, append at end, clamp-append, mid multi-word
assert "insertword: a X b c" in s
assert "insertword: X a b c" in s
assert "insertword: a b c X" in s
assert "insertword: one hello two three" in s

print("INSERTWORD TEST PASSED  (2 X a b c->a X b c, 1 X->X a b c, 4 X->a b c X, 10 X->a b c X, "
      "2 hello one two three->one hello two three vs python list insert)")
PY
