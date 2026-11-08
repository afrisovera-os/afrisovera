#!/usr/bin/env bash
# Verify the `doublefact` command computes the double factorial n!! (= n*(n-2)*
# (n-4)*... down to 2 for even n or 1 for odd n; 0!!=1!!=1) in u64 with an
# overflow guard. DISTINCT from factorial: n! multiplies EVERY integer down to 1,
# whereas n!! multiplies EVERY OTHER integer (same parity as n) -> 1,1,2,3,8,15,
# 48,105,... Self-checking: python recomputes n!!; the u64 boundary is 33!! fits /
# 34!! overflows.
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
OUT="${TMPDIR:-/tmp}/doublefact.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/doublefact.png" \
    "doublefact 0" "doublefact 1" "doublefact 2" "doublefact 3" "doublefact 4" \
    "doublefact 5" "doublefact 6" "doublefact 7" "doublefact 10" "doublefact 20" \
    "doublefact 33" "doublefact 34" "doublefact zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "doublefact:|usage: doublefact" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def df(n):
    r, k = 1, n
    while k >= 2:
        r *= k
        k -= 2
    return r

for n in [0, 1, 2, 3, 4, 5, 6, 7, 10, 20, 33]:
    want = f"doublefact: {n}!! = {df(n)}"
    if want not in s:
        print(f"DOUBLEFACT TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: 33!! fits, 34!! overflows.
M = 2**64 - 1
assert df(33) <= M and df(34) > M, "boundary assumption wrong"
if "doublefact: 34!! overflows u64" not in s:
    print("DOUBLEFACT TEST FAILED (missing 34!! overflow line)"); sys.exit(1)
if "usage: doublefact <n>" not in s:
    print("DOUBLEFACT TEST FAILED (missing usage line)"); sys.exit(1)

print("DOUBLEFACT TEST PASSED  (0!!..20!! & 33!! exact vs python, 34!! overflow-guarded, bad->usage)")
PY
