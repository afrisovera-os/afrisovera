#!/usr/bin/env bash
# Verify the `tribonacci` command computes the nth tribonacci number (T0=0, T1=0,
# T2=1, T(k)=T(k-1)+T(k-2)+T(k-3) -- a THREE-term recurrence, unlike the 2-term
# fib/lucas) in u64 with an overflow guard. The harness recomputes T(n) in python,
# so it is self-checking: T0=0,T1=0,T2=1,T3=1,T4=2,T5=4,T10=81,T20=35890; T(75)
# fits u64, T(76) overflows.
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
OUT="${TMPDIR:-/tmp}/tribonacci.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tribonacci.png" \
    "tribonacci 0" "tribonacci 1" "tribonacci 2" "tribonacci 3" "tribonacci 4" \
    "tribonacci 5" "tribonacci 10" "tribonacci 20" "tribonacci 75" "tribonacci 76" \
    "tribonacci abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tribonacci:|usage: tribonacci" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def trib(n):
    if n in (0, 1):
        return 0
    if n == 2:
        return 1
    a, b, c = 0, 0, 1
    for _ in range(3, n + 1):
        a, b, c = b, c, a + b + c
    return c

for n in [0, 1, 2, 3, 4, 5, 10, 20, 75]:
    want = f"tribonacci: T({n}) = {trib(n)}"
    if want not in s:
        print(f"TRIBONACCI TEST FAILED (missing '{want}')"); sys.exit(1)

# T(75) fits u64, T(76) overflows.
assert trib(75) <= 2**64 - 1 and trib(76) > 2**64 - 1
if "tribonacci: T(76) overflows u64" not in s:
    print("TRIBONACCI TEST FAILED (missing T(76) overflow line)"); sys.exit(1)
if "usage: tribonacci <n>" not in s:
    print("TRIBONACCI TEST FAILED (missing usage line)"); sys.exit(1)

print("TRIBONACCI TEST PASSED  (T0..T20 & T(75) exact vs python, T(76) overflow-guarded, bad->usage)")
PY
