#!/usr/bin/env bash
# Verify the `unique` command removes ALL duplicate words (keeping the first
# occurrence), preserving order -- distinct from `dedup` (adjacent-only):
# a b a c b -> a b c; x x x -> x; the cat the dog the cat -> the cat dog.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4972,server,nowait \
    -qmp tcp:127.0.0.1:4973,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/unique.out"
python3 scripts/shell-demo.py 4972 4973 "${TMPDIR:-/tmp}/unique.png" \
    "unique a b a c b" "unique x x x" "unique the cat the dog the cat" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "unique:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def uniq(text):
    out = []
    for w in text.split():
        if w not in out:
            out.append(w)
    return " ".join(out)

for text in ["a b a c b", "x x x", "the cat the dog the cat"]:
    want = f"unique: {uniq(text)}"
    if want not in s:
        print(f"UNIQUE TEST FAILED (expected '{want}' for {text!r})"); sys.exit(1)

print("UNIQUE TEST PASSED  (a b a c b->a b c, x x x->x, the cat the dog the cat->the cat dog)")
PY
