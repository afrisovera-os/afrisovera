#!/usr/bin/env bash
# Verify the `narayana <n>` command computes Narayana's cows number: N(0)=N(1)=N(2)=1,
# N(n)=N(n-1)+N(n-3) in u128 -> 1,1,1,2,3,4,6,9,13,19,28,41,... N(10)=28, N(20)=1278;
# N(234) overflows u128. Distinct from fib/padovan/perrin. Self-checking: python computes
# every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/narayana.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/narayana.png" \
    "echo warmup" "narayana 0" "narayana 10" "narayana 20" "narayana 234" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "narayana:|usage: narayana" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: N(n)=N(n-1)+N(n-3), u128 overflow guard
    if n <= 2:
        return f"narayana: N({n}) = 1"
    a = b = c = 1
    for _ in range(3, n + 1):
        cur = c + a
        if cur > LIM:
            return f"narayana: N({n}) overflows u128"
        a, b, c = b, c, cur
    return f"narayana: N({n}) = {c}"

for n in [0, 10, 20, 234]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"NARAYANA TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "narayana: N(0) = 1" in s
assert "narayana: N(10) = 28" in s
assert "narayana: N(20) = 1278" in s
assert "narayana: N(234) overflows u128" in s

print("NARAYANA TEST PASSED  (N(0)=1, N(10)=28, N(20)=1278, N(234) overflows u128 "
      "vs python N(n-1)+N(n-3))")
PY
