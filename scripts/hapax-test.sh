#!/usr/bin/env bash
# Verify `hapax <text>` lists the words occurring EXACTLY ONCE (hapax legomena) in first-occurrence
# order, single-spaced. Tests "the cat the dog cat"->"dog", "one two three"->all, "a a b b"->(none).
# Self-checking: python collections.Counter, keep count==1 in first-occurrence order.
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
OUT="${TMPDIR:-/tmp}/hapax.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hapax.png" \
    "echo warmup" "hapax the cat the dog cat" "hapax one two three" \
    "hapax a a b b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hapax:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
from collections import Counter
s = open(sys.argv[1], "rb").read().decode("latin1")

def hapax(t):                        # DERIVED reference: words with count==1, first-occurrence order
    ws = t.split()
    c = Counter(ws)
    seen, out = set(), []
    for w in ws:
        if c[w] == 1 and w not in seen:
            seen.add(w); out.append(w)
    return " ".join(out)

cases = {
    "the cat the dog cat": "dog",
    "one two three": "one two three",
}
for t, exp in cases.items():
    assert hapax(t) == exp, (t, hapax(t))
    want = "hapax: " + exp
    if want not in s:
        print(f"HAPAX TEST FAILED (missing '{want}')"); sys.exit(1)

assert hapax("a a b b") == ""
if "hapax: (none)" not in s:
    print("HAPAX TEST FAILED (missing '(none)' for all-repeated)"); sys.exit(1)

print("HAPAX TEST PASSED  (dog / one two three / (none) vs python Counter count==1)")
PY
