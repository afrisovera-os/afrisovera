#!/usr/bin/env bash
# Verify `sumfibs <n>` = sum of the first n Fibonacci numbers (F1=F2=1): 1->1, 5->12, 10->143,
# 20->17710, 0->0. Equals F(n+2)-1 by identity. Self-checking: python accumulates a,b=b,a+b
# (and independently checks the F(n+2)-1 identity).
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
OUT="${TMPDIR:-/tmp}/sumfibs.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumfibs.png" \
    "echo warmup" "sumfibs 1" "sumfibs 5" "sumfibs 10" \
    "sumfibs 20" "sumfibs 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumfibs:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sumfibs(n):
    a, b, tot = 1, 1, 0
    for _ in range(n):
        tot += a; a, b = b, a + b
    return tot

def fib(k):                     # F(0)=0,F(1)=1
    a, b = 0, 1
    for _ in range(k): a, b = b, a + b
    return a

for n in [1, 5, 10, 20, 0]:
    t = sumfibs(n)
    assert t == fib(n + 2) - 1, f"identity fail at n={n}"     # cross-check F(n+2)-1
    want = f"sumfibs: sum of first {n} Fibonacci numbers is {t}"   # DERIVED
    if want not in s:
        print(f"SUMFIBS TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "sumfibs: sum of first 1 Fibonacci numbers is 1" in s
assert "sumfibs: sum of first 5 Fibonacci numbers is 12" in s
assert "sumfibs: sum of first 10 Fibonacci numbers is 143" in s
assert "sumfibs: sum of first 20 Fibonacci numbers is 17710" in s
assert "sumfibs: sum of first 0 Fibonacci numbers is 0" in s

print("SUMFIBS TEST PASSED  (1->1, 5->12, 10->143, 20->17710, 0->0; each == F(n+2)-1 "
      "vs python)")
PY
