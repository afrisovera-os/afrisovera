#!/usr/bin/env bash
# Verify the `kaprekar <n>` command classifies whether n (n>=1) is a KAPREKAR
# number: split the decimal digits of n^2 so the RIGHT part has as many digits
# as n, and the two parts sum to n (45^2=2025 -> 20+25=45; 9^2=81 -> 8+1=9;
# 99^2=9801 -> 98+01=99; 297,703,999,2223 yes; 1 yes). Low part must be > 0.
# n<1 -> usage. Self-checking: python re-derives the split and sum.
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
OUT="${TMPDIR:-/tmp}/kaprekar.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/kaprekar.png" \
    "echo warmup" "kaprekar 45" "kaprekar 99" "kaprekar 703" \
    "kaprekar 10" "kaprekar 46" "kaprekar 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "kaprekar:|usage: kaprekar" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def split(n):
    d = len(str(n))
    sq = n * n
    pw = 10 ** d
    return sq, sq // pw, sq % pw  # (n^2, high, low)

def is_kaprekar(n):
    sq, high, low = split(n)
    return low > 0 and high + low == n

def line(n):
    if is_kaprekar(n):
        sq, high, low = split(n)
        return f"kaprekar: {n} IS a Kaprekar number ({n}^2 = {sq} -> {high} + {low})"
    return f"kaprekar: {n} is NOT a Kaprekar number"

for n in [45, 99, 703, 10, 46]:
    want = line(n)
    if want not in s:
        print(f"KAPREKAR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly (including the split arithmetic shown)
assert line(45) == "kaprekar: 45 IS a Kaprekar number (45^2 = 2025 -> 20 + 25)"
assert line(99) == "kaprekar: 99 IS a Kaprekar number (99^2 = 9801 -> 98 + 1)"
assert line(703) == "kaprekar: 703 IS a Kaprekar number (703^2 = 494209 -> 494 + 209)"
assert line(10) == "kaprekar: 10 is NOT a Kaprekar number"

# n=0 must hit the usage line (n>=1 guard), NOT a verdict
if "usage: kaprekar <n>" not in s:
    print("KAPREKAR TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "kaprekar: 0 " in s:
    print("KAPREKAR TEST FAILED (n=0 should NOT produce a verdict)"); sys.exit(1)

print("KAPREKAR TEST PASSED  (45/99/703 yes with split shown, 10/46 no vs python; "
      "n=0 -> usage)")
PY
