#!/usr/bin/env bash
# Verify the `partition` command computes the number of INTEGER PARTITIONS of n
# (ways to write n as a sum of positive integers, order-independent) in u64 via
# the classic coin-change DP, with an overflow guard. p(0)=1,1,2,3,5,7,11,...
# DISTINCT from `bell` (SET partitions of a labelled set, which grow far faster).
# Self-checking: python recomputes p(n) via the SAME DP; the u64 boundary is
# p(416) fits / p(417) overflows (p(n) grows sub-exponentially).
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
OUT="${TMPDIR:-/tmp}/partition.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/partition.png" \
    "partition 0" "partition 1" "partition 2" "partition 3" "partition 4" "partition 5" \
    "partition 10" "partition 20" "partition 50" "partition 100" \
    "partition 416" "partition 417" "partition zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "partition:|usage: partition" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def part(n):
    p = [0] * (n + 1)
    p[0] = 1
    for k in range(1, n + 1):
        for j in range(k, n + 1):
            p[j] += p[j - k]
    return p[n]

for n in [0, 1, 2, 3, 4, 5, 10, 20, 50, 100, 416]:
    want = f"partition: p({n}) = {part(n)}"
    if want not in s:
        print(f"PARTITION TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: p(416) fits, p(417) overflows.
M = 2**64 - 1
assert part(416) <= M and part(417) > M, "boundary assumption wrong"
if "partition: p(417) overflows u64" not in s:
    print("PARTITION TEST FAILED (missing p(417) overflow line)"); sys.exit(1)
if "usage: partition <n>" not in s:
    print("PARTITION TEST FAILED (missing usage line)"); sys.exit(1)

print("PARTITION TEST PASSED  (p0..p100 & p(416) exact vs python, p(417) overflow-guarded, bad->usage)")
PY
