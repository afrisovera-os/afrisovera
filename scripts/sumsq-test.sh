#!/usr/bin/env bash
# Verify the `sumsq` command computes the sum of the first n squares
# (1^2+2^2+...+n^2) via the closed form n(n+1)(2n+1)/6 in u128, overflow-guarded.
# Distinct from sumto (the linear triangular sum). The harness recomputes the
# closed form in python, so it is self-checking: sumsq 0=0, 1=1, 2=5, 3=14,
# 10=385, 100=338350; u64-max input overflows u128 -> overflow line.
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
OUT="${TMPDIR:-/tmp}/sumsq.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumsq.png" \
    "sumsq 0" "sumsq 1" "sumsq 2" "sumsq 3" "sumsq 10" "sumsq 100" \
    "sumsq 18446744073709551615" "sumsq abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumsq:|usage: sumsq" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sumsq(n):
    return n * (n + 1) * (2 * n + 1) // 6

for n in [0, 1, 2, 3, 10, 100]:
    # cross-check the closed form against a direct sum
    assert sumsq(n) == sum(k * k for k in range(1, n + 1))
    want = f"sumsq: sum of first {n} squares = {sumsq(n)}"
    if want not in s:
        print(f"SUMSQ TEST FAILED (missing '{want}')"); sys.exit(1)

# u64-max input: n(n+1)(2n+1) far exceeds u128 max.
big = 18446744073709551615
assert big * (big + 1) * (2 * big + 1) > 2**128 - 1
if "sumsq: overflows u128 (n too large)" not in s:
    print("SUMSQ TEST FAILED (missing overflow line)"); sys.exit(1)
if "usage: sumsq <n>" not in s:
    print("SUMSQ TEST FAILED (missing usage line)"); sys.exit(1)

print("SUMSQ TEST PASSED  (0=0,1=1,2=5,3=14,10=385,100=338350 vs closed form; u64max overflow-guarded; bad->usage)")
PY
