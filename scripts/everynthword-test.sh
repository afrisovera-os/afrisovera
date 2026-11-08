#!/usr/bin/env bash
# Verify `everynthword <n> <text>` keeps every nth WORD (positions 0,n,2n,...; words[::n]) -- the
# word-level analogue of everynth (chars): "2 a b c d e f"->"a c e", "3 a b c d e f g"->"a d g",
# "1 a b c"->"a b c", "2 one two three"->"one three". Self-checking: python words[::n].
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
OUT="${TMPDIR:-/tmp}/everynthword.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/everynthword.png" \
    "echo warmup" "everynthword 2 a b c d e f" "everynthword 3 a b c d e f g" \
    "everynthword 1 a b c" "everynthword 2 one two three" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "everynthword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference: words[::n]
    ws = text.split()[::n]
    return "everynthword: " + (" ".join(ws) if ws else "(none)")

cases = [(2, "a b c d e f"), (3, "a b c d e f g"), (1, "a b c"), (2, "one two three")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"EVERYNTHWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: stride 2, stride 3, stride 1 (identity), real words
assert "everynthword: a c e" in s
assert "everynthword: a d g" in s
assert "everynthword: a b c" in s
assert "everynthword: one three" in s

print("EVERYNTHWORD TEST PASSED  (2:a b c d e f->a c e, 3:...->a d g, 1:a b c->a b c, "
      "2:one two three->one three vs python words[::n])")
PY
