#!/usr/bin/env bash
# Verify `nthletter <n> <text>` returns the Nth letter skipping non-letters: "3 a b c d" -> c,
# "5 hello world" -> o, "2 a1b2c3" -> b, "1 hello" -> h, "10 abc" -> (none). Distinct from
# charat (file byte) and wordn (word). Self-checking: python [c for c in text if c.isalpha()].
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
OUT="${TMPDIR:-/tmp}/nthletter.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthletter.png" \
    "echo warmup" "nthletter 3 a b c d" "nthletter 5 hello world" "nthletter 2 a1b2c3" \
    "nthletter 1 hello" "nthletter 10 abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthletter:|usage: nthletter" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):
    letters = [c for c in text if c.isalpha()]                # DERIVED: skip non-letters
    return "nthletter: " + (letters[n - 1] if 1 <= n <= len(letters) else "(none)")

cases = [(3, "a b c d"), (5, "hello world"), (2, "a1b2c3"), (1, "hello"), (10, "abc")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"NTHLETTER TEST FAILED (n={n} text='{text}' missing '{w}')"); sys.exit(1)

# pin the notable branches: skip spaces, 5th of two words, skip digits, first, out-of-range
assert "nthletter: c" in s
assert "nthletter: o" in s
assert "nthletter: b" in s
assert "nthletter: h" in s
assert "nthletter: (none)" in s

print("NTHLETTER TEST PASSED  (3 a b c d->c, 5 hello world->o, 2 a1b2c3->b, 1 hello->h, "
      "10 abc->(none) vs python alpha filter)")
PY
