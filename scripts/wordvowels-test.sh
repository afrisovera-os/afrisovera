#!/usr/bin/env bash
# Verify `wordvowels <text>` outputs the vowel count (a,e,i,o,u, both cases; y is a consonant)
# of each whitespace-delimited word, space-separated: "hello world" -> "2 1", "sky" -> "0",
# "AfriSovera OS" -> "5 1". The per-word companion of vowels (whole-text totals). Self-checking:
# python counts vowels per word for every reference.
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
OUT="${TMPDIR:-/tmp}/wordvowels.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordvowels.png" \
    "echo warmup" "wordvowels hello world" "wordvowels sky rhythm" \
    "wordvowels AfriSovera OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordvowels:|usage: wordvowels" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def wv(text):  # mirror the kernel: vowels aeiou (both cases), y excluded, per word
    return " ".join(str(sum(1 for c in w if c.lower() in "aeiou")) for w in text.split())

cases = ["hello world", "sky rhythm", "AfriSovera OS"]
for text in cases:
    want = f"wordvowels: {wv(text)}"   # every count DERIVED by python, never hand-typed
    if want not in s:
        print(f"WORDVOWELS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: mixed counts, an all-consonant (0), mixed-case + a 1
assert "wordvowels: 2 1" in s          # hello->2, world->1
assert "wordvowels: 0 0" in s          # sky->0, rhythm->0 (y is a consonant)
assert "wordvowels: 5 1" in s          # AfriSovera->5, OS->1 (case-insensitive)

print("WORDVOWELS TEST PASSED  (hello world->2 1, sky rhythm->0 0, AfriSovera OS->5 1 "
      "vs python per-word vowel counts)")
PY
