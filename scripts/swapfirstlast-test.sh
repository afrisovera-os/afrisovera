#!/usr/bin/env bash
# Verify the `swapfirstlast <text>` command swaps the FIRST and LAST whitespace-
# delimited words, leaving the middle words unchanged: "alpha beta gamma delta" ->
# "delta beta gamma alpha". The word-level analogue of swapends (first/last CHAR);
# distinct from rwords (reverses ALL words). Self-checking: python replicates.
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
OUT="${TMPDIR:-/tmp}/swapfirstlast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# The 6-word tagline shows the middle four words preserved (vs rwords full reverse).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swapfirstlast.png" \
    "echo warmup" "swapfirstlast alpha beta gamma delta" "swapfirstlast own your device own your future" "swapfirstlast hello world" "swapfirstlast solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swapfirstlast:|usage: swapfirstlast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def swapfirstlast(t):
    w = t.split()
    if len(w) >= 2:
        w[0], w[-1] = w[-1], w[0]
    return " ".join(w)

for t in ["alpha beta gamma delta", "own your device own your future",
          "hello world", "solo"]:
    want = f"swapfirstlast: {swapfirstlast(t)}"
    if want not in s:
        print(f"SWAPFIRSTLAST TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "swapfirstlast: delta beta gamma alpha" in s          # middle two preserved
assert "swapfirstlast: future your device own your own" in s  # 6 words, middle four intact
assert "swapfirstlast: world hello" in s                      # two words
assert "swapfirstlast: solo" in s                             # single word unchanged

print("SWAPFIRSTLAST TEST PASSED  (alpha beta gamma delta->delta beta gamma alpha, 6-word tagline "
      "middle preserved, hello world->world hello, solo->solo vs python)")
PY
