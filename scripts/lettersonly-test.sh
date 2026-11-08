#!/usr/bin/env bash
# Verify the `lettersonly <text>` command keeps ONLY the ASCII letter bytes of
# the inline text, dropping every other byte ("ab12cd34"->"abcd",
# "phone: 555-0199"->"phone"). The letter-keeping mirror of digitsonly; distinct
# from disemvowel (drops only vowels, keeps punct/spaces), despace, squeeze. No
# letters -> "(none)". Self-checking: python's str.isalpha filter reproduces it.
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
OUT="${TMPDIR:-/tmp}/lettersonly.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lettersonly.png" \
    "echo warmup" "lettersonly ab12cd34" "lettersonly phone: 555-0199" \
    "lettersonly 2026-08-07" "lettersonly" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lettersonly:|usage: lettersonly" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lettersonly(t):
    # kernel filters bytes, and shell-demo joins argv with single spaces, so the
    # inline text for "lettersonly phone: 555-0199" is exactly "phone: 555-0199".
    r = "".join(c for c in t if c.isalpha())
    return r if r else "(none)"

cases = ["ab12cd34", "phone: 555-0199", "2026-08-07"]
for t in cases:
    want = f"lettersonly: {lettersonly(t)}"
    if want not in s:
        print(f"LETTERSONLY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms explicitly
assert "lettersonly: abcd" in s      # ab12cd34 -> keeps a,b,c,d
assert "lettersonly: phone" in s      # digits/punct/space dropped
assert "lettersonly: (none)" in s     # 2026-08-07 -> no letters

# empty input -> usage
if "usage: lettersonly <text>" not in s:
    print("LETTERSONLY TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("LETTERSONLY TEST PASSED  (ab12cd34->abcd, 'phone: 555-0199'->phone, 2026-08-07->(none) "
      "vs python isalpha filter; empty -> usage)")
PY
