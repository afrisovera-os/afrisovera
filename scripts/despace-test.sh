#!/usr/bin/env bash
# Verify the `despace <text>` command removes ALL ASCII whitespace from the inline
# text, concatenating the remaining characters. e.g. "hello world" -> "helloworld".
# DISTINCT from disemvowel (removes VOWELS, keeps spaces), squeeze (collapses
# adjacent repeats), slug/kebab/snake (REPLACE spaces with separators), dedup
# (dup LINES). Self-checking: python replicates the same whitespace filter.
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
OUT="${TMPDIR:-/tmp}/despace.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/despace.png" \
    "echo warmup" "despace hello world" "despace a b c" "despace nospace" \
    "despace" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "despace:|usage: despace" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def despace(t):
    return ''.join(c for c in t if not c.isspace())

cases = ["hello world", "a b c", "nospace"]
for text in cases:
    want = f"despace: {despace(text)}"
    if want not in s:
        print(f"DESPACE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin: spaces removed, everything else kept in order
assert despace("hello world") == "helloworld"
assert despace("a b c") == "abc"
assert despace("nospace") == "nospace"   # no whitespace -> unchanged

if "usage: despace <text>" not in s:
    print("DESPACE TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("DESPACE TEST PASSED  (whitespace removed exact vs python, non-space kept, "
      "empty->usage)")
PY
