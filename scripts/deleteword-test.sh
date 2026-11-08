#!/usr/bin/env bash
# Verify `deleteword <n> <text>` removes the nth word: "2 a b c" -> a c, "1 a b c" -> b c, "3
# a b c" -> a b, "5 a b c" -> a b c (out of range), "2 one two three four" -> one three four.
# Complement of insertword. Self-checking: python del words[n-1] when in range.
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
OUT="${TMPDIR:-/tmp}/deleteword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/deleteword.png" \
    "echo warmup" "deleteword 2 a b c" "deleteword 1 a b c" "deleteword 3 a b c" \
    "deleteword 5 a b c" "deleteword 2 one two three four" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "deleteword:|usage: deleteword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):
    words = text.split()
    if 1 <= n <= len(words):
        del words[n - 1]                             # DERIVED delete when in range
    return "deleteword: " + " ".join(words)

cases = [(2, "a b c"), (1, "a b c"), (3, "a b c"), (5, "a b c"),
         (2, "one two three four")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"DELETEWORD TEST FAILED (n={n} text='{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: mid delete, delete first, delete last, out-of-range no-op, multi-word
assert "deleteword: a c" in s
assert "deleteword: b c" in s
assert "deleteword: a b" in s
assert "deleteword: a b c" in s        # out of range -> unchanged
assert "deleteword: one three four" in s

print("DELETEWORD TEST PASSED  (2 a b c->a c, 1->b c, 3->a b, 5 out-of-range->a b c, "
      "2 one two three four->one three four vs python del)")
PY
