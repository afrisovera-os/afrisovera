#!/usr/bin/env bash
# Verify `longestrun <text>` reports the longest run of identical consecutive chars (first run
# wins a tie): "aaabbbbcc" -> 'b' x 4, "abc" -> 'a' x 1, "aabbaa" -> 'a' x 2, "zzzz" -> 'z' x 4,
# "mississippi" -> 's' x 2. Distinct from squeeze/rle. Self-checking: python scans runs.
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
OUT="${TMPDIR:-/tmp}/longestrun.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/longestrun.png" \
    "echo warmup" "longestrun aaabbbbcc" "longestrun abc" "longestrun aabbaa" \
    "longestrun zzzz" "longestrun mississippi" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "longestrun:|usage: longestrun" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lr(text):
    best_c, best_n = text[0], 1
    cur_c, cur_n = text[0], 1
    for c in text[1:]:
        if c == cur_c: cur_n += 1
        else: cur_c, cur_n = c, 1
        if cur_n > best_n:                    # strictly greater -> FIRST run wins a tie
            best_n, best_c = cur_n, cur_c
    return best_c, best_n

for text in ["aaabbbbcc", "abc", "aabbaa", "zzzz", "mississippi"]:
    c, n = lr(text)
    want = f"longestrun: '{c}' x {n}"         # DERIVED
    if want not in s:
        print(f"LONGESTRUN TEST FAILED (text={text} missing '{want}')"); sys.exit(1)

# pin the notable branches: max run b4, all-1 first char, tie->first, all same, first max in mississippi
assert "longestrun: 'b' x 4" in s
assert "longestrun: 'a' x 1" in s
assert "longestrun: 'a' x 2" in s
assert "longestrun: 'z' x 4" in s
assert "longestrun: 's' x 2" in s

print("LONGESTRUN TEST PASSED  (aaabbbbcc->'b'x4, abc->'a'x1, aabbaa->'a'x2, zzzz->'z'x4, "
      "mississippi->'s'x2 vs python run scan)")
PY
