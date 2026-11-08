#!/usr/bin/env bash
# Verify `pisano <m>` computes the Pisano period pi(m) -- the period of Fibonacci numbers mod m:
# pi(1)=1, pi(2)=3, pi(3)=8, pi(7)=16, pi(10)=60. A number-theory function (period of Fib mod m),
# distinct from fib/order/collatz. Self-checking: python iterates (a,b) mod m until (0,1) recurs.
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
OUT="${TMPDIR:-/tmp}/pisano.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pisano.png" \
    "echo warmup" "pisano 1" "pisano 2" "pisano 3" "pisano 7" "pisano 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pisano:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def pisano(m):                       # DERIVED reference (independent iteration)
    if m == 1:
        return 1
    a, b = 0, 1
    for i in range(1, 6 * m + 1):
        a, b = b, (a + b) % m
        if a == 0 and b == 1:
            return i
    return 0

for m in (1, 2, 3, 7, 10):
    w = f"pisano: pi({m}) = {pisano(m)}"
    if w not in s:
        print(f"PISANO TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the canonical values so a wrong recurrence can't slip through
assert "pisano: pi(1) = 1" in s
assert "pisano: pi(2) = 3" in s
assert "pisano: pi(3) = 8" in s
assert "pisano: pi(7) = 16" in s
assert "pisano: pi(10) = 60" in s

print("PISANO TEST PASSED  (pi(1)=1, pi(2)=3, pi(3)=8, pi(7)=16, pi(10)=60 vs python Fib-mod-m period)")
PY
