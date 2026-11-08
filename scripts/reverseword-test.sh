#!/usr/bin/env bash
# Verify `reverseword <n> <text>` reverses only the nth word's chars, keeping order: 1 hello world->
# olleh world, 2 abc def->abc fed, 2 ab cd ef->ab dc ef, 5 a b c->a b c (out of range no-op),
# 3 one two six->one two xis. Distinct from reveach (all words)/rwords (word order). Self-checking:
# python reverse words[n-1].
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
OUT="${TMPDIR:-/tmp}/reverseword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/reverseword.png" \
    "echo warmup" "reverseword 1 hello world" "reverseword 2 abc def" "reverseword 2 ab cd ef" \
    "reverseword 5 a b c" "reverseword 3 one two six" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "reverseword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    ws = text.split()
    if 1 <= n <= len(ws):
        ws[n - 1] = ws[n - 1][::-1]
    return "reverseword: " + " ".join(ws)

cases = [(1, "hello world"), (2, "abc def"), (2, "ab cd ef"), (5, "a b c"), (3, "one two six")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"REVERSEWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first word, second word, middle-of-three, out-of-range, last word
assert "reverseword: olleh world" in s
assert "reverseword: abc fed" in s
assert "reverseword: ab dc ef" in s
assert "reverseword: a b c" in s          # out-of-range no-op
assert "reverseword: one two xis" in s

print("REVERSEWORD TEST PASSED  (1 hello world->olleh world, 2 abc def->abc fed, 2 ab cd ef->ab dc ef, "
      "5 a b c->a b c, 3 one two six->one two xis vs python reverse words[n-1])")
PY
