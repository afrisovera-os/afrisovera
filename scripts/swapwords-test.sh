#!/usr/bin/env bash
# Verify `swapwords <i> <j> <text>` swaps the i-th and j-th (1-based) words: "1 3 a b c d" ->
# c b a d, "2 4 one two three four" -> one four three two, "1 1 a b c" -> a b c (same index),
# "1 5 a b c" -> a b c (out of range), "2 3 x y z" -> x z y. Distinct from swapwordpairs/rwords.
# Self-checking: python swaps w[i-1] and w[j-1] when both in range.
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
OUT="${TMPDIR:-/tmp}/swapwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapwords.png" \
    "echo warmup" "swapwords 1 3 a b c d" "swapwords 2 4 one two three four" \
    "swapwords 1 1 a b c" "swapwords 1 5 a b c" "swapwords 2 3 x y z" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapwords:|usage: swapwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(i, j, text):
    w = text.split()
    if i <= len(w) and j <= len(w):
        w[i - 1], w[j - 1] = w[j - 1], w[i - 1]     # DERIVED swap when both in range
    return "swapwords: " + " ".join(w)

cases = [(1, 3, "a b c d"), (2, 4, "one two three four"), (1, 1, "a b c"),
         (1, 5, "a b c"), (2, 3, "x y z")]
for i, j, text in cases:
    w = want(i, j, text)
    if w not in s:
        print(f"SWAPWORDS TEST FAILED (i={i} j={j} missing '{w}')"); sys.exit(1)

# pin the notable branches: swap 1&3, swap 2&4, same index no-op, out-of-range no-op, adjacent
assert "swapwords: c b a d" in s
assert "swapwords: one four three two" in s
assert "swapwords: a b c" in s        # covers both 1 1 and 1 5 (out of range) -> unchanged
assert "swapwords: x z y" in s

print("SWAPWORDS TEST PASSED  (1 3 a b c d->c b a d, 2 4 ...->one four three two, 1 1->a b c, "
      "1 5 out-of-range->a b c, 2 3 x y z->x z y vs python index swap)")
PY
