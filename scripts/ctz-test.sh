#!/usr/bin/env bash
# Verify `ctz <n>` counts TRAILING ZERO bits (2-adic valuation): 8->3, 12->2, 6->1, 1->0, 1024->10,
# and 0 reported specially. Distinct from ilog2 (high bit), popcount (set bits). Self-checking:
# python (n & -n).bit_length()-1 for n>0.
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
OUT="${TMPDIR:-/tmp}/ctz.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ctz.png" \
    "echo warmup" "ctz 8" "ctz 12" "ctz 6" "ctz 1024" "ctz 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ctz:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(n):                          # DERIVED reference: trailing zero bit count
    t = (n & -n).bit_length() - 1
    plural = "" if t == 1 else "s"
    return f"ctz: {n} has {t} trailing zero bit{plural} (2^{t} divides it)"

for n in (8, 12, 6, 1024):
    w = want(n)
    if w not in s:
        print(f"CTZ TEST FAILED (missing '{w}')"); sys.exit(1)

assert "ctz: 8 has 3 trailing zero bits (2^3 divides it)" in s
assert "ctz: 12 has 2 trailing zero bits (2^2 divides it)" in s
assert "ctz: 6 has 1 trailing zero bit (2^1 divides it)" in s
assert "ctz: 1024 has 10 trailing zero bits (2^10 divides it)" in s
assert "ctz: 0 has no set bit (all 64 bits are zero)" in s

print("CTZ TEST PASSED  (8->3, 12->2, 6->1, 1024->10, 0->special vs python trailing-zero count)")
PY
