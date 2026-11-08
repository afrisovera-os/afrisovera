#!/usr/bin/env bash
# Verify the `countdistinct <text>` command counts the DISTINCT whitespace-delimited
# words in inline text ("N distinct of M words"), case-sensitive. The inline analogue of
# uniqwords (which reads a VFS file); distinct from countword (occurrences of ONE word)
# and unique (which REMOVES duplicate words). Self-checking: python replicates the count.
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
OUT="${TMPDIR:-/tmp}/countdistinct.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "a b a c b" pins a mixed ratio (3/5); "same same same" the all-duplicate case (1/3).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/countdistinct.png" \
    "echo warmup" "countdistinct a b a c b" "countdistinct hello hello world" "countdistinct one two three" "countdistinct same same same" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "countdistinct:|usage: countdistinct" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(t):
    words = t.split()
    return f"countdistinct: {len(set(words))} distinct of {len(words)} words"

for t in ["a b a c b", "hello hello world", "one two three", "same same same"]:
    want = line(t)
    if want not in s:
        print(f"COUNTDISTINCT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "countdistinct: 3 distinct of 5 words" in s     # a b a c b
assert "countdistinct: 2 distinct of 3 words" in s     # hello hello world
assert "countdistinct: 3 distinct of 3 words" in s     # all distinct
assert "countdistinct: 1 distinct of 3 words" in s     # all the same

print("COUNTDISTINCT TEST PASSED  (a b a c b->3/5, hello hello world->2/3, one two three->3/3, "
      "same same same->1/3 vs python; inline analogue of uniqwords, distinct from countword/unique)")
PY
