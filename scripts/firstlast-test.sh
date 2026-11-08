#!/usr/bin/env bash
# Verify the `firstlast <text>` command keeps the first and last character of each
# whitespace-separated word, space-separated ("hello world" -> "ho wd"). A single-
# character word repeats it ("a" -> "aa"). Distinct from initials (first only) and
# lastletters (last only). Self-checking: python replicates w[0]+w[-1] per word.
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
OUT="${TMPDIR:-/tmp}/firstlast.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# Multi-word + single word + single-char words exercise the edges.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/firstlast.png" \
    "echo warmup" "firstlast hello world" "firstlast own your device" "firstlast hi" "firstlast a b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "firstlast:|usage: firstlast" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def firstlast(t):
    return " ".join(w[0] + w[-1] for w in t.split())

for t in ["hello world", "own your device", "hi", "a b"]:
    want = f"firstlast: {firstlast(t)}"
    if want not in s:
        print(f"FIRSTLAST TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "firstlast: ho wd" in s              # hello->ho, world->wd
assert "firstlast: on yr de" in s           # own->on, your->yr, device->de
assert "firstlast: hi" in s                 # hi -> h+i
assert "firstlast: aa bb" in s              # single-char words repeat: a->aa, b->bb

print("FIRSTLAST TEST PASSED  (hello world->ho wd, own your device->on yr de, hi->hi, a b->aa bb "
      "vs python; first+last char of each word)")
PY
