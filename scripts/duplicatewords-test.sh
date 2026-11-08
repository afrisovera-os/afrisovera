#!/usr/bin/env bash
# Verify `duplicatewords <text>` lists words occurring >1 time, each once, first-occurrence order:
# "a b a c b"->a b, "the cat the dog the"->the, "a b c"->(none), "x x x"->x, "one two two one"->one two.
# Distinct from dedupwords (adjacent)/uniqwords (FILE count). Self-checking: python first-occ dup order.
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
OUT="${TMPDIR:-/tmp}/duplicatewords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/duplicatewords.png" \
    "echo warmup" "duplicatewords a b a c b" "duplicatewords the cat the dog the" \
    "duplicatewords a b c" "duplicatewords x x x" "duplicatewords one two two one" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "duplicatewords:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference
    ws = text.split()
    out = []
    for i, w in enumerate(ws):
        if w not in ws[:i] and w in ws[i + 1:]:
            out.append(w)
    return "duplicatewords: " + (" ".join(out) if out else "(none)")

cases = ["a b a c b", "the cat the dog the", "a b c", "x x x", "one two two one"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"DUPLICATEWORDS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: two dups, one dup, none, triple, order-preserved pair
assert "duplicatewords: a b" in s
assert "duplicatewords: the" in s
assert "duplicatewords: (none)" in s      # all unique
assert "duplicatewords: x" in s           # x x x -> x once
assert "duplicatewords: one two" in s     # first-occurrence order

print("DUPLICATEWORDS TEST PASSED  (a b a c b->a b, the cat the dog the->the, a b c->(none), "
      "x x x->x, one two two one->one two vs python first-occ dup order)")
PY
