#!/usr/bin/env bash
# Verify the `lucas` command computes the nth Lucas number in u64 with an overflow
# guard. Same recurrence as fib but seeded L0=2, L1=1 (sequence 2,1,3,4,7,11,...).
# The harness recomputes each L(n) in python, so it is self-checking: L(0)=2,
# L(1)=1, L(2)=3, L(5)=11, L(10)=123, L(20)=15127, L(92) fits u64, L(93) overflows.
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
OUT="${TMPDIR:-/tmp}/lucas.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lucas.png" \
    "lucas 0" "lucas 1" "lucas 2" "lucas 5" "lucas 10" "lucas 20" \
    "lucas 92" "lucas 93" "lucas abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lucas:|usage: lucas" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lucas(n):
    if n == 0:
        return 2
    a, b = 2, 1
    for _ in range(1, n):
        a, b = b, a + b
    return b

for n in [0, 1, 2, 5, 10, 20, 92]:
    want = f"lucas: L({n}) = {lucas(n)}"
    if want not in s:
        print(f"LUCAS TEST FAILED (missing '{want}')"); sys.exit(1)

# L(92) fits u64, L(93) overflows.
assert lucas(92) <= 2**64 - 1 and lucas(93) > 2**64 - 1
if "lucas: L(93) overflows u64" not in s:
    print("LUCAS TEST FAILED (missing L(93) overflow line)"); sys.exit(1)
if "usage: lucas <n>" not in s:
    print("LUCAS TEST FAILED (missing usage line)"); sys.exit(1)

print("LUCAS TEST PASSED  (L0=2..L20=15127 exact vs python, L(92) fits, L(93) overflow-guarded, bad->usage)")
PY
