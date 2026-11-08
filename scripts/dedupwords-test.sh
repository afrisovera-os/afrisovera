#!/usr/bin/env bash
# Verify `dedupwords <text>` collapses only CONSECUTIVE duplicate words: "the the cat cat
# cat sat"->"the cat sat", "a a b a"->"a b a" (non-adjacent a kept), "one two three"->
# unchanged. Word-level analogue of squeeze; distinct from uniqwords/countdistinct.
# Self-checking: python replicates the adjacent-dedup rule.
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
OUT="${TMPDIR:-/tmp}/dedupwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/dedupwords.png" \
    "echo warmup" "dedupwords the the cat cat cat sat" "dedupwords a a b a" "dedupwords one two three" "dedupwords own your your device device" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dedupwords:|usage: dedupwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dw(text):  # mirror the kernel: collapse only CONSECUTIVE duplicate words
    out = []; prev = None
    for w in text.split():
        if w == prev:
            continue
        out.append(w); prev = w
    return " ".join(out)

for t in ["the the cat cat cat sat", "a a b a", "one two three",
          "own your your device device"]:
    want = f"dedupwords: {dw(t)}"
    if want not in s:
        print(f"DEDUPWORDS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "dedupwords: the cat sat" in s               # runs collapsed
assert "dedupwords: a b a" in s                      # non-adjacent 'a' preserved
assert "dedupwords: one two three" in s              # no dups -> unchanged
assert "dedupwords: own your device" in s            # your your / device device collapsed

print("DEDUPWORDS TEST PASSED  (the the cat cat cat sat->the cat sat, a a b a->a b a "
      "(non-adjacent kept), one two three unchanged vs python)")
PY
