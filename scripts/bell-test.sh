#!/usr/bin/env bash
# Verify the `bell` command computes the nth Bell number (the count of ways to
# PARTITION a set of n elements) in u64 via the Bell (Aitken) triangle, with an
# overflow guard. B(0)=1,1,2,5,15,52,203,877,4140,21147,115975,... Distinct from
# factorial and the linear recurrences (a 2D triangle, not a fixed-order
# recurrence). Self-checking: python recomputes B(n) via the SAME triangle; the
# u64 boundary is B(25) fits / B(26) overflows (Bell grows super-exponentially).
# NOTE: B(n) is the LAST element of row (n-1), so building row n (which would
# compute B(n+1)) must be avoided -- else B(25) wrongly reports overflow.
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
OUT="${TMPDIR:-/tmp}/bell.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/bell.png" \
    "bell 0" "bell 1" "bell 2" "bell 3" "bell 5" "bell 10" "bell 15" \
    "bell 25" "bell 26" "bell foo" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "bell:|usage: bell" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def bell(n):
    row = [1]
    for _ in range(n):
        newrow = [row[-1]]
        for x in row:
            newrow.append(newrow[-1] + x)
        row = newrow
    return row[0]

for n in [0, 1, 2, 3, 5, 10, 15, 25]:
    want = f"bell: B({n}) = {bell(n)}"
    if want not in s:
        print(f"BELL TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: B(25) fits, B(26) overflows.
M = 2**64 - 1
assert bell(25) <= M and bell(26) > M, "boundary assumption wrong"
if "bell: B(26) overflows u64" not in s:
    print("BELL TEST FAILED (missing B(26) overflow line)"); sys.exit(1)
if "usage: bell <n>" not in s:
    print("BELL TEST FAILED (missing usage line)"); sys.exit(1)

print("BELL TEST PASSED  (B0..B15 & B(25) exact vs python, B(26) overflow-guarded, bad->usage)")
PY
