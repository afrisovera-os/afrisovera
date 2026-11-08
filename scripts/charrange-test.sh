#!/usr/bin/env bash
# Verify `charrange <a> <b> <text>` emits chars a..b (1-based INCLUSIVE): "2 4 hello"->ell,
# "1 3 world"->wor, "3 10 abc"->c (b clamped), "6 8 hello"->(none) past end, "1 1 x"->x.
# Char-level analogue of wordrange; endpoint complement to substr. Self-checking: python text[a-1:b].
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
OUT="${TMPDIR:-/tmp}/charrange.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/charrange.png" \
    "echo warmup" "charrange 2 4 hello" "charrange 1 3 world" "charrange 3 10 abc" \
    "charrange 6 8 hello" "charrange 1 1 x" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "charrange:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b, text):                 # DERIVED reference
    sub = text[a - 1:b] if a >= 1 else ""
    return "charrange: " + (sub if sub else "(none)")

cases = [(2, 4, "hello"), (1, 3, "world"), (3, 10, "abc"), (6, 8, "hello"), (1, 1, "x")]
for a, b, text in cases:
    w = want(a, b, text)
    if w not in s:
        print(f"CHARRANGE TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mid-slice, prefix, upper-clamp, past-end, single
assert "charrange: ell" in s
assert "charrange: wor" in s
assert "charrange: c" in s            # b clamped to length
assert "charrange: (none)" in s       # a past end
assert "charrange: x" in s

print("CHARRANGE TEST PASSED  (2 4 hello->ell, 1 3 world->wor, 3 10 abc->c, 6 8 hello->(none), "
      "1 1 x->x vs python text[a-1:b])")
PY
