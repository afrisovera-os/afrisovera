#!/usr/bin/env bash
# Verify the `swappairs <text>` command swaps ADJACENT character pairs across the
# whole inline text ("abcd"->"badc", "abcde"->"badce" -- a lone final char at an
# odd length stays). Distinct from rev (reverses the ENTIRE string), reveach
# (reverses each word's chars), and rwords (reverses word ORDER). Self-checking:
# python swaps bytes pairwise.
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
OUT="${TMPDIR:-/tmp}/swappairs.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/swappairs.png" \
    "echo warmup" "swappairs abcd" "swappairs abcde" \
    "swappairs AfriSovera" "swappairs" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "swappairs:|usage: swappairs" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def swappairs(t):
    b = list(t)
    i = 0
    while i + 1 < len(b):
        b[i], b[i + 1] = b[i + 1], b[i]
        i += 2
    return "".join(b)

for t in ["abcd", "abcde", "AfriSovera"]:
    want = f"swappairs: {swappairs(t)}"
    if want not in s:
        print(f"SWAPPAIRS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable transforms
assert "swappairs: badc" in s        # abcd -> ba dc
assert "swappairs: badce" in s        # abcde -> ba dc e (odd tail 'e' stays)
assert "swappairs: fAiroSevar" in s   # AfriSovera -> fA ir oS ev ar

# empty input -> usage
if "usage: swappairs <text>" not in s:
    print("SWAPPAIRS TEST FAILED (missing usage line for empty input)"); sys.exit(1)

print("SWAPPAIRS TEST PASSED  (abcd->badc, abcde->badce [odd tail stays], "
      "AfriSovera->fAiroSevar vs python; empty -> usage)")
PY
