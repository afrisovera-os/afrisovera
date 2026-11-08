#!/usr/bin/env bash
# Verify the `longestword <text>` command reports the longest whitespace-delimited
# word (by byte length) in the inline text, plus its length, with TIES resolving
# to the FIRST such word. DISTINCT from `rwords` (reverses word ORDER),
# `chars`/`vowels` (counts), and `wordn`/`avglen` (which read VFS files).
# Self-checking: python recomputes the expected longest word (first-on-tie) for
# each input and asserts the exact output line.
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
OUT="${TMPDIR:-/tmp}/longestword.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/longestword.png" \
    "longestword the quick brown fox" "longestword size matters here" \
    "longestword cat dog" "longestword hello" "longestword" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "longestword:|usage: longestword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def longest(text):
    best = ""
    for w in text.split():
        if len(w) > len(best):   # strict > keeps the FIRST word on a tie
            best = w
    return best

cases = [
    "the quick brown fox",   # quick/brown both 5 -> quick (first)
    "size matters here",     # matters (7)
    "cat dog",               # cat/dog both 3 -> cat (first)
    "hello",                 # hello (5)
]
for text in cases:
    w = longest(text)
    want = f"longestword: {w} ({len(w)} bytes)"
    if want not in s:
        print(f"LONGESTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: longestword <text>" not in s:
    print("LONGESTWORD TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("LONGESTWORD TEST PASSED  (longest word + length exact vs python, ties->first, empty->usage)")
PY
