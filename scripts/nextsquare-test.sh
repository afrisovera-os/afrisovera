#!/usr/bin/env bash
# Verify the `nextsquare <n>` command reports the smallest perfect square >= n
# (round n up to a perfect square), with its root. e.g. 10 -> 4^2 = 16, 16 ->
# 4^2 = 16 (exact squares map to themselves), 50 -> 8^2 = 64. DISTINCT from
# nextpow2 (power of TWO >= n), issquare (a MEMBERSHIP test), isqrt (FLOOR root),
# nextprime. Self-checking: python's math.isqrt computes the ceiling square.
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
OUT="${TMPDIR:-/tmp}/nextsquare.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextsquare.png" \
    "echo warmup" "nextsquare 0" "nextsquare 1" "nextsquare 2" "nextsquare 10" \
    "nextsquare 16" "nextsquare 17" "nextsquare 50" "nextsquare 100" \
    "nextsquare" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextsquare:|usage: nextsquare" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def ns(n):
    r = math.isqrt(n)
    k = r if r * r == n else r + 1
    return k, k * k

for n in [0, 1, 2, 10, 16, 17, 50, 100]:
    k, sq = ns(n)
    want = f"nextsquare: smallest perfect square >= {n} is {k}^2 = {sq}"
    if want not in s:
        print(f"NEXTSQUARE TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin ceil behaviour + the exact-square self-mapping
assert ns(10) == (4, 16) and ns(50) == (8, 64)
assert ns(16) == (4, 16)      # exact square maps to itself (>= is inclusive)
assert ns(0) == (0, 0) and ns(1) == (1, 1)

if "usage: nextsquare <n>" not in s:
    print("NEXTSQUARE TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("NEXTSQUARE TEST PASSED  (0/1/2/10/16/17/50/100 ceil-square exact vs python; "
      "bad->usage)")
PY
