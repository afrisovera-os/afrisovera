#!/usr/bin/env bash
# Verify `prevsquare <n>` gives the largest perfect square <= n (rounds DOWN, mirror of
# nextsquare): 10 -> 9 (3^2), 16 -> 16 (4^2), 17 -> 16, 1 -> 1, 100 -> 100 (10^2). Self-
# checking: python isqrt(n)^2.
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
OUT="${TMPDIR:-/tmp}/prevsquare.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevsquare.png" \
    "echo warmup" "prevsquare 10" "prevsquare 16" "prevsquare 17" \
    "prevsquare 1" "prevsquare 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevsquare:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    r = math.isqrt(n)                    # DERIVED floor sqrt
    return f"prevsquare: largest square <= {n} is {r * r} ({r}^2)"

for n in [10, 16, 17, 1, 100]:
    w = line(n)
    if w not in s:
        print(f"PREVSQUARE TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

assert "prevsquare: largest square <= 10 is 9 (3^2)" in s
assert "prevsquare: largest square <= 16 is 16 (4^2)" in s
assert "prevsquare: largest square <= 17 is 16 (4^2)" in s
assert "prevsquare: largest square <= 1 is 1 (1^2)" in s
assert "prevsquare: largest square <= 100 is 100 (10^2)" in s

print("PREVSQUARE TEST PASSED  (10->9, 16->16, 17->16, 1->1, 100->100 vs python isqrt^2)")
PY
