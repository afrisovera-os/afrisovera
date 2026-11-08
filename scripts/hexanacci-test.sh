#!/usr/bin/env bash
# Verify `hexanacci <n>` computes the nth hexanacci (H0..H4=0, H5=1; six-term recurrence
# H(n)=H(n-1)+...+H(n-6), OEIS A001592): 0,0,0,0,0,1,1,2,4,8,16,32,63,125,... in u128 with an
# honest overflow guard. Extends fib/tribonacci/tetranacci/pentanacci. Self-checking: python
# computes every reference with the same checked six-wide rolling window.
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
OUT="${TMPDIR:-/tmp}/hexanacci.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/hexanacci.png" \
    "echo warmup" "hexanacci 5" "hexanacci 13" "hexanacci 25" "hexanacci 300" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "hexanacci:|usage: hexanacci" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: H0..H4=0, H5=1, 6-term window, u128 overflow guard
    if n <= 4:
        return f"hexanacci: H({n}) = 0"
    if n == 5:
        return "hexanacci: H(5) = 1"
    w = [0, 0, 0, 0, 0, 1]  # H(k-6..k-1) starting at k=5
    for _ in range(6, n + 1):
        ssum = sum(w)
        if ssum > LIM:
            return f"hexanacci: H({n}) overflows u128"
        w = w[1:] + [ssum]
    return f"hexanacci: H({n}) = {w[-1]}"

want_all = {n: message(n) for n in [5, 13, 25, 300]}   # EVERY value DERIVED by python
for n, want in want_all.items():
    if want not in s:
        print(f"HEXANACCI TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: the H5=1 base, two mid-sequence exact values, the overflow branch
assert want_all[5] in s and "H(5) = 1" in want_all[5]
assert want_all[13] in s
assert want_all[25] in s
assert "overflows u128" in want_all[300]

print(f"HEXANACCI TEST PASSED  ({want_all[13]}, {want_all[25]}, H(300) overflows u128 "
      "vs python 6-term checked window)")
PY
