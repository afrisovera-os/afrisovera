#!/usr/bin/env bash
# Verify the `isevil <n>` command classifies by the PARITY of the population count:
# n is EVIL when it has an EVEN number of 1-bits, ODIOUS when the count is ODD.
# evil: 0,3,5,6,9,10,12 ; odious: 1,2,4,7,8,11. Distinct from popcount (which COUNTS
# the bits) and ispernicious (whose test is whether the count is PRIME, not its parity).
# Self-checking: python replicates the parity classification.
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
OUT="${TMPDIR:-/tmp}/isevil.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 0 pins the zero-bits edge (EVIL); 7 pins ODIOUS; 6 pins EVIL; 255 pins a large even count.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isevil.png" \
    "echo warmup" "isevil 6" "isevil 7" "isevil 0" "isevil 255" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isevil:|usage: isevil" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    ones = bin(n).count("1")
    if ones % 2 == 0:
        return f"isevil: {n} IS evil ({ones} set bits, even)"
    return f"isevil: {n} is NOT evil -- ODIOUS ({ones} set bits, odd)"

for n in [6, 7, 0, 255]:
    want = line(n)
    if want not in s:
        print(f"ISEVIL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isevil: 6 IS evil (2 set bits, even)" in s
assert "isevil: 7 is NOT evil -- ODIOUS (3 set bits, odd)" in s
assert "isevil: 0 IS evil (0 set bits, even)" in s       # zero-bits edge
assert "isevil: 255 IS evil (8 set bits, even)" in s     # 0xff -> 8 ones, even

print("ISEVIL TEST PASSED  (6/0/255 EVIL even-popcount, 7/4 ODIOUS odd-popcount vs python; "
      "distinct from popcount COUNT and ispernicious PRIME-popcount)")
PY
