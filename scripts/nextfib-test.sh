#!/usr/bin/env bash
# Verify `nextfib <n>` gives the smallest Fibonacci strictly greater than n: 10 -> 13, 13 ->
# 21, 100 -> 144, 0 -> 1, 1 -> 2. Companion of nextprime/nextsquare. Self-checking: python
# advances a,b=b,a+b until a>n.
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
OUT="${TMPDIR:-/tmp}/nextfib.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextfib.png" \
    "echo warmup" "nextfib 10" "nextfib 13" "nextfib 100" \
    "nextfib 0" "nextfib 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextfib:|usage: nextfib" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def nextfib(n):
    a, b = 0, 1
    while a <= n:                       # advance until strictly greater than n
        a, b = b, a + b
    return a

for n in [10, 13, 100, 0, 1]:
    want = f"nextfib: next Fibonacci after {n} is {nextfib(n)}"    # DERIVED
    if want not in s:
        print(f"NEXTFIB TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

# pin the notable cases
assert "nextfib: next Fibonacci after 10 is 13" in s
assert "nextfib: next Fibonacci after 13 is 21" in s
assert "nextfib: next Fibonacci after 100 is 144" in s
assert "nextfib: next Fibonacci after 0 is 1" in s
assert "nextfib: next Fibonacci after 1 is 2" in s

print("NEXTFIB TEST PASSED  (10->13, 13->21, 100->144, 0->1, 1->2 vs python Fibonacci advance)")
PY
