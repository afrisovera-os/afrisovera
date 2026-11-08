#!/usr/bin/env bash
# Verify the `swapwordpairs <text>` command swaps adjacent whitespace-delimited word
# pairs ("alpha beta gamma delta" -> "beta alpha delta gamma"); at an odd word count
# the lone final word stays ("a b c" -> "b a c"). The word-level analogue of swappairs
# (adjacent chars); distinct from rwords (full reverse). Self-checking: python matches.
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
OUT="${TMPDIR:-/tmp}/swapwordpairs.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 4 words (even), 5 words (odd tail stays), 2 words, 1 word.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapwordpairs.png" \
    "echo warmup" "swapwordpairs alpha beta gamma delta" "swapwordpairs own your device now here" "swapwordpairs hello world" "swapwordpairs solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapwordpairs:|usage: swapwordpairs" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def swapwordpairs(t):
    w = t.split()
    i = 0
    while i + 1 < len(w):
        w[i], w[i + 1] = w[i + 1], w[i]
        i += 2
    return " ".join(w)

for t in ["alpha beta gamma delta", "own your device now here", "hello world", "solo"]:
    want = f"swapwordpairs: {swapwordpairs(t)}"
    if want not in s:
        print(f"SWAPWORDPAIRS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "swapwordpairs: beta alpha delta gamma" in s              # 4 words, both pairs swapped
assert "swapwordpairs: your own now device here" in s            # 5 words, "here" stays
assert "swapwordpairs: world hello" in s                         # two words
assert "swapwordpairs: solo" in s                                # single word unchanged

print("SWAPWORDPAIRS TEST PASSED  (alpha beta gamma delta->beta alpha delta gamma, 5-word odd tail stays, "
      "hello world->world hello, solo->solo vs python)")
PY
