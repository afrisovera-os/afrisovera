#!/usr/bin/env bash
# Verify the `countword <word> <text>` command counts how many WHOLE whitespace-
# delimited words in the INLINE text exactly equal <word> (case-sensitive). The
# word-level companion of countchar; distinct from wordfreq (most frequent word
# in a VFS file), grepc (matching lines), and wc. Self-checking: python's
# str.split().count recomputes each expected count.
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
OUT="${TMPDIR:-/tmp}/countword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countword.png" \
    "echo warmup" "countword the the cat and the dog and the bird" \
    "countword and the cat and the dog and the bird" \
    "countword cat the cat and the dog and the bird" \
    "countword fish the cat and the dog" "countword the" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countword:|usage: countword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# (word, text) cases -> count of whole-word matches
cases = [
    ("the", "the cat and the dog and the bird"),
    ("and", "the cat and the dog and the bird"),
    ("cat", "the cat and the dog and the bird"),
    ("fish", "the cat and the dog"),   # absent -> 0
]
for w, t in cases:
    want = f"countword: {w} occurs {t.split().count(w)} times"
    if want not in s:
        print(f"COUNTWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable values explicitly
assert "countword: the occurs 3 times" in s
assert "countword: and occurs 2 times" in s
assert "countword: cat occurs 1 times" in s
assert "countword: fish occurs 0 times" in s   # absent word reports 0

# a bare word with no text -> usage
if "usage: countword <word> <text>" not in s:
    print("COUNTWORD TEST FAILED (missing usage line for no text)"); sys.exit(1)

print("COUNTWORD TEST PASSED  (the->3, and->2, cat->1, fish->0 vs python split().count; "
      "no-text -> usage)")
PY
