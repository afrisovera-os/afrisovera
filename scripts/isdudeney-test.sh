#!/usr/bin/env bash
# Verify the `isdudeney <n>` command classifies Dudeney numbers: a perfect cube
# whose digit SUM equals its cube ROOT (512 = 8^3 and 5+1+2 = 8; 4913 = 17^3 and
# 4+9+1+3 = 17; also 1, 5832, 17576, 19683). Reuses icbrt's u128-cube binary
# search for the perfect-cube check. Distinct from icbrt (just the root), digitsum
# (just the sum), and armstrong/isfactorion. Self-checking: python cube root +
# digit sum.
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
OUT="${TMPDIR:-/tmp}/isdudeney.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 27 is a perfect cube whose digit sum != root; 100 is not a cube at all.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isdudeney.png" \
    "echo warmup" "isdudeney 512" "isdudeney 17576" "isdudeney 27" "isdudeney 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isdudeney:|usage: isdudeney" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def icbrt(n):
    lo, hi = 0, 2_642_246
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if mid ** 3 <= n: lo = mid
        else: hi = mid - 1
    return lo

def dsum(n):
    return sum(int(d) for d in str(n))

# build the exact expected line for each case (three shapes)
for n in [512, 17576, 27, 100]:
    r = icbrt(n)
    perfect = r ** 3 == n
    d = dsum(n)
    if perfect and d == r:
        want = f"isdudeney: {n} IS a Dudeney number ({r}^3 and digit sum {d})"
    elif perfect:
        want = f"isdudeney: {n} is NOT a Dudeney number ({r}^3 but digit sum {d} != {r})"
    else:
        want = f"isdudeney: {n} is NOT a Dudeney number (not a perfect cube)"
    if want not in s:
        print(f"ISDUDENEY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isdudeney: 512 IS a Dudeney number (8^3 and digit sum 8)" in s
assert "isdudeney: 17576 IS a Dudeney number (26^3 and digit sum 26)" in s
assert "isdudeney: 27 is NOT a Dudeney number (3^3 but digit sum 9 != 3)" in s   # perfect cube, wrong dsum
assert "isdudeney: 100 is NOT a Dudeney number (not a perfect cube)" in s        # not a cube

print("ISDUDENEY TEST PASSED  (512/17576 IS, 27 perfect-cube-but-dsum-mismatch, 100 "
      "not-a-cube vs python; distinct from icbrt/digitsum)")
PY
