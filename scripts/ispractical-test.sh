#!/usr/bin/env bash
# Verify the `ispractical <n>` command classifies PRACTICAL numbers: every integer
# 1..=n is a sum of DISTINCT divisors of n. Test: sort divisors ascending, keep a
# running reachable-sum s (start 0); each divisor d must satisfy d <= s+1, then s += d.
# 1,2,4,6,8,12,16,18,20,24,28 ARE; 3,5,7,9,10 NOT. Distinct from perfect (sigma class)
# and divisors (tau/sigma). Self-checking: python replicates the reachability test.
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
OUT="${TMPDIR:-/tmp}/ispractical.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 10 pins the classic non-practical case (divisors 1,2,5,10 -- 4 is unreachable).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispractical.png" \
    "echo warmup" "ispractical 12" "ispractical 10" "ispractical 18" "ispractical 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispractical:|usage: ispractical" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_practical(n):
    divs = sorted(d for d in range(1, n + 1) if n % d == 0)
    reach = 0
    for d in divs:
        if d > reach + 1:
            return False
        reach += d
    return True

def line(n):
    if is_practical(n):
        return f"ispractical: {n} IS practical (every m in 1..{n} is a sum of distinct divisors)"
    return f"ispractical: {n} is NOT practical"

for n in [12, 10, 18, 7]:
    want = line(n)
    if want not in s:
        print(f"ISPRACTICAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin both output shapes explicitly
assert "ispractical: 12 IS practical (every m in 1..12 is a sum of distinct divisors)" in s
assert "ispractical: 18 IS practical (every m in 1..18 is a sum of distinct divisors)" in s
assert "ispractical: 10 is NOT practical" in s     # divisors 1,2,5,10 -> 4 unreachable
assert "ispractical: 7 is NOT practical" in s       # prime -> divisors 1,7 -> 2 unreachable

print("ISPRACTICAL TEST PASSED  (12/18 IS, 10 NOT (can't make 4), 7 NOT (prime) vs python; "
      "divisor-reachability, distinct from perfect/divisors)")
PY
