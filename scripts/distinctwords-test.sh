#!/usr/bin/env bash
# Verify `distinctwords <text>` removes ALL duplicate words, keeping the FIRST occurrence in order --
# the word analogue of uniqchars: "the cat the dog cat"->"the cat dog", "a a b a"->"a b" (NOT "a b a"
# like dedupwords), "one two three"->all, "hi hi hi"->"hi", "Own your Own device your"->"Own your
# device" (case-sensitive). Self-checking: python order-preserving dedup.
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
OUT="${TMPDIR:-/tmp}/distinctwords.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/distinctwords.png" \
    "echo warmup" "distinctwords the cat the dog cat" "distinctwords a a b a" \
    "distinctwords one two three" "distinctwords hi hi hi" \
    "distinctwords Own your Own device your" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "distinctwords:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dw(t):                            # DERIVED reference: order-preserving dedup of words
    seen, out = set(), []
    for w in t.split():
        if w not in seen:
            seen.add(w); out.append(w)
    return "distinctwords: " + " ".join(out)

for t in ("the cat the dog cat", "a a b a", "one two three", "hi hi hi",
          "Own your Own device your"):
    w = dw(t)
    if w not in s:
        print(f"DISTINCTWORDS TEST FAILED (missing '{w}')"); sys.exit(1)

assert "distinctwords: the cat dog" in s
assert "distinctwords: a b" in s
assert "distinctwords: one two three" in s
assert "distinctwords: hi" in s
assert "distinctwords: Own your device" in s

print("DISTINCTWORDS TEST PASSED  (the cat the dog cat->the cat dog, a a b a->a b [not a b a], "
      "hi hi hi->hi, Own your Own device your->Own your device vs python order-preserving dedup)")
PY
