#!/usr/bin/env bash
# Verify `prevfib <n>` gives the largest Fibonacci strictly less than n: 10 -> 8, 13 -> 8,
# 100 -> 89, 1 -> 0, 2 -> 1. Mirror of nextfib. Self-checking: python advances a,b=b,a+b,
# tracking the last a < n.
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
OUT="${TMPDIR:-/tmp}/prevfib.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevfib.png" \
    "echo warmup" "prevfib 10" "prevfib 13" "prevfib 100" \
    "prevfib 1" "prevfib 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevfib:|usage: prevfib" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def prevfib(n):
    a, b, last = 0, 1, 0
    while a < n:                    # track the largest Fibonacci strictly below n
        last = a; a, b = b, a + b
    return last

for n in [10, 13, 100, 1, 2]:
    want = f"prevfib: prev Fibonacci before {n} is {prevfib(n)}"    # DERIVED
    if want not in s:
        print(f"PREVFIB TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "prevfib: prev Fibonacci before 10 is 8" in s
assert "prevfib: prev Fibonacci before 13 is 8" in s
assert "prevfib: prev Fibonacci before 100 is 89" in s
assert "prevfib: prev Fibonacci before 1 is 0" in s
assert "prevfib: prev Fibonacci before 2 is 1" in s

print("PREVFIB TEST PASSED  (10->8, 13->8, 100->89, 1->0, 2->1 vs python Fibonacci scan)")
PY
