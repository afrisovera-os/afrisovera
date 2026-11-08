#!/usr/bin/env bash
# Verify `substr <start> <len> <text>` extracts the substring from 1-based position <start>
# for <len> chars: "2 3 hello" -> ell, "1 5 hello" -> hello, "3 10 hello" -> llo (len past
# end clamps), "1 3 hello world" -> hel, "4 2 sovereign" -> er. Distinct from firstn/lastn/
# charat/middlechar. Self-checking: python's own slice text[start-1:start-1+len] is the ref.
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
OUT="${TMPDIR:-/tmp}/substr.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/substr.png" \
    "echo warmup" "substr 2 3 hello" "substr 1 5 hello" "substr 3 10 hello" \
    "substr 1 3 hello world" "substr 4 2 sovereign" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "substr:|usage: substr" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(start, length, text):
    return "substr: " + text[start - 1 : start - 1 + length]   # DERIVED via python slice

cases = [(2, 3, "hello"), (1, 5, "hello"), (3, 10, "hello"),
         (1, 3, "hello world"), (4, 2, "sovereign")]
for start, length, text in cases:
    w = want(start, length, text)
    if w not in s:
        print(f"SUBSTR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mid-slice, whole string, len clamp past end, multi-word, offset
assert "substr: ell" in s
assert "substr: hello" in s
assert "substr: llo" in s          # len 10 clamps to what remains
assert "substr: hel" in s          # slice stops before the space
assert "substr: er" in s           # sovereign[3:5]

print("SUBSTR TEST PASSED  (2 3 hello->ell, 1 5 hello->hello, 3 10 hello->llo clamp, "
      "1 3 hello world->hel, 4 2 sovereign->er vs python slice)")
PY
