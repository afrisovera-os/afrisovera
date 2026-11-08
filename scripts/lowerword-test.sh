#!/usr/bin/env bash
# Verify `lowerword <n> <text>` lowercases only the nth word, keeping order: 1 HELLO WORLD->
# hello WORLD, 2 ONE TWO THREE->ONE two THREE, 3 X Y Z->X Y z, 5 A B C->A B C (out of range no-op),
# 2 AB CD EF->AB cd EF. Complement of upperword; distinct from whisper (whole text). Self-checking:
# python lower words[n-1].
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
OUT="${TMPDIR:-/tmp}/lowerword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lowerword.png" \
    "echo warmup" "lowerword 1 HELLO WORLD" "lowerword 2 ONE TWO THREE" "lowerword 3 X Y Z" \
    "lowerword 5 A B C" "lowerword 2 AB CD EF" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lowerword:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    ws = text.split()
    if 1 <= n <= len(ws):
        ws[n - 1] = ws[n - 1].lower()
    return "lowerword: " + " ".join(ws)

cases = [(1, "HELLO WORLD"), (2, "ONE TWO THREE"), (3, "X Y Z"), (5, "A B C"), (2, "AB CD EF")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"LOWERWORD TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: first, middle, last, out-of-range, second-of-three
assert "lowerword: hello WORLD" in s
assert "lowerword: ONE two THREE" in s
assert "lowerword: X Y z" in s
assert "lowerword: A B C" in s          # out-of-range no-op
assert "lowerword: AB cd EF" in s

print("LOWERWORD TEST PASSED  (1 HELLO WORLD->hello WORLD, 2 ONE TWO THREE->ONE two THREE, "
      "3 X Y Z->X Y z, 5 A B C->A B C, 2 AB CD EF->AB cd EF vs python lower words[n-1])")
PY
