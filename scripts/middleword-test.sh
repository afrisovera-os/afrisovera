#!/usr/bin/env bash
# Verify the `middleword <text>` command emits the middle word(s): an odd word
# count gives the single middle word ("alpha beta gamma" -> "beta"), an even count
# gives the two middle words ("a b c d" -> "b c"), mirroring middlechar. The word-
# level analogue of middlechar; distinct from wordn. Self-checking: python replicates.
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
OUT="${TMPDIR:-/tmp}/middleword.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 3 words (single middle), 4 words (two middle), 5 words (single), 1 word.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/middleword.png" \
    "echo warmup" "middleword alpha beta gamma" "middleword own your device future" "middleword a b c d e" "middleword solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "middleword:|usage: middleword" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def middleword(t):
    w = t.split()
    n = len(w)
    if n % 2 == 1:
        return w[(n - 1) // 2]
    return " ".join(w[n // 2 - 1:n // 2 + 1])

for t in ["alpha beta gamma", "own your device future", "a b c d e", "solo"]:
    want = f"middleword: {middleword(t)}"
    if want not in s:
        print(f"MIDDLEWORD TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "middleword: beta" in s                # 3 words -> single middle
assert "middleword: your device" in s         # 4 words -> two middle words
assert "middleword: c" in s                   # 5 words -> single middle
assert "middleword: solo" in s                # 1 word -> itself

print("MIDDLEWORD TEST PASSED  (alpha beta gamma->beta, own your device future->your device, "
      "a b c d e->c, solo->solo vs python; middle word(s), mirrors middlechar)")
PY
