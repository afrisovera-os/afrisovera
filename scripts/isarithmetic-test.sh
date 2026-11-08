#!/usr/bin/env bash
# Verify the `isarithmetic <n>` command classifies arithmetic numbers: the average of
# n's positive divisors is an integer, i.e. sigma(n) is divisible by tau(n). 1,3,6,7
# IS (means 1,2,3,4); 2 is NOT (divisors 1,2 sum 3 / 2 = 1.5). Distinct from perfect.
# Self-checking: python replicates the exact sigma/tau branch message.
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
OUT="${TMPDIR:-/tmp}/isarithmetic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 6 (arithmetic, mean 3), 2 (NOT), 7 (arithmetic, mean 4), 1 (arithmetic, mean 1).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isarithmetic.png" \
    "echo warmup" "isarithmetic 6" "isarithmetic 2" "isarithmetic 7" "isarithmetic 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isarithmetic:|usage: isarithmetic" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    if n < 1:
        return f"isarithmetic: {n} is NOT arithmetic (needs n >= 1)"
    tau = sigma = 0
    d = 1
    while d * d <= n:
        if n % d == 0:
            tau += 1; sigma += d
            e = n // d
            if e != d:
                tau += 1; sigma += e
        d += 1
    if sigma % tau == 0:
        return f"isarithmetic: {n} IS arithmetic (divisor mean {sigma // tau})"
    return f"isarithmetic: {n} is NOT arithmetic (divisor sum {sigma} not divisible by count {tau})"

for n in [6, 2, 7, 1]:
    want = message(n)   # sigma/tau/mean DERIVED by python, never hand-typed
    if want not in s:
        print(f"ISARITHMETIC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "isarithmetic: 6 IS arithmetic (divisor mean 3)" in s
assert "isarithmetic: 2 is NOT arithmetic (divisor sum 3 not divisible by count 2)" in s
assert "isarithmetic: 7 IS arithmetic (divisor mean 4)" in s
assert "isarithmetic: 1 IS arithmetic (divisor mean 1)" in s

print("ISARITHMETIC TEST PASSED  (6->mean 3, 2->NOT (3/2), 7->mean 4, 1->mean 1 vs python; "
      "average of divisors is an integer)")
PY
