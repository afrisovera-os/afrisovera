#!/usr/bin/env bash
# Verify the `wordlengths <text>` command outputs the byte length of each
# whitespace-delimited word, space-separated. e.g. "hello world" -> "5 5",
# "AfriSovera OS" -> "10 2". DISTINCT from numeronym (first+middlecount+last),
# wc (file counts), acronym (initials), wordn (the Nth word). Self-checking:
# python joins str(len(w)) for each word.
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
OUT="${TMPDIR:-/tmp}/wordlengths.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordlengths.png" \
    "echo warmup" "wordlengths hello world" "wordlengths a bb ccc" \
    "wordlengths AfriSovera OS" "wordlengths x" "wordlengths" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordlengths:|usage: wordlengths" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def wl(t):
    return ' '.join(str(len(w)) for w in t.split())

cases = ["hello world", "a bb ccc", "AfriSovera OS", "x"]
for text in cases:
    want = f"wordlengths: {wl(text)}"
    if want not in s:
        print(f"WORDLENGTHS TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin incl. the multi-digit case
assert wl("hello world") == "5 5"
assert wl("a bb ccc") == "1 2 3"
assert wl("AfriSovera OS") == "10 2"   # 10 is two digits
assert wl("x") == "1"

if "usage: wordlengths <text>" not in s:
    print("WORDLENGTHS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("WORDLENGTHS TEST PASSED  (5 5 / 1 2 3 / 10 2 (multi-digit) / 1 exact vs "
      "python; empty->usage)")
PY
