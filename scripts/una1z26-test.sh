#!/usr/bin/env bash
# Verify `una1z26 <numbers>` decodes an A1Z26 cipher back to (lowercase) letters -- the inverse of
# a1z26. Numbers 1-26 (dash-joined within words, space-separated between words) -> letters.
# "1-2-3"->"abc", "8-5-12-12-15 23-15-18-12-4"->"hello world", "15-19"->"os". Self-checking: python.
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
OUT="${TMPDIR:-/tmp}/una1z26.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/una1z26.png" \
    "echo warmup" "una1z26 1-2-3" "una1z26 8-5-12-12-15 23-15-18-12-4" \
    "una1z26 15-19" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "una1z26:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def una1z26(t):                      # DERIVED reference
    words = []
    for w in t.split():
        letters = []
        for part in w.split("-"):
            if not part: continue
            if part.isdigit() and 1 <= int(part) <= 26:
                letters.append(chr(ord("a") + int(part) - 1))
            else:
                letters.append("?")
        words.append("".join(letters))
    return " ".join(words)

def a1z26(t):
    return " ".join("-".join(str(ord(c.lower()) - ord("a") + 1)
                    for c in w if c.isascii() and c.isalpha()) for w in t.split())

cases = ["1-2-3", "8-5-12-12-15 23-15-18-12-4", "15-19"]
for t in cases:
    want = "una1z26: " + una1z26(t)
    if want not in s:
        print(f"UNA1Z26 TEST FAILED (missing '{want}')"); sys.exit(1)

assert una1z26("1-2-3") == "abc"
assert una1z26("8-5-12-12-15 23-15-18-12-4") == "hello world"
assert una1z26("15-19") == "os"
assert una1z26(a1z26("hello world")) == "hello world"   # round-trip
print("UNA1Z26 TEST PASSED  (abc/hello world/os vs python; una1z26(a1z26(x)) round-trips)")
PY
