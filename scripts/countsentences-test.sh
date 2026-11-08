#!/usr/bin/env bash
# Verify `countsentences <text>` counts sentences via . ! ? terminators (a run of terminators ends one
# sentence; trailing unterminated text still counts): "Hi there. How are you? Good!"->3,
# "Wait... really?!"->2, "No punctuation"->1, "One. Two."->2. Self-checking: python non-empty splits.
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
OUT="${TMPDIR:-/tmp}/countsentences.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countsentences.png" \
    "echo warmup" "countsentences Hi there. How are you? Good!" "countsentences Wait... really?!" \
    "countsentences No punctuation" "countsentences One. Two." > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countsentences:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, re
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(text):                       # DERIVED reference: non-empty [.!?]-split segments
    segs = [seg for seg in re.split(r"[.!?]", text) if seg.strip()]
    return f"countsentences: {len(segs)} sentence(s)"

cases = ["Hi there. How are you? Good!", "Wait... really?!", "No punctuation", "One. Two."]
for text in cases:
    w = want(text)
    if w not in s:
        print(f"COUNTSENTENCES TEST FAILED (missing '{w}' for '{text}')"); sys.exit(1)

# pin the notable branches: three sentences, terminator-run collapse, no terminator, two
assert "countsentences: 3 sentence(s)" in s
assert "countsentences: 2 sentence(s)" in s   # both "Wait... really?!" and "One. Two."
assert "countsentences: 1 sentence(s)" in s

print("COUNTSENTENCES TEST PASSED  (Hi there.How are you?Good!->3, Wait...really?!->2, "
      "No punctuation->1, One.Two.->2 vs python non-empty [.!?]-splits)")
PY
