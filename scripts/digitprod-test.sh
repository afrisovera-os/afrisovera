#!/usr/bin/env bash
# Verify the `digitprod` command computes the decimal digit PRODUCT, multiplicative
# persistence (steps to reach a single digit), and multiplicative digital root --
# the multiplicative parallel of `digitsum`. The harness recomputes each value the
# same way in python, so it is self-checking: 39->27/pers3/root4, 999->729/pers4/
# root2, 7->7/pers0/root7, 10->0/pers1/root0, 25->10/pers2/root0; bad arg -> usage.
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
OUT="${TMPDIR:-/tmp}/digitprod.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/digitprod.png" \
    "digitprod 39" "digitprod 999" "digitprod 7" "digitprod 10" "digitprod 25" \
    "digitprod abc" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "digitprod:|usage: digitprod" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dprod(x):
    if x == 0:
        return 0
    p = 1
    while x > 0:
        p *= x % 10
        x //= 10
    return p

def pers(n):
    steps, cur = 0, n
    while cur >= 10:
        cur = dprod(cur)
        steps += 1
    return steps, cur

for n in [39, 999, 7, 10, 25]:
    steps, root = pers(n)
    line1 = f"digitprod: {n} digit product = {dprod(n)}"
    line2 = f"digitprod: multiplicative persistence = {steps}, root = {root}"
    for want in (line1, line2):
        if want not in s:
            print(f"DIGITPROD TEST FAILED (missing '{want}')"); sys.exit(1)

if "usage: digitprod <n>" not in s:
    print("DIGITPROD TEST FAILED (missing usage line)"); sys.exit(1)

print("DIGITPROD TEST PASSED  (39->27/3/4, 999->729/4/2, 7->7/0/7, 10->0/1/0, 25->10/2/0; bad->usage)")
PY
