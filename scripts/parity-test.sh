#!/usr/bin/env bash
# Verify `parity <n>` reports the binary parity (set-bit count mod 2): 0->even(0), 1->odd(1),
# 3->even(2 bits), 7->odd(3 bits), 255->even(8 bits). Distinct from popcount (count) and ispernicious
# (count prime). Self-checking: python bin(n).count("1") % 2.
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
OUT="${TMPDIR:-/tmp}/parity.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/parity.png" \
    "echo warmup" "parity 0" "parity 1" "parity 3" "parity 7" "parity 255" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "parity:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference: set-bit count and its parity
    ones = bin(n).count("1")
    p = ones & 1
    return f"parity: {n} has {ones} set bits -> {'even' if p == 0 else 'odd'} parity ({p})"

for n in (0, 1, 3, 7, 255):
    w = want(n)
    if w not in s:
        print(f"PARITY TEST FAILED (missing '{w}')"); sys.exit(1)

assert "parity: 0 has 0 set bits -> even parity (0)" in s
assert "parity: 1 has 1 set bits -> odd parity (1)" in s
assert "parity: 3 has 2 set bits -> even parity (0)" in s
assert "parity: 7 has 3 set bits -> odd parity (1)" in s
assert "parity: 255 has 8 set bits -> even parity (0)" in s

print("PARITY TEST PASSED  (0->even, 1->odd, 3->even, 7->odd, 255->even vs python popcount mod 2)")
PY
