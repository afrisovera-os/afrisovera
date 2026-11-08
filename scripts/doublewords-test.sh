#!/usr/bin/env bash
# Verify the `doublewords <text>` command repeats each whitespace-separated word
# twice, space-separated: "hello world" -> "hello hello world world". The word-level
# analogue of double (each char twice); distinct from repeat (whole text n times).
# Self-checking: python replicates " ".join(w for w in t.split() for _ in range(2)).
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
OUT="${TMPDIR:-/tmp}/doublewords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# Single word + multi-word + single char exercise the spacing edges.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/doublewords.png" \
    "echo warmup" "doublewords hello world" "doublewords own your device" "doublewords hi" "doublewords a b" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "doublewords:|usage: doublewords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def doublewords(t):
    return " ".join(w for w in t.split() for _ in range(2))

for t in ["hello world", "own your device", "hi", "a b"]:
    want = f"doublewords: {doublewords(t)}"
    if want not in s:
        print(f"DOUBLEWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "doublewords: hello hello world world" in s
assert "doublewords: own own your your device device" in s
assert "doublewords: hi hi" in s          # single word -> twice
assert "doublewords: a a b b" in s        # single-char words

print("DOUBLEWORDS TEST PASSED  (hello world->hello hello world world, own your device->own own your your "
      "device device, hi->hi hi, a b->a a b b vs python; each word twice)")
PY
