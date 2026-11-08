#!/usr/bin/env bash
# Verify the `pronic <n>` command classifies whether n is a PRONIC (oblong)
# number (n = k*(k+1)), reporting the index k. A number is pronic iff 4n+1 is a
# perfect square; then k = (sqrt(4n+1)-1)/2. Pronic: 0,2,6,12,20,30,42,56,72,90.
# DISTINCT from triangular (n=k(k+1)/2; pronic are exactly 2x triangular), isfib,
# sumto, and the digit classifiers. Self-checking: python recomputes via
# math.isqrt.
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
OUT="${TMPDIR:-/tmp}/pronic.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pronic.png" \
    "echo warmup" "pronic 0" "pronic 2" "pronic 6" "pronic 12" "pronic 20" \
    "pronic 90" "pronic 1" "pronic 5" "pronic" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pronic:|usage: pronic" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    r = math.isqrt(4 * n + 1)
    if r * r == 4 * n + 1:
        k = (r - 1) // 2
        return f"pronic: {n} IS pronic ({k}*({k}+1) = {n})"
    return f"pronic: {n} is NOT pronic"

# pronic: 0,2,6,12,20,90 (with indices) ; not: 1, 5
for n in [0, 2, 6, 12, 20, 90, 1, 5]:
    want = line(n)
    if want not in s:
        print(f"PRONIC TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact indices + the 4n+1 membership property
assert line(6) == "pronic: 6 IS pronic (2*(2+1) = 6)"
assert line(90) == "pronic: 90 IS pronic (9*(9+1) = 90)"
assert line(5) == "pronic: 5 is NOT pronic"
# 3 is triangular (T(2)) but NOT pronic -- different figurate families
assert line(3) == "pronic: 3 is NOT pronic"

if "usage: pronic <n>" not in s:
    print("PRONIC TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("PRONIC TEST PASSED  (0/2/6/12/20/90 IS with exact k, 1/5 NOT vs python; "
      "distinct from triangular; bad->usage)")
PY
