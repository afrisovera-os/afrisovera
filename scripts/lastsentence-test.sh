#!/usr/bin/env bash
# Verify `lastsentence <text>` prints the LAST sentence of inline text -- a sentence is a
# maximal run with at least one non-space char, terminated by one or more of . ! ? (trailing
# unterminated text still counts); the terminator is stripped and surrounding whitespace
# trimmed. "lastsentence Hi there. How are you? Good!" -> "Good", "lastsentence No punctuation"
# -> "No punctuation" (single trailing sentence, no terminator). Self-checking: python
# re-implementation of the segmentation rule (shared with nthsentence/countsentences).
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
OUT="${TMPDIR:-/tmp}/lastsentence.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastsentence.png" \
    "echo warmup" \
    "lastsentence Hi there. How are you? Good!" \
    "lastsentence Wait... really?!" \
    "lastsentence No punctuation" \
    "lastsentence Only one." > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastsentence:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lastsentence(text):                   # DERIVED reference (mirrors the kernel byte scan)
    seg_start = 0
    has_content = False
    last = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                last = text[seg_start:i].strip()
                has_content = False
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if has_content:
        last = text[seg_start:].strip()
    if last is not None:
        return f"lastsentence: {last}"
    return "lastsentence: text has no sentences"

CASES = [
    "Hi there. How are you? Good!",
    "Wait... really?!",
    "No punctuation",
    "Only one.",
]
for text in CASES:
    want = lastsentence(text)
    if want not in s:
        print(f"LASTSENTENCE TEST FAILED (missing '{want}')"); sys.exit(1)

assert lastsentence("Hi there. How are you? Good!") == "lastsentence: Good"
assert lastsentence("Wait... really?!") == "lastsentence: really"
assert lastsentence("No punctuation") == "lastsentence: No punctuation"
assert lastsentence("Only one.") == "lastsentence: Only one"
print("LASTSENTENCE TEST PASSED  (Good / really / No punctuation / Only one vs python)")
PY
