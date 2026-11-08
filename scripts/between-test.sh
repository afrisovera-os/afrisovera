#!/usr/bin/env bash
# Verify `between <open> <close> <text>` extracts text between the first <open> and the next
# <close>: "( ) foo(bar)baz" -> bar, "[ ] a[xy]b" -> xy, "< > <html>hi" -> html, "{ } a{1}{2}"
# -> 1 (first close after first open), "( ) noparens" -> empty (open absent). Distinct from
# before/after/substr/cut. Self-checking: python finds open then the next close after it.
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
OUT="${TMPDIR:-/tmp}/between.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/between.png" \
    "echo warmup" "between ( ) foo(bar)baz" "between [ ] a[xy]b" \
    "between < > <html>hi" "between { } a{1}{2}" \
    "between ( ) noparens" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "between:|usage: between" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def between(o, c, text):
    i = text.find(o)
    if i < 0:
        return ""
    after = text[i + len(o):]
    j = after.find(c)
    return after[:j] if j >= 0 else ""    # DERIVED: open then next close after it

cases = [("(", ")", "foo(bar)baz"), ("[", "]", "a[xy]b"), ("<", ">", "<html>hi"),
         ("{", "}", "a{1}{2}"), ("(", ")", "noparens")]
for o, c, text in cases:
    want = "between: " + between(o, c, text)
    if want not in s:
        print(f"BETWEEN TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: parens, brackets, angle tag, first-close-wins, absent->empty
assert "between: bar" in s
assert "between: xy" in s
assert "between: html" in s
assert "between: 1" in s        # a{1}{2} -> first close after first open
assert "between: noparens" not in s   # open absent -> empty, NOT the whole text

print("BETWEEN TEST PASSED  (( ) foo(bar)baz->bar, [ ] a[xy]b->xy, < > <html>hi->html, "
      "{ } a{1}{2}->1, ( ) noparens->empty vs python open+next-close)")
PY
