#!/usr/bin/env bash
# Verify `revmiddle <text>` reverses the middle chars of each word, keeping first+last (classic
# typoglycemia): "hello world" -> "hlleo wlrod", "own your device" -> "own yuor dcivee", words of
# length <= 3 unchanged (cat, a). Distinct from reveach (whole word) and firstlast (ends only).
# Self-checking: python reverses each word's interior for every reference.
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
OUT="${TMPDIR:-/tmp}/revmiddle.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/revmiddle.png" \
    "echo warmup" "revmiddle hello world" "revmiddle own your device" \
    "revmiddle cat a OS" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "revmiddle:|usage: revmiddle" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rm(w):  # mirror the kernel: reverse interior, keep first+last; len<=2 unchanged
    return w if len(w) <= 2 else w[0] + w[1:-1][::-1] + w[-1]

def line(text):
    return "revmiddle: " + " ".join(rm(w) for w in text.split())

cases = ["hello world", "own your device", "cat a OS"]
for text in cases:
    want = line(text)   # every word DERIVED by python, never hand-typed
    if want not in s:
        print(f"REVMIDDLE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: middle reversed, tagline words, short-word passthrough
assert "revmiddle: hlleo wlrod" in s
assert "revmiddle: own yuor dcivee" in s   # own(3)->own, your->yuor, device->dcivee
assert "revmiddle: cat a OS" in s          # all <= 3 chars -> unchanged

print("REVMIDDLE TEST PASSED  (hello world->hlleo wlrod, own your device->own yuor dcivee, "
      "cat a OS unchanged vs python interior-reverse)")
PY
