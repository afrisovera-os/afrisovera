#!/usr/bin/env bash
# Verify the `rot5 <text>` command shifts every ASCII digit by 5, wrapping
# within 0-9 (0<->5, 1<->6, ...); letters and everything else pass through.
# Like rot13 for digits, it is SELF-INVERSE. Distinct from rot13 (letters),
# rot47 (all printable), and caesar (a letter shift). Self-checking: python
# rotates the digits and confirms rot5(rot5(x)) == x.
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
OUT="${TMPDIR:-/tmp}/rot5.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "Room 656" is rot5("Room 101"), so feeding it back exercises the self-inverse.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot5.png" \
    "echo warmup" "rot5 Room 101" "rot5 Room 656" \
    "rot5 2026-08-07" "rot5 abc" "rot5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot5:|usage: rot5" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rot5(t):
    return "".join(chr((ord(c) - 48 + 5) % 10 + 48) if c.isdigit() else c for c in t)

cases = ["Room 101", "Room 656", "2026-08-07", "abc"]
for t in cases:
    want = f"rot5: {rot5(t)}"
    if want not in s:
        print(f"ROT5 TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms
assert "rot5: Room 656" in s        # "Room 101" -> digits 1,0,1 -> 6,5,6
assert "rot5: Room 101" in s         # self-inverse: rot5("Room 656") -> "Room 101"
assert "rot5: 7571-53-52" in s       # "2026-08-07" digits rotated, '-' kept
assert "rot5: abc" in s              # letters unchanged

# self-inverse sanity in python
assert rot5(rot5("2026-08-07")) == "2026-08-07"

# empty input -> usage
if "usage: rot5 <text>" not in s:
    print("ROT5 TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("ROT5 TEST PASSED  (Room 101<->Room 656 self-inverse, 2026-08-07->7571-53-52, "
      "letters unchanged vs python; empty -> usage)")
PY
