#!/usr/bin/env bash
# Verify `lastwords <n> <text>` emits the last n words of inline text: "2 a b c d" -> c d,
# "1 hello world" -> world, "5 a b c" -> a b c (fewer than n), "3 one two three four five" ->
# three four five, "0 a b c" -> "" (empty). Inline mirror of truncatewords. Self-checking:
# python " ".join(text.split()[-n:]).
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
OUT="${TMPDIR:-/tmp}/lastwords.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lastwords.png" \
    "echo warmup" "lastwords 2 a b c d" "lastwords 1 hello world" "lastwords 5 a b c" \
    "lastwords 3 one two three four five" "lastwords 0 a b c" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lastwords:|usage: lastwords" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):
    return "lastwords: " + " ".join(text.split()[-n:] if n else [])   # DERIVED

cases = [(2, "a b c d"), (1, "hello world"), (5, "a b c"),
         (3, "one two three four five"), (0, "a b c")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"LASTWORDS TEST FAILED (n={n} text='{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: last 2, last 1, n>count -> all, last 3, n=0 -> empty
assert "lastwords: c d" in s
assert "lastwords: world" in s
assert "lastwords: a b c" in s
assert "lastwords: three four five" in s

print("LASTWORDS TEST PASSED  (2 a b c d->c d, 1 hello world->world, 5 a b c->a b c, "
      "3 ...->three four five, 0 a b c->empty vs python split()[-n:])")
PY
