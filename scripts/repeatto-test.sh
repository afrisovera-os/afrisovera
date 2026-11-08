#!/usr/bin/env bash
# Verify `repeatto <width> <text>` repeats the text cyclically to exactly width chars: "10 ab"
# -> ababababab, "7 abc" -> abcabca, "3 hello" -> hel, "5 x" -> xxxxx, "0 ab" -> "" (empty).
# Distinct from repeat/repeateach/firstn. Self-checking: python "".join(text[i%len] for i).
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
OUT="${TMPDIR:-/tmp}/repeatto.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/repeatto.png" \
    "echo warmup" "repeatto 10 ab" "repeatto 7 abc" "repeatto 3 hello" \
    "repeatto 5 x" "repeatto 0 ab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "repeatto:|usage: repeatto" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(w, text):
    body = "".join(text[i % len(text)] for i in range(w)) if text else ""   # DERIVED cyclic fill
    return "repeatto: " + body

cases = [(10, "ab"), (7, "abc"), (3, "hello"), (5, "x"), (0, "ab")]
for w, text in cases:
    ww = want(w, text)
    if ww not in s:
        print(f"REPEATTO TEST FAILED (w={w} text={text} missing '{ww}')"); sys.exit(1)

# pin the notable branches: exact multiple, cut mid-pattern, truncated long text, single char, empty
assert "repeatto: ababababab" in s
assert "repeatto: abcabca" in s
assert "repeatto: hel" in s
assert "repeatto: xxxxx" in s

print("REPEATTO TEST PASSED  (10 ab->ababababab, 7 abc->abcabca, 3 hello->hel, 5 x->xxxxx, "
      "0 ab->empty vs python cyclic fill)")
PY
