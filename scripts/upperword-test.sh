#!/usr/bin/env bash
# Verify `upperword <n> <text>` uppercases only the nth word, keeping order: 1 hello world->
# HELLO world, 2 one two three->one TWO three, 3 x y z->x y Z, 5 a b c->a b c (out of range no-op),
# 2 ab cd ef->ab CD ef. Distinct from shout (whole text)/title (each word's first). Self-checking:
# python upper words[n-1].
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
OUT="${TMPDIR:-/tmp}/upperword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/upperword.png" \
    "echo warmup" "upperword 1 hello world" "upperword 2 one two three" "upperword 3 x y z" \
    "upperword 5 a b c" "upperword 2 ab cd ef" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "upperword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    ws = text.split()
    if 1 <= n <= len(ws):
        ws[n - 1] = ws[n - 1].upper()
    return "upperword: " + " ".join(ws)

cases = [(1, "hello world"), (2, "one two three"), (3, "x y z"), (5, "a b c"), (2, "ab cd ef")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"UPPERWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first, middle, last, out-of-range, second-of-three
assert "upperword: HELLO world" in s
assert "upperword: one TWO three" in s
assert "upperword: x y Z" in s
assert "upperword: a b c" in s          # out-of-range no-op
assert "upperword: ab CD ef" in s

print("UPPERWORD TEST PASSED  (1 hello world->HELLO world, 2 one two three->one TWO three, "
      "3 x y z->x y Z, 5 a b c->a b c, 2 ab cd ef->ab CD ef vs python upper words[n-1])")
PY
