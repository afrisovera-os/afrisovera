#!/usr/bin/env bash
# Verify the `isugly <n>` command classifies whether n (n>=1) is an UGLY number
# (5-smooth): its only prime factors are 2, 3, and/or 5 (1,2,3,4,5,6,8,9,10,12,
# 15,16,30,60 yes; 7,11,14,21,77 no; 1 yes -- the empty product). n<1 -> usage.
# Self-checking: python divides out 2/3/5 and checks the remainder is 1.
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
OUT="${TMPDIR:-/tmp}/isugly.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isugly.png" \
    "echo warmup" "isugly 60" "isugly 16" "isugly 1" \
    "isugly 7" "isugly 14" "isugly 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isugly:|usage: isugly" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ugly(n):
    for p in (2, 3, 5):
        while n % p == 0:
            n //= p
    return n == 1

def line(n):
    return f"isugly: {n} IS an ugly number" if ugly(n) else f"isugly: {n} is NOT an ugly number"

for n in [60, 16, 1, 7, 14]:
    want = line(n)
    if want not in s:
        print(f"ISUGLY TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(60) == "isugly: 60 IS an ugly number"      # 2^2*3*5
assert line(16) == "isugly: 16 IS an ugly number"       # 2^4
assert line(1) == "isugly: 1 IS an ugly number"         # empty product
assert line(7) == "isugly: 7 is NOT an ugly number"     # prime 7
assert line(14) == "isugly: 14 is NOT an ugly number"   # 2*7 (7 excluded)

# n=0 must hit the usage line (n>=1 guard), NOT a verdict
if "usage: isugly <n>" not in s:
    print("ISUGLY TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "isugly: 0 " in s:
    print("ISUGLY TEST FAILED (n=0 should NOT produce a verdict)"); sys.exit(1)

print("ISUGLY TEST PASSED  (60/16/1 ugly, 7/14 not vs python; only primes 2,3,5; n=0 -> usage)")
PY
