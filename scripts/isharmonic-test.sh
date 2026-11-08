#!/usr/bin/env bash
# Verify `isharmonic <n>` classifies n as a HARMONIC DIVISOR (Ore) number: the harmonic mean of
# its divisors H = n*tau(n)/sigma(n) is an integer (OEIS A001599). 1,6,28,140,270,496,672 IS;
# 2,10,100 NOT. Distinct from perfect (sigma==2n) and isarithmetic (arithmetic mean sigma/tau).
# Self-checking: python computes tau/sigma with the same loop and derives the harmonic mean.
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
OUT="${TMPDIR:-/tmp}/isharmonic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isharmonic.png" \
    "echo warmup" "isharmonic 6" "isharmonic 28" "isharmonic 140" "isharmonic 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isharmonic:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):  # mirror the kernel: tau/sigma via d <= n/d, then n*tau % sigma
    tau = sigma = 0; d = 1
    while d <= n // d:
        if n % d == 0:
            tau += 1; sigma += d
            e = n // d
            if e != d:
                tau += 1; sigma += e
        d += 1
    if (n * tau) % sigma == 0:
        return f"isharmonic: {n} IS harmonic (divisor harmonic mean {(n*tau)//sigma})"
    return f"isharmonic: {n} is NOT harmonic (n*tau {n*tau} not divisible by sigma {sigma})"

for n in [6, 28, 140, 100]:
    want = message(n)   # every value DERIVED by python, never hand-typed
    if want not in s:
        print(f"ISHARMONIC TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: harmonic means (python-derived) + a non-member
assert message(6) in s and "harmonic mean 2)" in message(6)
assert message(28) in s and "harmonic mean 3)" in message(28)
assert message(140) in s and "harmonic mean 5)" in message(140)
assert "isharmonic: 100 is NOT harmonic" in s

print("ISHARMONIC TEST PASSED  (6 mean 2, 28 mean 3, 140 mean 5, 100 NOT "
      "vs python n*tau/sigma Ore gate)")
PY
