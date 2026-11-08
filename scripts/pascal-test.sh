#!/usr/bin/env bash
# Verify the `pascal <text>` command joins space-separated words into PascalCase
# (UpperCamelCase): EVERY word's first letter uppercased, all else lowercased,
# spaces removed. DISTINCT from camel (which lowercases the FIRST word), title
# (keeps spaces), slug (hyphens), capitalize (only the very first letter).
# Self-checking: python replicates the same algorithm and asserts each output.
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
OUT="${TMPDIR:-/tmp}/pascal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pascal.png" \
    "pascal hello world foo" "pascal own your device" "pascal AfriSovera OS" \
    "pascal hello" "pascal" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pascal:|usage: pascal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pascal(t):
    out = []
    for w in t.split():
        lw = w.lower()
        if lw and lw[0].isalpha():
            lw = lw[0].upper() + lw[1:]
        out.append(lw)
    return ''.join(out)

cases = ["hello world foo", "own your device", "AfriSovera OS", "hello"]
for text in cases:
    want = f"pascal: {pascal(text)}"
    if want not in s:
        print(f"PASCAL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: pascal capitalizes the first word (unlike camel)
assert pascal("own your device") == "OwnYourDevice"
assert pascal("AfriSovera OS") == "AfrisoveraOs"

if "usage: pascal <text>" not in s:
    print("PASCAL TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("PASCAL TEST PASSED  (PascalCase exact vs python; every word capitalized, empty->usage)")
PY
