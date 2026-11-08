#!/usr/bin/env bash
# Verify the `sentencecap <text>` command capitalizes the first letter of EACH
# sentence (start of text or after . ! ?), leaving every other character untouched:
# "hello world. foo bar" -> "Hello world. Foo bar". Unlike sentencecase (caps only
# the very first letter and lowercases the rest), sentencecap leaves mid-sentence
# text as-is. Self-checking: python replicates the per-sentence capitalize.
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
OUT="${TMPDIR:-/tmp}/sentencecap.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# "hello WORLD. ok" shows mid-sentence text preserved (vs sentencecase lowercasing).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sentencecap.png" \
    "echo warmup" "sentencecap hello world. foo bar" "sentencecap hi! bye? ok" "sentencecap hello WORLD. ok" "sentencecap solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sentencecap:|usage: sentencecap" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sentencecap(t):
    out = []
    start = True
    for x in t:
        if x.isalpha():
            if start:
                out.append(x.upper()); start = False
            else:
                out.append(x)
        else:
            if x in ".!?":
                start = True
            out.append(x)
    return "".join(out)

for t in ["hello world. foo bar", "hi! bye? ok", "hello WORLD. ok", "solo"]:
    want = f"sentencecap: {sentencecap(t)}"
    if want not in s:
        print(f"SENTENCECAP TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly
assert "sentencecap: Hello world. Foo bar" in s   # each sentence start capitalized
assert "sentencecap: Hi! Bye? Ok" in s            # ! and ? are terminators too
assert "sentencecap: Hello WORLD. Ok" in s        # mid-sentence WORLD preserved (vs sentencecase)
assert "sentencecap: Solo" in s                   # single word / single sentence

print("SENTENCECAP TEST PASSED  (hello world. foo bar->Hello world. Foo bar, hi! bye? ok->Hi! Bye? Ok, "
      "hello WORLD. ok->Hello WORLD. Ok (mid-sentence preserved), solo->Solo vs python)")
PY
