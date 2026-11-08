#!/usr/bin/env bash
# Verify the `shortestword <text>` command reports the shortest whitespace-
# delimited word (by byte length) in the inline text, plus its length, with TIES
# resolving to the FIRST such word. The min-analog of `longestword`. Self-checking:
# python replicates the same algorithm (best=None; strict < keeps first-on-tie).
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
OUT="${TMPDIR:-/tmp}/shortestword.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/shortestword.png" \
    "shortestword own your device" "shortestword a bb ccc" "shortestword hi there" \
    "shortestword cat dog pig" "shortestword hello" "shortestword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "shortestword:|usage: shortestword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def shortest(text):
    best = None
    for w in text.split():
        if best is None or len(w) < len(best):   # strict < keeps FIRST on tie
            best = w
    return best

cases = [
    "own your device",   # own (3) < your (4) < device (6)
    "a bb ccc",          # a (1)
    "hi there",          # hi (2)
    "cat dog pig",       # all 3 bytes -> cat (first)
    "hello",             # hello (5)
]
for text in cases:
    w = shortest(text)
    want = f"shortestword: {w} ({len(w)} bytes)"
    if want not in s:
        print(f"SHORTESTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: shortestword <text>" not in s:
    print("SHORTESTWORD TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SHORTESTWORD TEST PASSED  (shortest word + length exact vs python, ties->first, empty->usage)")
PY
