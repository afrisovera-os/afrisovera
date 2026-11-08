#!/usr/bin/env bash
# Verify the `perrin` command computes the nth Perrin number (SAME gap recurrence
# as padovan, P(n)=P(n-2)+P(n-3), but seeds P(0)=3, P(1)=0, P(2)=2 -> 3,0,2,3,2,
# 5,5,7,10,12,17,...) in u64 with an overflow guard. DISTINCT from padovan (same
# recurrence, different seeds) exactly as lucas is distinct from fib. Self-checking:
# python recomputes P(n) by the same recurrence; the u64 boundary is P(157) fits /
# P(158) overflows (plastic ratio ~1.3247, same slow growth as padovan).
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
OUT="${TMPDIR:-/tmp}/perrin.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/perrin.png" \
    "perrin 0" "perrin 1" "perrin 2" "perrin 3" "perrin 4" "perrin 5" "perrin 6" \
    "perrin 10" "perrin 20" "perrin 157" "perrin 158" "perrin zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "perrin:|usage: perrin" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def P(n):
    if n == 0: return 3
    if n == 1: return 0
    if n == 2: return 2
    a, b, c = 3, 0, 2
    for _ in range(3, n + 1):
        a, b, c = b, c, a + b
    return c

for n in [0, 1, 2, 3, 4, 5, 6, 10, 20, 157]:
    want = f"perrin: P({n}) = {P(n)}"
    if want not in s:
        print(f"PERRIN TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: P(157) fits, P(158) overflows.
M = 2**64 - 1
assert P(157) <= M and P(158) > M, "boundary assumption wrong"
if "perrin: P(158) overflows u64" not in s:
    print("PERRIN TEST FAILED (missing P(158) overflow line)"); sys.exit(1)
if "usage: perrin <n>" not in s:
    print("PERRIN TEST FAILED (missing usage line)"); sys.exit(1)

print("PERRIN TEST PASSED  (P0..P20 & P(157) exact vs python, P(158) overflow-guarded, bad->usage)")
PY
