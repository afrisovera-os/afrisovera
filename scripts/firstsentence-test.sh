#!/usr/bin/env bash
# Verify `firstsentence <text>` prints the FIRST sentence of inline text -- a sentence is a
# maximal run with at least one non-space char, terminated by one or more of . ! ? (trailing
# unterminated text still counts); the terminator is stripped and surrounding whitespace
# trimmed. "firstsentence Hi there. How are you? Good!" -> "Hi there", "firstsentence No
# punctuation" -> "No punctuation" (single trailing sentence, no terminator). Self-checking:
# python re-implementation of the segmentation rule (shared with lastsentence/nthsentence/
# countsentences).
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
OUT="${TMPDIR:-/tmp}/firstsentence.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstsentence.png" \
    "echo warmup" \
    "firstsentence Hi there. How are you? Good!" \
    "firstsentence Wait... really?!" \
    "firstsentence No punctuation" \
    "firstsentence Only one." > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstsentence:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def firstsentence(text):                  # DERIVED reference (mirrors the kernel byte scan)
    seg_start = 0
    has_content = False
    first = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                first = text[seg_start:i].strip()
                break
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if first is None and has_content:
        first = text[seg_start:].strip()
    if first is not None:
        return f"firstsentence: {first}"
    return "firstsentence: text has no sentences"

CASES = [
    "Hi there. How are you? Good!",
    "Wait... really?!",
    "No punctuation",
    "Only one.",
]
for text in CASES:
    want = firstsentence(text)
    if want not in s:
        print(f"FIRSTSENTENCE TEST FAILED (missing '{want}')"); sys.exit(1)

assert firstsentence("Hi there. How are you? Good!") == "firstsentence: Hi there"
assert firstsentence("Wait... really?!") == "firstsentence: Wait"
assert firstsentence("No punctuation") == "firstsentence: No punctuation"
assert firstsentence("Only one.") == "firstsentence: Only one"
print("FIRSTSENTENCE TEST PASSED  (Hi there / Wait / No punctuation / Only one vs python)")
PY
