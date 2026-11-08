#!/usr/bin/env bash
# Verify the `chunk <n> <text>` command splits text into fixed-width groups of n
# characters, separated by a single space (a final short group is kept): "3 abcdefgh"
# -> "abc def gh". Distinct from wrap (greedy word-wrap to a column width) and spaceout
# (a space between every character = the n==1 case). Self-checking: python replicates
# the fixed-width chunking.
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
OUT="${TMPDIR:-/tmp}/chunk.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "3 abcdefgh" pins the short final group ("gh"); "1 abc" the spaceout-equivalent edge.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/chunk.png" \
    "echo warmup" "chunk 3 abcdefgh" "chunk 2 abcdef" "chunk 4 abcdefgh" "chunk 1 abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "chunk:|usage: chunk" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def chunk(n, t):
    return " ".join(t[i:i+n] for i in range(0, len(t), n))

cases = [(3, "abcdefgh"), (2, "abcdef"), (4, "abcdefgh"), (1, "abc")]
for n, t in cases:
    want = f"chunk: {chunk(n, t)}"
    if want not in s:
        print(f"CHUNK TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "chunk: abc def gh" in s        # short final group kept
assert "chunk: ab cd ef" in s
assert "chunk: abcd efgh" in s
assert "chunk: a b c" in s             # n == 1 -> spaceout-equivalent

print("CHUNK TEST PASSED  (3 abcdefgh->abc def gh (short final), 2 abcdef->ab cd ef, "
      "4 abcdefgh->abcd efgh, 1 abc->a b c vs python; fixed-width, distinct from wrap/spaceout)")
PY
