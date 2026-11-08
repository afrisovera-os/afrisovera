#!/usr/bin/env bash
# Verify the `issquare <n>` command classifies whether n is a PERFECT SQUARE
# (n = k^2), reporting the root k. Perfect squares: 0,1,4,9,16,25,...,100,144.
# DISTINCT from isqrt (which COMPUTES the floor square root of any n) and the
# figurate classifiers triangular/pronic -- issquare tests membership.
# Self-checking: python's math.isqrt recomputes the verdict + root.
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
OUT="${TMPDIR:-/tmp}/issquare.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/issquare.png" \
    "echo warmup" "issquare 0" "issquare 1" "issquare 4" "issquare 9" \
    "issquare 16" "issquare 100" "issquare 144" "issquare 10000" \
    "issquare 2" "issquare 15" "issquare" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "issquare:|usage: issquare" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(n):
    r = math.isqrt(n)
    if r * r == n:
        return f"issquare: {n} IS a perfect square ({r}^2 = {n})"
    return f"issquare: {n} is NOT a perfect square"

# squares: 0,1,4,9,16,100,144,10000 ; not: 2,15
for n in [0, 1, 4, 9, 16, 100, 144, 10000, 2, 15]:
    want = line(n)
    if want not in s:
        print(f"ISSQUARE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin exact roots + the membership property
assert line(144) == "issquare: 144 IS a perfect square (12^2 = 144)"
assert line(10000) == "issquare: 10000 IS a perfect square (100^2 = 10000)"
assert line(15) == "issquare: 15 is NOT a perfect square"

if "usage: issquare <n>" not in s:
    print("ISSQUARE TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("ISSQUARE TEST PASSED  (0/1/4/9/16/100/144/10000 IS with exact root, 2/15 NOT "
      "vs python; bad->usage)")
PY
