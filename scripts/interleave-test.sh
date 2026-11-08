#!/usr/bin/env bash
# Verify the `interleave <a> <b>` command weaves the characters of two whitespace-
# separated words (a0 b0 a1 b1 ...), appending the tail of the longer word once the
# shorter is exhausted. "abc xyz"->"axbycz", "ab wxyz"->"awbxyz", "abcd xy"->"axbycd".
# Distinct from anagram (compares two words), swappairs (pairs of one word), rotate.
# Self-checking: python replicates the index-weave-then-tail.
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
OUT="${TMPDIR:-/tmp}/interleave.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "ab wxyz" and "abcd xy" pin the two tail directions; "solo" pins the usage branch.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/interleave.png" \
    "echo warmup" "interleave abc xyz" "interleave ab wxyz" "interleave abcd xy" "interleave solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "interleave:|usage: interleave" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def interleave(a, b):
    out = []
    for i in range(max(len(a), len(b))):
        if i < len(a): out.append(a[i])
        if i < len(b): out.append(b[i])
    return "".join(out)

for a, b in [("abc", "xyz"), ("ab", "wxyz"), ("abcd", "xy")]:
    want = f"interleave: {interleave(a, b)}"
    if want not in s:
        print(f"INTERLEAVE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results + the usage branch (single word)
assert "interleave: axbycz" in s        # equal length
assert "interleave: awbxyz" in s        # b longer -> b tail appended
assert "interleave: axbycd" in s        # a longer -> a tail appended
assert "usage: interleave <word1> <word2>" in s

print("INTERLEAVE TEST PASSED  (abc xyz->axbycz, ab wxyz->awbxyz (b tail), abcd xy->axbycd "
      "(a tail), solo->usage vs python; char-weave, distinct from anagram/swappairs)")
PY
