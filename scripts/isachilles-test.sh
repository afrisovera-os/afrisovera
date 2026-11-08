#!/usr/bin/env bash
# Verify the `isachilles <n>` command classifies ACHILLES numbers: POWERFUL (every
# prime factor exponent >= 2) but NOT a perfect power (not a^b, a>=2, b>=2). 72, 108,
# 200, 288, 800 ARE (smallest is 72); 4=2^2 and 36=6^2 are powerful but ARE perfect
# powers (NOT); 6=2*3 is not powerful (NOT). A novel COMBINATION of the ispowerful and
# isperfectpower predicates. Self-checking: python replicates both predicates.
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
OUT="${TMPDIR:-/tmp}/isachilles.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 72 = smallest Achilles; 4 pins the "a perfect power" reason; 6 pins "not powerful".
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isachilles.png" \
    "echo warmup" "isachilles 72" "isachilles 4" "isachilles 6" "isachilles 36" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isachilles:|usage: isachilles" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_powerful(n):
    x = n; d = 2
    while d * d <= x:
        if x % d == 0:
            e = 0
            while x % d == 0:
                x //= d; e += 1
            if e < 2:
                return False
        d += 1
    return x <= 1

def is_perfect_power(n):
    if n == 1: return True
    if n < 4: return False
    b = 2
    while (1 << b) <= n:
        lo, hi = 1, n
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if mid ** b <= n: lo = mid
            else: hi = mid - 1
        if lo >= 2 and lo ** b == n:
            return True
        b += 1
    return False

def line(n):
    powerful = is_powerful(n)
    pp = is_perfect_power(n)
    if powerful and not pp:
        return f"isachilles: {n} IS an Achilles number (powerful but not a perfect power)"
    reason = "not powerful" if not powerful else "a perfect power"
    return f"isachilles: {n} is NOT an Achilles number ({reason})"

for n in [72, 4, 6, 36]:
    want = line(n)
    if want not in s:
        print(f"ISACHILLES TEST FAILED (missing '{want}')"); sys.exit(1)

# pin all three output shapes explicitly
assert "isachilles: 72 IS an Achilles number (powerful but not a perfect power)" in s
assert "isachilles: 4 is NOT an Achilles number (a perfect power)" in s      # 2^2 powerful but a perfect power
assert "isachilles: 6 is NOT an Achilles number (not powerful)" in s          # 2*3 not powerful
assert "isachilles: 36 is NOT an Achilles number (a perfect power)" in s      # 6^2

print("ISACHILLES TEST PASSED  (72 IS (2^3*3^2), 4/36 NOT (perfect power), 6 NOT (not "
      "powerful) vs python; combines ispowerful AND !isperfectpower)")
PY
