#!/usr/bin/env bash
# Verify the `icbrt` command computes the integer floor cube root + perfect-cube flag
# (the cube-root sibling of isqrt), deterministic: 27 -> 3 (perfect), 26 -> 2 (floor),
# 1000000 -> 100 (perfect), and u64::MAX -> 2642245 (floor, exercises the u128 cube).
# The test recomputes the integer cube root in python for a self-checking assert.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4888,server,nowait \
    -qmp tcp:127.0.0.1:4889,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/icbrt.out"
python3 scripts/shell-demo.py 4888 4889 "${TMPDIR:-/tmp}/icbrt.png" \
    "icbrt 27" "icbrt 26" "icbrt 1000000" "icbrt 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "icbrt:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def icbrt(n):
    lo, hi = 0, 2642246
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if mid ** 3 <= n:
            lo = mid
        else:
            hi = mid - 1
    return lo, lo ** 3 == n

for n in [27, 26, 1000000, 2**64 - 1]:
    r, perfect = icbrt(n)
    tag = "perfect cube" if perfect else "floor"
    want = f"icbrt: cbrt({n}) = {r} ({tag})"
    if want not in s:
        print(f"ICBRT TEST FAILED (expected '{want}')"); sys.exit(1)

print("ICBRT TEST PASSED  (27->3 perfect, 26->2 floor, 1000000->100 perfect, u64max->2642245 floor)")
PY
