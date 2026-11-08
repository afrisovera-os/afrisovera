#!/usr/bin/env bash
# Verify the `vowels` command counts vowels vs consonants in inline text (ASCII
# letters only; y is counted as a consonant) -- distinct from chars, which splits
# letters/digits/spaces/other but does not distinguish vowels. The harness counts
# aeiou (case-insensitive) vs other letters in python, so it is self-checking:
# "Hello World"->3/7, "AEIOU"->5/0, "xyz"->0/3 (y is a consonant), "12345"->0/0.
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
OUT="${TMPDIR:-/tmp}/vowels.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/vowels.png" \
    "vowels Hello World" "vowels AEIOU" "vowels xyz" "vowels 12345" "vowels" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "vowels:|usage: vowels" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def counts(t):
    v = sum(1 for ch in t.lower() if ch in "aeiou")
    c = sum(1 for ch in t.lower() if "a" <= ch <= "z" and ch not in "aeiou")
    return v, c

for t in ["Hello World", "AEIOU", "xyz", "12345"]:
    v, c = counts(t)
    want = f"vowels: {v} consonants: {c} (letters: {v + c})"
    if want not in s:
        print(f"VOWELS TEST FAILED (missing '{want}' for {t!r})"); sys.exit(1)

if "usage: vowels <text>" not in s:
    print("VOWELS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("VOWELS TEST PASSED  (Hello World->3/7, AEIOU->5/0, xyz->0/3 (y consonant), 12345->0/0; empty->usage)")
PY
