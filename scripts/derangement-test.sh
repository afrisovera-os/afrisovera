#!/usr/bin/env bash
# Verify the `derangement` command computes the nth derangement number
# (subfactorial !n: permutations of n items with NO fixed point) in u64 with an
# overflow guard. Recurrence D(0)=1, D(n)=n*D(n-1)+(-1)^n -> 1,0,1,2,9,44,265,...
# Distinct from factorial (n!) and the linear recurrences (the alternating +/-1
# term). Self-checking: python recomputes D(n) by the same recurrence; the u64
# boundary is D(20) fits / D(21) overflows (!n = round(n!/e), near factorial's).
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
OUT="${TMPDIR:-/tmp}/derangement.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/derangement.png" \
    "derangement 0" "derangement 1" "derangement 2" "derangement 3" "derangement 4" \
    "derangement 5" "derangement 10" "derangement 20" "derangement 21" \
    "derangement xyz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "derangement:|usage: derangement" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def D(n):
    d = 1
    for k in range(1, n + 1):
        d = k * d + (1 if k % 2 == 0 else -1)
    return d

for n in [0, 1, 2, 3, 4, 5, 10, 20]:
    want = f"derangement: D({n}) = {D(n)}"
    if want not in s:
        print(f"DERANGEMENT TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: D(20) fits, D(21) overflows.
M = 2**64 - 1
assert D(20) <= M and D(21) > M, "boundary assumption wrong"
if "derangement: D(21) overflows u64" not in s:
    print("DERANGEMENT TEST FAILED (missing D(21) overflow line)"); sys.exit(1)
if "usage: derangement <n>" not in s:
    print("DERANGEMENT TEST FAILED (missing usage line)"); sys.exit(1)

print("DERANGEMENT TEST PASSED  (D0..D20 exact vs python, D(21) overflow-guarded, bad->usage)")
PY
