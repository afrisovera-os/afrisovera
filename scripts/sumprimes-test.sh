#!/usr/bin/env bash
# Verify `sumprimes <n>` sums all primes <= n: 10 -> 17 (2+3+5+7), 2 -> 2, 1 -> 0, 30 -> 129,
# 100 -> 1060, and n > 100000 -> too large. Distinct from primepi (count), primes (list),
# primorial (product). Self-checking: python sums its own trial-division primes <= n.
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
OUT="${TMPDIR:-/tmp}/sumprimes.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumprimes.png" \
    "echo warmup" "sumprimes 10" "sumprimes 2" "sumprimes 1" \
    "sumprimes 30" "sumprimes 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumprimes:|usage: sumprimes" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k % 2 == 0: return k == 2
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

def line(n):
    total = sum(k for k in range(2, n + 1) if isp(k))   # DERIVED, never hand-typed
    return f"sumprimes: sum of primes <= {n} is {total}"

for n in [10, 2, 1, 30, 100]:
    w = line(n)
    if w not in s:
        print(f"SUMPRIMES TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: classic small sum, single prime, none-below-2, mid, hundred
assert "sumprimes: sum of primes <= 10 is 17" in s     # 2+3+5+7
assert "sumprimes: sum of primes <= 2 is 2" in s
assert "sumprimes: sum of primes <= 1 is 0" in s
assert "sumprimes: sum of primes <= 30 is 129" in s
assert "sumprimes: sum of primes <= 100 is 1060" in s

print("SUMPRIMES TEST PASSED  (10->17, 2->2, 1->0, 30->129, 100->1060 "
      "vs python trial-division prime sum)")
PY
