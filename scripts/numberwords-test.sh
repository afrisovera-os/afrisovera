#!/usr/bin/env bash
# Verify the `numberwords <text>` command prefixes each whitespace-delimited word
# with its 1-based index and a colon: "alpha beta gamma" -> "1:alpha 2:beta 3:gamma".
# The inline-word analogue of nl (which numbers FILE lines); distinct from
# wordlengths (per-word lengths). Self-checking: python replicates the enumerate.
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
OUT="${TMPDIR:-/tmp}/numberwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# A 10+ word phrase exercises the two-digit index; single word the 1: base case.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/numberwords.png" \
    "echo warmup" "numberwords alpha beta gamma" "numberwords own your device own your future" "numberwords a b c d e f g h i j k" "numberwords solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "numberwords:|usage: numberwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def numberwords(t):
    return " ".join(f"{i+1}:{w}" for i, w in enumerate(t.split()))

for t in ["alpha beta gamma", "own your device own your future",
          "a b c d e f g h i j k", "solo"]:
    want = f"numberwords: {numberwords(t)}"
    if want not in s:
        print(f"NUMBERWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "numberwords: 1:alpha 2:beta 3:gamma" in s
assert "numberwords: 1:own 2:your 3:device 4:own 5:your 6:future" in s
assert "numberwords: 1:a 2:b 3:c 4:d 5:e 6:f 7:g 8:h 9:i 10:j 11:k" in s  # two-digit index
assert "numberwords: 1:solo" in s                                          # single word

print("NUMBERWORDS TEST PASSED  (alpha beta gamma->1:alpha 2:beta 3:gamma, 11-word ->two-digit "
      "index 10:/11:, solo->1:solo vs python; 1-based per-word index prefix)")
PY
