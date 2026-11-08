#!/usr/bin/env bash
# Verify `a1z26 <text>` encodes each letter as its 1-26 position, dash-joined within words, space-
# separated between words (non-letters in a word skipped, case-folded). "abc"->"1-2-3", "hello world"
# ->"8-5-12-12-15 23-15-18-12-4", "OS"->"15-19". Self-checking: python position mapping.
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
OUT="${TMPDIR:-/tmp}/a1z26.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/a1z26.png" \
    "echo warmup" "a1z26 abc" "a1z26 hello world" "a1z26 OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "a1z26:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def a1z26(t):                        # DERIVED reference
    words = []
    for w in t.split():
        nums = [str(ord(c.lower()) - ord("a") + 1) for c in w if c.isascii() and c.isalpha()]
        words.append("-".join(nums))
    return " ".join(words)

cases = ["abc", "hello world", "OS"]
for t in cases:
    want = "a1z26: " + a1z26(t)
    if want not in s:
        print(f"A1Z26 TEST FAILED (missing '{want}')"); sys.exit(1)

assert a1z26("abc") == "1-2-3"
assert a1z26("hello world") == "8-5-12-12-15 23-15-18-12-4"
assert a1z26("OS") == "15-19"
print("A1Z26 TEST PASSED  (1-2-3 / 8-5-12-12-15 23-15-18-12-4 / 15-19 vs python)")
PY
