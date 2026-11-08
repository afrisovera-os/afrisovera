#!/usr/bin/env bash
# Verify the `isfib <n>` command classifies whether n is a FIBONACCI number,
# reporting its index k. Fibonacci: F(0)=0,F(1)=1,F(2)=1,F(3)=2,F(4)=3,F(5)=5,
# F(6)=8,F(7)=13,F(8)=21,F(9)=34,F(10)=55. DISTINCT from fib (which COMPUTES the
# nth Fibonacci) -- isfib tests membership. Self-checking: python builds the
# Fibonacci index map and matches the verdict + index.
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
OUT="${TMPDIR:-/tmp}/isfib.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isfib.png" \
    "echo warmup" "isfib 0" "isfib 1" "isfib 2" "isfib 3" "isfib 5" \
    "isfib 8" "isfib 55" "isfib 4" "isfib 9" "isfib" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isfib:|usage: isfib" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

# First index k at which each Fibonacci value appears (matches the kernel's loop:
# it stops at the FIRST F(k) >= target, so F(1)=1 wins over F(2)=1).
first_index = {}
a, b, k = 0, 1, 0
while a <= 100:
    if a not in first_index:
        first_index[a] = k
    a, b, k = b, a + b, k + 1

def line(n):
    if n in first_index:
        return f"isfib: {n} IS a Fibonacci number (F({first_index[n]}) = {n})"
    return f"isfib: {n} is NOT a Fibonacci number"

for n in [0, 1, 2, 3, 5, 8, 55, 4, 9]:
    want = line(n)
    if want not in s:
        print(f"ISFIB TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the fib(6)=8 relationship
assert line(8) == "isfib: 8 IS a Fibonacci number (F(6) = 8)"
assert line(55) == "isfib: 55 IS a Fibonacci number (F(10) = 55)"
assert line(1) == "isfib: 1 IS a Fibonacci number (F(1) = 1)"   # F(1) wins over F(2)
assert line(9) == "isfib: 9 is NOT a Fibonacci number"

if "usage: isfib <n>" not in s:
    print("ISFIB TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("ISFIB TEST PASSED  (0/1/2/3/5/8/55 IS with exact index, 4/9 NOT vs python; "
      "F(1) wins the 1-duplicate; bad->usage)")
PY
