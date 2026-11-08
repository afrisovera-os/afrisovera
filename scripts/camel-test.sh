#!/usr/bin/env bash
# Verify the `camel <text>` command joins space-separated words into camelCase:
# first word all lowercase, each SUBSEQUENT word's first letter uppercased, all
# else lowercased, spaces removed. DISTINCT from slug (hyphen-joins + lowercases),
# title (keeps spaces, capitalizes each word), and capitalize (only the very
# first letter of the whole text). Self-checking: python replicates the same
# algorithm and asserts the exact output for each input.
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
OUT="${TMPDIR:-/tmp}/camel.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/camel.png" \
    "camel own your device" "camel hello world foo" "camel AfriSovera OS project" \
    "camel hello" "camel" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "camel:|usage: camel" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def camel(text):
    out = []
    for i, w in enumerate(text.split()):
        lw = w.lower()
        if i > 0 and lw and lw[0].isalpha():
            lw = lw[0].upper() + lw[1:]
        out.append(lw)
    return "".join(out)

cases = [
    "own your device",       # ownYourDevice
    "hello world foo",       # helloWorldFoo
    "AfriSovera OS project",  # afrisoveraOsProject
    "hello",                 # hello
]
for text in cases:
    want = f"camel: {camel(text)}"
    if want not in s:
        print(f"CAMEL TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the expected transforms
assert camel("own your device") == "ownYourDevice"
assert camel("AfriSovera OS project") == "afrisoveraOsProject"

if "usage: camel <text>" not in s:
    print("CAMEL TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("CAMEL TEST PASSED  (camelCase exact vs python; first word lowered, rest normalized, empty->usage)")
PY
