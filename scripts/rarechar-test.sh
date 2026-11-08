#!/usr/bin/env bash
# Verify `rarechar <text>` reports the LEAST frequent non-whitespace char + count, ties->first:
# "aabbbc"->c (1), "mississippi"->m (1), "aabb"->a (2), "abc"->a (1), "a a b b b"->a (2). Complement
# of topchar; inline char analogue of rareword. Self-checking: python least-frequent non-space char.
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
OUT="${TMPDIR:-/tmp}/rarechar.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rarechar.png" \
    "echo warmup" "rarechar aabbbc" "rarechar mississippi" "rarechar aabb" \
    "rarechar abc" "rarechar a a b b b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rarechar:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):
    best, best_c = "", None
    seen = set()
    for ch in text:
        if ch.isspace() or ch in seen:
            continue
        seen.add(ch)
        c = text.count(ch)
        if best_c is None or c < best_c:
            best_c, best = c, ch
    return f"rarechar: {best} ({best_c})"

for text in ["aabbbc", "mississippi", "aabb", "abc", "a a b b b"]:
    w = want(text)
    if w not in s:
        print(f"RARECHAR TEST FAILED (missing '{w}' for '{text}')"); sys.exit(1)

assert "rarechar: c (1)" in s
assert "rarechar: m (1)" in s
assert "rarechar: a (2)" in s     # aabb and 'a a b b b'
assert "rarechar: a (1)" in s     # abc

print("RARECHAR TEST PASSED  (aabbbc->c (1), mississippi->m (1), aabb->a (2), abc->a (1), "
      "'a a b b b'->a (2) vs python least-frequent non-space char)")
PY
