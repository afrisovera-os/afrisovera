#!/usr/bin/env bash
# Verify `topword <text>` reports the most frequent word + count, ties->first: "a b a c a"->a (3),
# "the cat the dog"->the (2), "x y z"->x (1), "hi hi bye bye hi"->hi (3), "one two two"->two (2).
# Inline analogue of wordfreq (FILE). Self-checking: python first-occ max count.
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
OUT="${TMPDIR:-/tmp}/topword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/topword.png" \
    "echo warmup" "topword a b a c a" "topword the cat the dog" "topword x y z" \
    "topword hi hi bye bye hi" "topword one two two" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "topword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference
    ws = text.split()
    best, best_c = "", 0
    for i, w in enumerate(ws):
        if w in ws[:i]:
            continue
        c = ws.count(w)
        if c > best_c:
            best_c, best = c, w
    return f"topword: {best} ({best_c})"

cases = ["a b a c a", "the cat the dog", "x y z", "hi hi bye bye hi", "one two two"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"TOPWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: clear winner, one repeat, all-tie->first, interleaved, trailing dup
assert "topword: a (3)" in s
assert "topword: the (2)" in s
assert "topword: x (1)" in s          # all tie -> first word
assert "topword: hi (3)" in s
assert "topword: two (2)" in s

print("TOPWORD TEST PASSED  (a b a c a->a (3), the cat the dog->the (2), x y z->x (1), "
      "hi hi bye bye hi->hi (3), one two two->two (2) vs python first-occ max count)")
PY
