#!/usr/bin/env bash
# Verify the `alnum <text>` command keeps ONLY the ASCII alphanumeric bytes
# (letters AND digits) of the inline text, dropping punctuation/spaces/symbols
# ("phone: 555-0199"->"phone5550199", "a-b_c 1!2"->"abc12"). The union of
# lettersonly + digitsonly, distinct from each (keeps BOTH). No alnum -> "(none)".
# Self-checking: python's str.isalnum filter reproduces the output.
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
OUT="${TMPDIR:-/tmp}/alnum.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/alnum.png" \
    "echo warmup" "alnum phone: 555-0199" "alnum a-b_c 1!2" \
    "alnum ...---..." "alnum" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "alnum:|usage: alnum" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def alnum(t):
    # kernel keeps ASCII [A-Za-z0-9]; the ASCII test inputs make str.isalnum equivalent.
    r = "".join(c for c in t if c.isalnum())
    return r if r else "(none)"

# shell-demo joins argv with single spaces, so the inline text matches exactly.
cases = ["phone: 555-0199", "a-b_c 1!2", "...---..."]
for t in cases:
    want = f"alnum: {alnum(t)}"
    if want not in s:
        print(f"ALNUM TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly
assert "alnum: phone5550199" in s   # letters AND digits kept, ':' '-' ' ' dropped
assert "alnum: abc12" in s           # '-' '_' ' ' '!' dropped, letters+digits kept
assert "alnum: (none)" in s          # pure punctuation -> nothing

# empty input -> usage
if "usage: alnum <text>" not in s:
    print("ALNUM TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("ALNUM TEST PASSED  ('phone: 555-0199'->phone5550199, 'a-b_c 1!2'->abc12, "
      "'...---...'->(none) vs python isalnum filter; empty -> usage)")
PY
