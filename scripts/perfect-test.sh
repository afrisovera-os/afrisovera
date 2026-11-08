#!/usr/bin/env bash
# Verify the `perfect` command classifies a positive integer by the sum of its
# PROPER divisors (all divisors except n): perfect (sum==n), deficient (sum<n),
# abundant (sum>n). Distinct from `divisors` (tau/sigma). The harness recomputes
# the proper-divisor sum + classification in python, so it is self-checking:
# 6 & 28 & 496 perfect, 12 abundant, 8 & 1 deficient; a bad arg -> usage.
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
OUT="${TMPDIR:-/tmp}/perfect.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/perfect.png" \
    "perfect 6" "perfect 28" "perfect 12" "perfect 8" "perfect 1" "perfect 496" \
    "perfect xyz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "perfect:|usage: perfect" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def classify(n):
    tot = sum(d for d in range(1, n) if n % d == 0)
    kind = "perfect" if tot == n else ("deficient" if tot < n else "abundant")
    return tot, kind

for n in [6, 28, 12, 8, 1, 496]:
    tot, kind = classify(n)
    want = f"perfect: {n} -- proper divisor sum {tot} -- {kind}"
    if want not in s:
        print(f"PERFECT TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: perfect <n>" not in s:
    print("PERFECT TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("PERFECT TEST PASSED  (6/28/496 perfect, 12 abundant, 8/1 deficient; bad->usage)")
PY
