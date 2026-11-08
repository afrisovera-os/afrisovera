#!/usr/bin/env bash
# Verify `setword <n> <new> <text>` replaces the nth word by index: 2 X a b c->a X c,
# 1 hi a b c->hi b c, 3 Z a b c->a b Z, 5 Q a b c->a b c (out of range no-op), 2 yo one two three->one yo three.
# Distinct from replaceword (by VALUE). Self-checking: python ws[n-1]=new.
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
OUT="${TMPDIR:-/tmp}/setword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/setword.png" \
    "echo warmup" "setword 2 X a b c" "setword 1 hi a b c" "setword 3 Z a b c" \
    "setword 5 Q a b c" "setword 2 yo one two three" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "setword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, new, text):               # DERIVED reference
    ws = text.split()
    if 1 <= n <= len(ws):
        ws[n - 1] = new
    return "setword: " + " ".join(ws)

cases = [(2, "X", "a b c"), (1, "hi", "a b c"), (3, "Z", "a b c"), (5, "Q", "a b c"),
         (2, "yo", "one two three")]
for n, new, text in cases:
    w = want(n, new, text)
    if w not in s:
        print(f"SETWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: middle, first, last, out-of-range, multi-char words
assert "setword: a X c" in s
assert "setword: hi b c" in s
assert "setword: a b Z" in s
assert "setword: a b c" in s          # out-of-range no-op
assert "setword: one yo three" in s

print("SETWORD TEST PASSED  (2 X a b c->a X c, 1 hi a b c->hi b c, 3 Z a b c->a b Z, 5 Q a b c->a b c, "
      "2 yo one two three->one yo three vs python ws[n-1]=new)")
PY
