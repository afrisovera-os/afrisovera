#!/usr/bin/env bash
# Verify `wordrange <a> <b> <text>` emits words a..b (1-based INCLUSIVE) of inline text:
# "2 4 a b c d e"->b c d, "1 2 a b c d"->a b, "3 10 a b c"->c (b clamped), "5 6 a b c"->(none)
# past end. Inline analogue of rangewords (FILE). Self-checking: python " ".join(words[a-1:b]).
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
OUT="${TMPDIR:-/tmp}/wordrange.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/wordrange.png" \
    "echo warmup" "wordrange 2 4 a b c d e" "wordrange 1 2 a b c d" \
    "wordrange 3 10 a b c" "wordrange 5 6 a b c" "wordrange 1 1 solo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "wordrange:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b, text):                 # DERIVED reference
    sel = text.split()[a - 1:b] if a >= 1 else []
    return "wordrange: " + (" ".join(sel) if sel else "(none)")

cases = [(2, 4, "a b c d e"), (1, 2, "a b c d"), (3, 10, "a b c"),
         (5, 6, "a b c"), (1, 1, "solo")]
for a, b, text in cases:
    w = want(a, b, text)
    if w not in s:
        print(f"WORDRANGE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mid-slice, prefix, upper-clamp, past-end, single
assert "wordrange: b c d" in s
assert "wordrange: a b" in s
assert "wordrange: c" in s            # b clamped to word count
assert "wordrange: (none)" in s       # a past end
assert "wordrange: solo" in s

print("WORDRANGE TEST PASSED  (2 4 ...->b c d, 1 2 ...->a b, 3 10 a b c->c, 5 6 a b c->(none), "
      "1 1 solo->solo vs python words[a-1:b])")
PY
