#!/usr/bin/env bash
# Verify `pentanacci <n>` computes the nth pentanacci (P0=P1=P2=P3=0, P4=1; five-term recurrence
# P(n)=P(n-1)+P(n-2)+P(n-3)+P(n-4)+P(n-5), OEIS A001591): 0,0,0,0,1,1,2,4,8,16,31,61,120,... in
# u128 with an honest overflow guard. Extends fib/tribonacci/tetranacci. Self-checking: python
# computes every reference (including the exact values) with the same checked rolling window.
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
OUT="${TMPDIR:-/tmp}/pentanacci.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pentanacci.png" \
    "echo warmup" "pentanacci 4" "pentanacci 12" "pentanacci 30" "pentanacci 250" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pentanacci:|usage: pentanacci" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: P0..P3=0, P4=1, 5-term window, u128 overflow guard
    if n <= 3:
        return f"pentanacci: P({n}) = 0"
    if n == 4:
        return "pentanacci: P(4) = 1"
    a, b, c, d, e = 0, 0, 0, 0, 1  # P(k-5..k-1) starting at k=4
    for _ in range(5, n + 1):
        ssum = a + b + c + d + e
        if ssum > LIM:
            return f"pentanacci: P({n}) overflows u128"
        a, b, c, d, e = b, c, d, e, ssum
    return f"pentanacci: P({n}) = {e}"

want_all = {n: message(n) for n in [4, 12, 30, 250]}   # EVERY value DERIVED by python
for n, want in want_all.items():
    if want not in s:
        print(f"PENTANACCI TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: exact values (python-derived above) + the overflow branch
assert want_all[4] in s and "P(4) = 1" in want_all[4]
assert want_all[12] in s          # a mid-sequence exact value
assert want_all[30] in s          # a larger exact value
assert "overflows u128" in want_all[250]

print(f"PENTANACCI TEST PASSED  ({want_all[12]}, {want_all[30]}, P(250) overflows u128 "
      "vs python 5-term checked window)")
PY
