#!/usr/bin/env bash
# Verify `sumdigitsquares <n>` sums the SQUARES of decimal digits (A003132, happy step):
# 23->13 (4+9), 7->49, 100->1, 999->243, 0->0. Distinct from digitsum (sum)/digitprod (product).
# Self-checking: python sum(int(d)**2 for d in str(n)).
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
OUT="${TMPDIR:-/tmp}/sumdigitsquares.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumdigitsquares.png" \
    "echo warmup" "sumdigitsquares 23" "sumdigitsquares 7" "sumdigitsquares 100" \
    "sumdigitsquares 999" "sumdigitsquares 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumdigitsquares:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference
    return f"sumdigitsquares: {n} -> {sum(int(d) ** 2 for d in str(n))}"

for n in [23, 7, 100, 999, 0]:
    w = want(n)
    if w not in s:
        print(f"SUMDIGITSQUARES TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: two-digit, single-digit, with zeros, max-digits, zero
assert "sumdigitsquares: 23 -> 13" in s
assert "sumdigitsquares: 7 -> 49" in s
assert "sumdigitsquares: 100 -> 1" in s
assert "sumdigitsquares: 999 -> 243" in s
assert "sumdigitsquares: 0 -> 0" in s

print("SUMDIGITSQUARES TEST PASSED  (23->13, 7->49, 100->1, 999->243, 0->0 vs python sum of digit squares)")
PY
