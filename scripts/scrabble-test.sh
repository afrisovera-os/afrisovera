#!/usr/bin/env bash
# Verify `scrabble <text>` sums English Scrabble tile values (case-insensitive, non-letters
# ignored): quiz -> 22, hello -> 8, jazz -> 29, a -> 1, AfriSovera -> 16. Distinct from
# wordvalue (positional a=1..z=26). Self-checking: python with the standard Scrabble table.
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
OUT="${TMPDIR:-/tmp}/scrabble.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/scrabble.png" \
    "echo warmup" "scrabble quiz" "scrabble hello" "scrabble jazz" \
    "scrabble a" "scrabble AfriSovera" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "scrabble:|usage: scrabble" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

VAL = {}
for letters, v in [("aeioulnstr",1),("dg",2),("bcmp",3),("fhvwy",4),("k",5),("jx",8),("qz",10)]:
    for c in letters: VAL[c] = v

def scr(text):
    return sum(VAL[c] for c in text.lower() if c in VAL)     # DERIVED via Scrabble table

for text in ["quiz", "hello", "jazz", "a", "AfriSovera"]:
    want = f"scrabble: {scr(text)}"
    if want not in s:
        print(f"SCRABBLE TEST FAILED (text={text} missing '{want}')"); sys.exit(1)

# pin the notable cases
assert "scrabble: 22" in s     # quiz = 10+1+1+10
assert "scrabble: 8" in s      # hello
assert "scrabble: 29" in s     # jazz = 8+1+10+10
assert "scrabble: 1" in s      # a
assert "scrabble: 16" in s     # AfriSovera

print("SCRABBLE TEST PASSED  (quiz->22, hello->8, jazz->29, a->1, AfriSovera->16 "
      "vs python Scrabble tile table)")
PY
