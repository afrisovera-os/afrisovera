#!/usr/bin/env bash
# Verify the `wordvalue <text>` command prints each word's alphabetic value (a/A=1..z/Z=26,
# non-letters 0), space-separated: "abc"->6, "hello world"->"52 72", "AfriSovera"->114,
# "Zoo"->56. Distinct from wordlengths (byte lengths) and digitsum. Self-checking: python
# computes every reference value from the letter positions (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/wordvalue.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordvalue.png" \
    "echo warmup" "wordvalue abc" "wordvalue hello world" "wordvalue AfriSovera" "wordvalue Zoo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordvalue:|usage: wordvalue" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def wv(text):  # mirror the kernel: per-word sum of 1-based letter positions
    out = []
    for w in text.split():
        out.append(str(sum(ord(c.lower()) - 96 for c in w if c.isalpha())))
    return " ".join(out)

for t in ["abc", "hello world", "AfriSovera", "Zoo"]:
    want = f"wordvalue: {wv(t)}"
    if want not in s:
        print(f"WORDVALUE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable results explicitly (derived above)
assert "wordvalue: 6" in s               # a+b+c = 1+2+3
assert "wordvalue: 52 72" in s            # hello=52, world=72
assert "wordvalue: 114" in s              # AfriSovera
assert "wordvalue: 56" in s               # Zoo = 26+15+15

print("WORDVALUE TEST PASSED  (abc->6, hello world->52 72, AfriSovera->114, Zoo->56 "
      "vs python letter-position sums)")
PY
