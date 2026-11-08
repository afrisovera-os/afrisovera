#!/usr/bin/env bash
# Verify the `rot18 <text>` command applies ROT13 to letters AND ROT5 to digits
# in one pass ("Room 101" -> "Ebbz 656"; "abc XYZ 789" -> "nop KLM 234"); other
# bytes pass through. Because both halves are self-inverse over disjoint sets,
# rot18 is self-inverse (rot18(rot18(x))==x) -- the test drives the round-trip
# in-kernel. Distinct from rot13 (letters only) and rot5 (digits only). Self-
# checking: python applies the combined map.
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
OUT="${TMPDIR:-/tmp}/rot18.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# "Ebbz 656" is rot18("Room 101"), so feeding it back exercises the self-inverse.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/rot18.png" \
    "echo warmup" "rot18 Room 101" "rot18 Ebbz 656" \
    "rot18 abc XYZ 789" "rot18" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "rot18:|usage: rot18" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def rot18(t):
    r = ""
    for c in t:
        o = ord(c)
        if 65 <= o <= 90:
            r += chr(65 + (o - 65 + 13) % 26)
        elif 97 <= o <= 122:
            r += chr(97 + (o - 97 + 13) % 26)
        elif 48 <= o <= 57:
            r += chr(48 + (o - 48 + 5) % 10)
        else:
            r += c
    return r

cases = ["Room 101", "Ebbz 656", "abc XYZ 789"]
for t in cases:
    want = f"rot18: {rot18(t)}"
    if want not in s:
        print(f"ROT18 TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms (letters via rot13, digits via rot5)
assert "rot18: Ebbz 656" in s        # "Room 101" -> letters rot13, digits rot5
assert "rot18: Room 101" in s         # self-inverse: rot18("Ebbz 656") -> "Room 101"
assert "rot18: nop KLM 234" in s      # both cases + digits

# self-inverse sanity in python
assert rot18(rot18("AfriSovera 2026")) == "AfriSovera 2026"

# empty input -> usage
if "usage: rot18 <text>" not in s:
    print("ROT18 TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("ROT18 TEST PASSED  (Room 101<->Ebbz 656 self-inverse, abc XYZ 789->nop KLM 234; "
      "rot13 letters + rot5 digits vs python; empty -> usage)")
PY
