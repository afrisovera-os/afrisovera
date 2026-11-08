#!/usr/bin/env bash
# Verify the `pell` command computes the nth Pell number (P0=0, P1=1,
# P(n)=2*P(n-1)+P(n-2) -- the "2*" coefficient, unlike the 1*prev+1*prev2
# fib/lucas and the three-unit-term tribonacci) in u64 with an overflow guard.
# Self-checking: python recomputes P(n) by the same recurrence and asserts
# P(0),P(1),P(2),P(5),P(10),P(20); the u64 boundary is P(51) fits / P(52)
# overflows (Pell grows at the silver ratio ~2.414, so it overflows far earlier
# than fib's n=93).
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
OUT="${TMPDIR:-/tmp}/pell.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pell.png" \
    "pell 0" "pell 1" "pell 2" "pell 5" "pell 10" "pell 20" \
    "pell 51" "pell 52" "pell abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pell:|usage: pell" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pell(n):
    if n == 0:
        return 0
    a, b = 0, 1
    for _ in range(2, n + 1):
        a, b = b, 2 * b + a
    return b

for n in [0, 1, 2, 5, 10, 20, 51]:
    want = f"pell: P({n}) = {pell(n)}"
    if want not in s:
        print(f"PELL TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: P(51) fits, P(52) overflows.
M = 2**64 - 1
assert pell(51) <= M and pell(52) > M, "boundary assumption wrong"
if "pell: P(52) overflows u64" not in s:
    print("PELL TEST FAILED (missing P(52) overflow line)"); sys.exit(1)
if "usage: pell <n>" not in s:
    print("PELL TEST FAILED (missing usage line)"); sys.exit(1)

print("PELL TEST PASSED  (P0..P20 & P(51) exact vs python, P(52) overflow-guarded, bad->usage)")
PY
