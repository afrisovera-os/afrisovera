#!/usr/bin/env bash
# Verify `sumdigitcubes <n>` sums the CUBES of decimal digits (A055012): 153->153 (1+125+27),
# 371->371, 9->729, 100->1, 0->0. Cube analogue of sumdigitsquares; distinct from digitsum/digitprod.
# Self-checking: python sum(int(d)**3 for d in str(n)).
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
OUT="${TMPDIR:-/tmp}/sumdigitcubes.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumdigitcubes.png" \
    "echo warmup" "sumdigitcubes 153" "sumdigitcubes 371" "sumdigitcubes 9" \
    "sumdigitcubes 100" "sumdigitcubes 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumdigitcubes:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    return f"sumdigitcubes: {n} -> {sum(int(d) ** 3 for d in str(n))}"

for n in [153, 371, 9, 100, 0]:
    w = want(n)
    if w not in s:
        print(f"SUMDIGITCUBES TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: two narcissistic fixed points, single digit, with zeros, zero
assert "sumdigitcubes: 153 -> 153" in s      # Armstrong fixed point
assert "sumdigitcubes: 371 -> 371" in s      # Armstrong fixed point
assert "sumdigitcubes: 9 -> 729" in s
assert "sumdigitcubes: 100 -> 1" in s
assert "sumdigitcubes: 0 -> 0" in s

print("SUMDIGITCUBES TEST PASSED  (153->153, 371->371, 9->729, 100->1, 0->0 vs python sum of digit cubes)")
PY
