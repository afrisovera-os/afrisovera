#!/usr/bin/env bash
# Verify `caesarf <shift> <file>` Caesar-shifts every ASCII letter of a VFS file by <shift> (rot-N),
# non-letters passing through -- the file analogue of caesar, generalizing rot13f. Tests shift 3,
# shift 13 (== rot13f), shift -3, shift 29 (== 3 mod 26), missing file. Self-checking: python shift.
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
OUT="${TMPDIR:-/tmp}/caesarf.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/caesarf.png" \
    "echo warmup" "caesarf 3 motd" "caesarf 13 motd" "caesarf -3 motd" \
    "caesarf 29 motd" "caesarf 3 nope.txt" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "caesarf:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

MOTD = "Welcome to AfriSovera OS -- a sovereign, capability-secured OS."

def caesar(t, shift):                 # DERIVED reference: shift letters within case
    k = ((shift % 26) + 26) % 26
    out = []
    for ch in t:
        o = ord(ch)
        if 65 <= o <= 90:    out.append(chr(65 + (o - 65 + k) % 26))
        elif 97 <= o <= 122: out.append(chr(97 + (o - 97 + k) % 26))
        else:                out.append(ch)
    return "".join(out)

for shift in (3, 13, -3, 29):
    want = "caesarf: " + caesar(MOTD, shift)
    if want not in s:
        print(f"CAESARF TEST FAILED (missing shift {shift}: '{want!r}')"); sys.exit(1)

# shift 13 must equal rot13f's output; shift 29 == shift 3
assert caesar(MOTD, 13) == "Jrypbzr gb NsevFbiren BF -- n fbirervta, pncnovyvgl-frpherq BF."
assert caesar(MOTD, 29) == caesar(MOTD, 3)
assert "caesarf: no such file: nope.txt" in s

print("CAESARF TEST PASSED  (shift 3/13/-3/29 on motd vs python; 13==rot13f, 29==3; nope.txt errors)")
PY
