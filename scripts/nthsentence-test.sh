#!/usr/bin/env bash
# Verify `nthsentence <n> <text>` prints the Nth (1-based) sentence of inline text -- a sentence
# is a maximal run with at least one non-space char, terminated by one or more of . ! ? (trailing
# unterminated text still counts); the terminator is stripped and surrounding whitespace trimmed.
# "nthsentence 2 Hi there. How are you? Good!" -> "How are you", "nthsentence 1 Wait... really?!"
# -> "Wait", "nthsentence 1 No punctuation" -> "No punctuation" (no terminator). Out-of-range n
# reports a count message. Self-checking: python re-implementation of the segmentation rule.
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
OUT="${TMPDIR:-/tmp}/nthsentence.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthsentence.png" \
    "echo warmup" \
    "nthsentence 2 Hi there. How are you? Good!" \
    "nthsentence 1 Wait... really?!" \
    "nthsentence 2 Wait... really?!" \
    "nthsentence 1 No punctuation" \
    "nthsentence 3 Wait... really?!" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthsentence:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nthsentence(n, text):                 # DERIVED reference (mirrors the kernel byte scan)
    seg_start = 0
    has_content = False
    count = 0
    found = None
    for i, ch in enumerate(text):
        if ch in ".!?":
            if has_content:
                count += 1
                if count == n:
                    found = text[seg_start:i].strip()
                has_content = False
            seg_start = i + 1
        elif not ch.isspace():
            has_content = True
    if has_content:
        count += 1
        if count == n:
            found = text[seg_start:].strip()
    if found is not None:
        return f"nthsentence: {found}"
    return f"nthsentence: text has fewer than {n} sentence(s)"

CASES = [
    (2, "Hi there. How are you? Good!"),
    (1, "Wait... really?!"),
    (2, "Wait... really?!"),
    (1, "No punctuation"),
    (3, "Wait... really?!"),
]
for n, text in CASES:
    want = nthsentence(n, text)
    if want not in s:
        print(f"NTHSENTENCE TEST FAILED (missing '{want}')"); sys.exit(1)

assert nthsentence(2, "Hi there. How are you? Good!") == "nthsentence: How are you"
assert nthsentence(1, "Wait... really?!") == "nthsentence: Wait"
assert nthsentence(2, "Wait... really?!") == "nthsentence: really"
assert nthsentence(1, "No punctuation") == "nthsentence: No punctuation"
assert nthsentence(3, "Wait... really?!") == "nthsentence: text has fewer than 3 sentence(s)"
print("NTHSENTENCE TEST PASSED  (How are you / Wait / really / No punctuation / fewer-than vs python)")
PY
