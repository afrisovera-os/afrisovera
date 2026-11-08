#!/usr/bin/env bash
# Verify `prevpow2 <n>` gives the largest power of two <= n (rounds DOWN, mirror of nextpow2):
# 100 -> 64 (2^6), 128 -> 128 (2^7), 1 -> 1 (2^0), 1000 -> 512 (2^9), 5 -> 4 (2^2); 0 ->
# undefined. Self-checking: python (n.bit_length()-1) exponent and 1<<e.
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
OUT="${TMPDIR:-/tmp}/prevpow2.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevpow2.png" \
    "echo warmup" "prevpow2 100" "prevpow2 128" "prevpow2 1" \
    "prevpow2 1000" "prevpow2 5" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevpow2:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    e = n.bit_length() - 1              # DERIVED floor log2
    return f"prevpow2: largest power of 2 <= {n} is {1 << e} (2^{e})"

for n in [100, 128, 1, 1000, 5]:
    w = line(n)
    if w not in s:
        print(f"PREVPOW2 TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable cases: round-down, exact power, 1, mid, small
assert "prevpow2: largest power of 2 <= 100 is 64 (2^6)" in s
assert "prevpow2: largest power of 2 <= 128 is 128 (2^7)" in s
assert "prevpow2: largest power of 2 <= 1 is 1 (2^0)" in s
assert "prevpow2: largest power of 2 <= 1000 is 512 (2^9)" in s
assert "prevpow2: largest power of 2 <= 5 is 4 (2^2)" in s

print("PREVPOW2 TEST PASSED  (100->64, 128->128, 1->1, 1000->512, 5->4 vs python floor-log2)")
PY
