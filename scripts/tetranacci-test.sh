#!/usr/bin/env bash
# Verify `tetranacci <n>` computes the nth tetranacci (T0=T1=T2=0, T3=1; four-term recurrence
# T(n)=T(n-1)+T(n-2)+T(n-3)+T(n-4), OEIS A000078): 0,0,0,1,1,2,4,8,15,29,56,... in u128 with an
# honest overflow guard. T(10)=56, T(20)=39648; a large n overflows u128. Extends fib/tribonacci.
# Self-checking: python computes every reference with the same checked rolling window.
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
OUT="${TMPDIR:-/tmp}/tetranacci.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/tetranacci.png" \
    "echo warmup" "tetranacci 3" "tetranacci 10" "tetranacci 20" "tetranacci 200" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "tetranacci:|usage: tetranacci" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: T0..T2=0, T3=1, 4-term window, u128 overflow guard
    if n <= 2:
        return f"tetranacci: T({n}) = 0"
    if n == 3:
        return "tetranacci: T(3) = 1"
    a, b, c, d = 0, 0, 0, 1  # T(k-4..k-1) starting at k=3
    for _ in range(4, n + 1):
        ssum = a + b + c + d
        if ssum > LIM:
            return f"tetranacci: T({n}) overflows u128"
        a, b, c, d = b, c, d, ssum
    return f"tetranacci: T({n}) = {d}"

for n in [3, 10, 20, 200]:
    want = message(n)   # DERIVED by python, never hand-typed
    if want not in s:
        print(f"TETRANACCI TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: exact values + the overflow branch (values python-derived above)
assert "tetranacci: T(3) = 1" in s
assert "tetranacci: T(10) = 56" in s
assert "tetranacci: T(20) = 39648" in s
assert "tetranacci: T(200) overflows u128" in s

print("TETRANACCI TEST PASSED  (T(3)=1, T(10)=56, T(20)=39648, T(200) overflows u128 "
      "vs python 4-term checked window)")
PY
