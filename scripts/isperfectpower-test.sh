#!/usr/bin/env bash
# Verify the `isperfectpower <n>` command classifies perfect powers: n = a^b for
# some integer a>=2 and exponent b>=2 (1, 4, 8, 9, 16, 25, 27, 32, 36, 49, 64,
# 1000 IS; 2, 3, 10, 12, 17 NOT). It searches exponents b from 2 while 2^b <= n and
# reports the SMALLEST exponent found (64 -> 8^2, not 4^3 or 2^6). Generalises
# issquare (b == 2) and icbrt (cube root). Self-checking: python replicates the
# smallest-exponent search.
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
OUT="${TMPDIR:-/tmp}/isperfectpower.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 32 pins a high exponent (2^5); 64 pins the smallest-exponent choice (8^2).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isperfectpower.png" \
    "echo warmup" "isperfectpower 32" "isperfectpower 64" "isperfectpower 12" "isperfectpower 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isperfectpower:|usage: isperfectpower" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ipp(n):
    # replicate the kernel: smallest exponent b>=2 with an integer base a>=2, a^b == n
    if n == 1:
        return (1, 2)
    if n < 4:
        return None
    b = 2
    while (1 << b) <= n:
        # integer b-th root via binary search
        lo, hi = 1, n
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if mid ** b <= n: lo = mid
            else: hi = mid - 1
        if lo >= 2 and lo ** b == n:
            return (lo, b)
        b += 1
    return None

for n in [32, 64, 12, 1]:
    res = ipp(n)
    if res:
        a, b = res
        want = f"isperfectpower: {n} IS a perfect power ({a}^{b} = {n})"
    else:
        want = f"isperfectpower: {n} is NOT a perfect power"
    if want not in s:
        print(f"ISPERFECTPOWER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "isperfectpower: 32 IS a perfect power (2^5 = 32)" in s
assert "isperfectpower: 64 IS a perfect power (8^2 = 64)" in s   # smallest exponent, not 2^6
assert "isperfectpower: 12 is NOT a perfect power" in s
assert "isperfectpower: 1 IS a perfect power (1^2 = 1)" in s

print("ISPERFECTPOWER TEST PASSED  (32->2^5, 64->8^2 (smallest exp), 12 NOT, 1->1^2 "
      "vs python; generalises issquare/icbrt; distinct from ispowerful)")
PY
