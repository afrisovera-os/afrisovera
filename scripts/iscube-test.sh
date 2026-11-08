#!/usr/bin/env bash
# Verify the `iscube <n>` command classifies perfect cubes (n = k^3), reporting the
# root k (27 -> 3, 1000000 -> 100, 26 NOT, 2 NOT). Distinct from icbrt (floor cube
# root of any n), issquare, and isperfectpower. Self-checking: python replicates the
# exact-integer cube-root membership test.
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
OUT="${TMPDIR:-/tmp}/iscube.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 27 (IS 3), 1000000 (IS 100), 26 (NOT), 2 (NOT).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iscube.png" \
    "echo warmup" "iscube 27" "iscube 1000000" "iscube 26" "iscube 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iscube:|usage: iscube" "$OUT" | sed 's/^/  /'

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
    return lo

def message(n):
    c = icbrt(n)
    if c ** 3 == n:
        return f"iscube: {n} IS a perfect cube ({c}^3 = {n})"
    return f"iscube: {n} is NOT a perfect cube"

for n in [27, 1000000, 26, 2]:
    want = message(n)   # root DERIVED by python, never hand-typed
    if want not in s:
        print(f"ISCUBE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "iscube: 27 IS a perfect cube (3^3 = 27)" in s
assert "iscube: 1000000 IS a perfect cube (100^3 = 1000000)" in s
assert "iscube: 26 is NOT a perfect cube" in s
assert "iscube: 2 is NOT a perfect cube" in s

print("ISCUBE TEST PASSED  (27->3, 1000000->100, 26 NOT, 2 NOT vs python; perfect-cube membership)")
PY
