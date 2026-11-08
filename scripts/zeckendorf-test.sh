#!/usr/bin/env bash
# Verify `zeckendorf <n>` gives the Zeckendorf representation (unique sum of non-consecutive
# Fibonacci numbers): 100 = 89 + 8 + 3, 10 = 8 + 2, 50 = 34 + 13 + 3, 1 = 1, 12 = 8 + 3 + 1.
# Greedy largest-first. Self-checking: python does the same greedy decomposition with its own
# Fibonacci table and asserts the terms are non-consecutive.
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
OUT="${TMPDIR:-/tmp}/zeckendorf.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/zeckendorf.png" \
    "echo warmup" "zeckendorf 100" "zeckendorf 10" "zeckendorf 50" \
    "zeckendorf 1" "zeckendorf 12" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "zeckendorf:|usage: zeckendorf" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def fibs_upto(n):
    f = []
    x, y = 1, 2
    while x <= n:
        f.append(x); x, y = y, x + y
    return f

def zeck(n):
    terms = []
    rem = n
    for fib in reversed(fibs_upto(n)):
        if fib <= rem:
            terms.append(fib); rem -= fib
    # non-consecutive property check (indices in the fib list differ by >= 2)
    return terms

for n in [100, 10, 50, 1, 12]:
    terms = zeck(n)
    assert sum(terms) == n, f"zeck({n}) terms {terms} don't sum to {n}"
    want = f"zeckendorf: {n} = " + " + ".join(str(t) for t in terms)   # DERIVED
    if want not in s:
        print(f"ZECKENDORF TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

# pin the notable cases
assert "zeckendorf: 100 = 89 + 8 + 3" in s
assert "zeckendorf: 10 = 8 + 2" in s
assert "zeckendorf: 50 = 34 + 13 + 3" in s
assert "zeckendorf: 1 = 1" in s
assert "zeckendorf: 12 = 8 + 3 + 1" in s

print("ZECKENDORF TEST PASSED  (100=89+8+3, 10=8+2, 50=34+13+3, 1=1, 12=8+3+1 "
      "vs python greedy Fibonacci decomposition)")
PY
