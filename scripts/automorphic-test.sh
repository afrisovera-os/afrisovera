#!/usr/bin/env bash
# Verify the `automorphic <n>` command classifies whether n is an AUTOMORPHIC
# number: n^2 ENDS IN n (n^2 mod 10^k == n, k = digit count). e.g. 5^2=25,
# 25^2=625, 76^2=5776, 376^2=141376. DISTINCT from armstrong (digit-power-sum),
# isqrt/icbrt (roots), sumsq (sum of squares), pow (exponent). Self-checking:
# python recomputes the verdict and the exact square.
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
OUT="${TMPDIR:-/tmp}/automorphic.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/automorphic.png" \
    "automorphic 5" "automorphic 6" "automorphic 25" "automorphic 76" \
    "automorphic 376" "automorphic 625" "automorphic 7" "automorphic 10" \
    "automorphic 0" "automorphic 1" "automorphic zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "automorphic:|usage: automorphic" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    k = len(str(n))
    sq = n * n
    if sq % (10 ** k) == n:
        return f"automorphic: {n} IS automorphic ({n}^2 = {sq} ends in {n})"
    return f"automorphic: {n} is NOT automorphic ({n}^2 = {sq})"

# Automorphic: 5,6,25,76,376,625,0,1 ; not: 7,10
for n in [5, 6, 25, 76, 376, 625, 7, 10, 0, 1]:
    want = line(n)
    if want not in s:
        print(f"AUTOMORPHIC TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin a couple of exact squares + verdicts
assert line(76) == "automorphic: 76 IS automorphic (76^2 = 5776 ends in 76)"
assert line(625).endswith("(625^2 = 390625 ends in 625)")
assert " is NOT " in line(7) and line(7).endswith("(7^2 = 49)")

if "usage: automorphic <n>" not in s:
    print("AUTOMORPHIC TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("AUTOMORPHIC TEST PASSED  (5/6/25/76/376/625/0/1 IS, 7/10 NOT, exact squares "
      "vs python, bad->usage)")
PY
