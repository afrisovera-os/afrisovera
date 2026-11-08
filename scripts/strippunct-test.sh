#!/usr/bin/env bash
# Verify the `strippunct <text>` command drops ASCII punctuation/symbols while
# KEEPING letters, digits, and spaces ("phone: 555-0199"->"phone 5550199",
# "Hello, World! 42"->"Hello World 42"). Distinct from alnum (which ALSO drops
# spaces) and despace (drops spaces, keeps punctuation). No kept bytes ->
# "(none)". Self-checking: python keeps alnum-or-space chars.
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
OUT="${TMPDIR:-/tmp}/strippunct.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/strippunct.png" \
    "echo warmup" "strippunct phone: 555-0199" "strippunct Hello, World! 42" \
    "strippunct !!!---" "strippunct" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "strippunct:|usage: strippunct" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def strippunct(t):
    r = "".join(c for c in t if c.isalnum() or c == " ")
    return r if r else "(none)"

# shell-demo joins argv with single spaces, so the inline text matches exactly.
cases = ["phone: 555-0199", "Hello, World! 42", "!!!---"]
for t in cases:
    want = f"strippunct: {strippunct(t)}"
    if want not in s:
        print(f"STRIPPUNCT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms (spaces kept, ':' '-' ',' '!' dropped)
assert "strippunct: phone 5550199" in s
assert "strippunct: Hello World 42" in s
assert "strippunct: (none)" in s      # pure punctuation -> nothing

# empty input -> usage
if "usage: strippunct <text>" not in s:
    print("STRIPPUNCT TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("STRIPPUNCT TEST PASSED  ('phone: 555-0199'->phone 5550199 (space kept), "
      "'Hello, World! 42'->Hello World 42, '!!!---'->(none) vs python; empty -> usage)")
PY
