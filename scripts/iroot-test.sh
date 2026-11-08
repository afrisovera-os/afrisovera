#!/usr/bin/env bash
# Verify `iroot <k> <n>` computes the integer k-th root floor(n^(1/k)): iroot 4 16 = 2
# (perfect power), iroot 5 32 = 2 (perfect), iroot 2 10 = 3 (floor), iroot 10 1024 = 2
# (perfect), iroot 10 1023 = 1 (floor, just below 2^10). Generalizes isqrt/icbrt. Self-
# checking: python binary-searches the same floor with big ints (no overflow limit).
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
OUT="${TMPDIR:-/tmp}/iroot.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iroot.png" \
    "echo warmup" "iroot 4 16" "iroot 5 32" "iroot 2 10" \
    "iroot 10 1024" "iroot 10 1023" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iroot:|usage: iroot" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def iroot(k, n):
    lo, hi = 0, n
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if mid ** k <= n:            # python big int, INDEPENDENT of the kernel's checked u64
            lo = mid
        else:
            hi = mid - 1
    return lo

def line(k, n):
    r = iroot(k, n)
    tag = "perfect power" if r ** k == n else "floor"
    return f"iroot: floor({n}^(1/{k})) = {r} ({tag})"

for k, n in [(4, 16), (5, 32), (2, 10), (10, 1024), (10, 1023)]:
    w = line(k, n)
    if w not in s:
        print(f"IROOT TEST FAILED (k={k} n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: 4th root exact, 5th root exact, sqrt floor, 10th root exact, floor below
assert "iroot: floor(16^(1/4)) = 2 (perfect power)" in s
assert "iroot: floor(32^(1/5)) = 2 (perfect power)" in s
assert "iroot: floor(10^(1/2)) = 3 (floor)" in s
assert "iroot: floor(1024^(1/10)) = 2 (perfect power)" in s
assert "iroot: floor(1023^(1/10)) = 1 (floor)" in s

print("IROOT TEST PASSED  (4/16->2, 5/32->2, 2/10->3, 10/1024->2, 10/1023->1 "
      "vs python integer k-th root)")
PY
