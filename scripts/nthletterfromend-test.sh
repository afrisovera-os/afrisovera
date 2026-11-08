#!/usr/bin/env bash
# Verify `nthletterfromend <n> <text>` returns the Nth letter from the END, skipping non-letters:
# 1 a b c->c, 2 a b c->b, 1 hello!->o, 2 a1b2c3->b, 4 a b c->(none). Mirror of nthletter.
# Self-checking: python [c for c in text if c.isalpha()][-n].
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
OUT="${TMPDIR:-/tmp}/nthletterfromend.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nthletterfromend.png" \
    "echo warmup" "nthletterfromend 1 a b c" "nthletterfromend 2 a b c" "nthletterfromend 1 hello!" \
    "nthletterfromend 2 a1b2c3" "nthletterfromend 4 a b c" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nthletterfromend:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n, text):                    # DERIVED reference
    letters = [c for c in text if c.isalpha()]
    return "nthletterfromend: " + (letters[-n] if 1 <= n <= len(letters) else "(none)")

cases = [(1, "a b c"), (2, "a b c"), (1, "hello!"), (2, "a1b2c3"), (4, "a b c")]
for n, text in cases:
    w = want(n, text)
    if w not in s:
        print(f"NTHLETTERFROMEND TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: last letter, second-last, skip punctuation, skip digits, out-of-range
assert "nthletterfromend: c" in s
assert "nthletterfromend: b" in s
assert "nthletterfromend: o" in s      # "hello!" last letter, '!' skipped
assert "nthletterfromend: (none)" in s

print("NTHLETTERFROMEND TEST PASSED  (1 a b c->c, 2 a b c->b, 1 hello!->o, 2 a1b2c3->b, 4 a b c->(none) "
      "vs python letters[-n])")
PY
