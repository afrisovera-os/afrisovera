#!/usr/bin/env bash
# Verify the `smith <n>` command classifies whether n is a SMITH number: a
# COMPOSITE whose decimal digit sum equals the combined digit sum of its prime
# factors with multiplicity (4=2*2, 22=2*11, 27=3^3, 58, 85, 94, 121, 166, 378
# yes; primes excluded by definition; 10=2*5 composite but digit sums differ,
# NOT). n<2 -> usage. Self-checking: python re-derives both digit sums + the
# composite gate.
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
OUT="${TMPDIR:-/tmp}/smith.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/smith.png" \
    "echo warmup" "smith 4" "smith 22" "smith 27" "smith 378" \
    "smith 13" "smith 10" "smith 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "smith:|usage: smith" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dsum(n):
    return sum(int(c) for c in str(n))

def factors(n):
    f = []
    x = n
    d = 2
    while d * d <= x:
        while x % d == 0:
            f.append(d)
            x //= d
        d += 1
    if x > 1:
        f.append(x)
    return f

def is_smith(n):
    if n < 2:
        return False
    f = factors(n)
    if len(f) < 2:            # prime (single prime factor) -> not composite
        return False
    return dsum(n) == sum(dsum(p) for p in f)

def line(n):
    return f"smith: {n} IS a Smith number" if is_smith(n) else f"smith: {n} is NOT a Smith number"

for n in [4, 22, 27, 378, 13, 10]:
    want = line(n)
    if want not in s:
        print(f"SMITH TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(4) == "smith: 4 IS a Smith number"       # 2*2: 4 == 2+2
assert line(378) == "smith: 378 IS a Smith number"    # 2*3^3*7: 18 == 2+3+3+3+7
assert line(13) == "smith: 13 is NOT a Smith number"  # prime -> excluded
assert line(10) == "smith: 10 is NOT a Smith number"  # 2*5: ds 1 != 2+5=7

# n=1 must hit the usage line (n>=2 guard), NOT a verdict
if "usage: smith <n>" not in s:
    print("SMITH TEST FAILED (n=1 should report usage)"); sys.exit(1)
if "smith: 1 " in s:
    print("SMITH TEST FAILED (n=1 should NOT produce a verdict)"); sys.exit(1)

print("SMITH TEST PASSED  (4/22/27/378 yes, 13(prime)/10 no vs python; composite gate; "
      "n=1 -> usage)")
PY
