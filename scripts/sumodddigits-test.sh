#!/usr/bin/env bash
# Verify `sumodddigits <n>` sums the ODD-valued decimal digits: 12345->9 (1+3+5), 2468->0 (all even),
# 13579->25, 135->9, 0->0. Complement of sumevendigits (sum even+sum odd = digitsum).
# Self-checking: python sum(int(d) for d in str(n) if int(d)%2==1).
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
OUT="${TMPDIR:-/tmp}/sumodddigits.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumodddigits.png" \
    "echo warmup" "sumodddigits 12345" "sumodddigits 2468" "sumodddigits 13579" \
    "sumodddigits 135" "sumodddigits 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumodddigits:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    return f"sumodddigits: {n} -> {sum(int(d) for d in str(n) if int(d) % 2 == 1)}"

for n in [12345, 2468, 13579, 135, 0]:
    w = want(n)
    if w not in s:
        print(f"SUMODDDIGITS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: mixed, all even (0), all odd, odd set, zero
assert "sumodddigits: 12345 -> 9" in s
assert "sumodddigits: 2468 -> 0" in s        # all even -> 0
assert "sumodddigits: 13579 -> 25" in s
assert "sumodddigits: 135 -> 9" in s
assert "sumodddigits: 0 -> 0" in s

print("SUMODDDIGITS TEST PASSED  (12345->9, 2468->0, 13579->25, 135->9, 0->0 vs python sum of odd digits)")
PY
