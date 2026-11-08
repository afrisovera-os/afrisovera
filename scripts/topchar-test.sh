#!/usr/bin/env bash
# Verify `topchar <text>` reports the MOST frequent non-whitespace char + count, ties->first:
# "aabbbc"->b (3), "hello"->l (2), "mississippi"->i (4) (i & s tie, i first), "abc"->a (1),
# "a a b b b"->b (3) (spaces ignored). Inline char analogue of topword. Self-checking: python.
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
OUT="${TMPDIR:-/tmp}/topchar.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/topchar.png" \
    "echo warmup" "topchar aabbbc" "topchar hello" "topchar mississippi" \
    "topchar abc" "topchar a a b b b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "topchar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference: most frequent non-space char, ties->first
    best, best_c = "", 0
    seen = set()
    for i, ch in enumerate(text):
        if ch.isspace() or ch in seen:
            continue
        seen.add(ch)
        c = text.count(ch)
        if c > best_c:
            best_c, best = c, ch
    return f"topchar: {best} ({best_c})"

cases = ["aabbbc", "hello", "mississippi", "abc", "a a b b b"]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"TOPCHAR TEST FAILED (missing '{w}' for '{text}')"); sys.exit(1)

# pin the notable branches: clear winner, double letter, tie->first, all-tie->first, spaces ignored
assert "topchar: b (3)" in s
assert "topchar: l (2)" in s
assert "topchar: i (4)" in s      # mississippi: i & s tie at 4, i first
assert "topchar: a (1)" in s      # abc all tie -> first

print("TOPCHAR TEST PASSED  (aabbbc->b (3), hello->l (2), mississippi->i (4), abc->a (1), "
      "'a a b b b'->b (3) vs python most-frequent non-space char)")
PY
