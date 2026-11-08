#!/usr/bin/env bash
# Verify the `nextpow2 <n>` command reports the SMALLEST power of two that is >= n
# (round n up to a power of two), with the exponent. e.g. 5 -> 2^3 = 8, 100 ->
# 2^7 = 128, 8 -> 2^3 = 8 (exact powers map to themselves), 0/1 -> 2^0 = 1.
# DISTINCT from ilog2 (FLOOR log2), popcount (bit COUNT), pow (a^b), gray (Gray
# code). Self-checking: python computes the ceiling power of two + exponent.
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
OUT="${TMPDIR:-/tmp}/nextpow2.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextpow2.png" \
    "echo warmup" "nextpow2 0" "nextpow2 1" "nextpow2 2" "nextpow2 5" \
    "nextpow2 8" "nextpow2 17" "nextpow2 100" "nextpow2 1000" \
    "nextpow2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextpow2:|usage: nextpow2" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def np2(n):
    p, k = 1, 0
    while p < n:
        p *= 2
        k += 1
    return k, p

for n in [0, 1, 2, 5, 8, 17, 100, 1000]:
    k, p = np2(n)
    want = f"nextpow2: smallest power of 2 >= {n} is 2^{k} = {p}"
    if want not in s:
        print(f"NEXTPOW2 TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin ceil-vs-floor and exact-power mapping
assert np2(5) == (3, 8) and np2(100) == (7, 128)
assert np2(8) == (3, 8)        # exact power maps to itself (>= is inclusive)
assert np2(0) == (0, 1) and np2(1) == (0, 1)

if "usage: nextpow2 <n>" not in s:
    print("NEXTPOW2 TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("NEXTPOW2 TEST PASSED  (0/1/2/5/8/17/100/1000 ceil-power-of-2 exact vs python; "
      "bad->usage)")
PY
