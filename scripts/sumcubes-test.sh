#!/usr/bin/env bash
# Verify the `sumcubes` command computes the sum of the first n cubes
# (1^3+2^3+...+n^3) via the closed form (n(n+1)/2)^2 = T(n)^2 in u128, overflow-
# guarded. Completes the power-sum trio with sumto (linear) and sumsq (squares).
# The harness recomputes (n(n+1)//2)^2 in python, so it is self-checking: 0=0,
# 1=1, 2=9, 3=36, 10=3025 (=55^2 = sumto(10)^2), 100=25502500; u64-max -> overflow.
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
OUT="${TMPDIR:-/tmp}/sumcubes.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumcubes.png" \
    "sumcubes 0" "sumcubes 1" "sumcubes 2" "sumcubes 3" "sumcubes 10" "sumcubes 100" \
    "sumcubes 18446744073709551615" "sumcubes abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumcubes:|usage: sumcubes" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sumcubes(n):
    return (n * (n + 1) // 2) ** 2

for n in [0, 1, 2, 3, 10, 100]:
    # cross-check the closed form against a direct sum of cubes
    assert sumcubes(n) == sum(k ** 3 for k in range(1, n + 1))
    want = f"sumcubes: sum of first {n} cubes = {sumcubes(n)}"
    if want not in s:
        print(f"SUMCUBES TEST FAILED (missing '{want}')"); sys.exit(1)

# u64-max input: T(n)^2 far exceeds u128 max.
big = 18446744073709551615
t = big * (big + 1) // 2
assert t * t > 2**128 - 1
if "sumcubes: overflows u128 (n too large)" not in s:
    print("SUMCUBES TEST FAILED (missing overflow line)"); sys.exit(1)
if "usage: sumcubes <n>" not in s:
    print("SUMCUBES TEST FAILED (missing usage line)"); sys.exit(1)

print("SUMCUBES TEST PASSED  (0,1,9,36,3025,25502500 vs closed form; u64max overflow-guarded; bad->usage)")
PY
