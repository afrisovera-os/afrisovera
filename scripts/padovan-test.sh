#!/usr/bin/env bash
# Verify the `padovan` command computes the nth Padovan number (P0=P1=P2=1,
# P(n)=P(n-2)+P(n-3) -> 1,1,1,2,2,3,4,5,7,9,12,...) in u64 with an overflow guard.
# DISTINCT from tribonacci (three CONSECUTIVE terms) and fib: Padovan is a GAP
# recurrence that SKIPS the immediate predecessor, growing at the plastic ratio
# ~1.3247 (very slowly). Self-checking: python recomputes P(n) by the same
# recurrence; the u64 boundary is P(158) fits / P(159) overflows.
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
OUT="${TMPDIR:-/tmp}/padovan.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/padovan.png" \
    "padovan 0" "padovan 1" "padovan 2" "padovan 3" "padovan 4" "padovan 5" "padovan 6" \
    "padovan 10" "padovan 20" "padovan 158" "padovan 159" "padovan xy" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "padovan:|usage: padovan" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def P(n):
    if n < 3:
        return 1
    a, b, c = 1, 1, 1
    for _ in range(3, n + 1):
        a, b, c = b, c, a + b
    return c

for n in [0, 1, 2, 3, 4, 5, 6, 10, 20, 158]:
    want = f"padovan: P({n}) = {P(n)}"
    if want not in s:
        print(f"PADOVAN TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: P(158) fits, P(159) overflows.
M = 2**64 - 1
assert P(158) <= M and P(159) > M, "boundary assumption wrong"
if "padovan: P(159) overflows u64" not in s:
    print("PADOVAN TEST FAILED (missing P(159) overflow line)"); sys.exit(1)
if "usage: padovan <n>" not in s:
    print("PADOVAN TEST FAILED (missing usage line)"); sys.exit(1)

print("PADOVAN TEST PASSED  (P0..P20 & P(158) exact vs python, P(159) overflow-guarded, bad->usage)")
PY
