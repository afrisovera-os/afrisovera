#!/usr/bin/env bash
# Verify the `omega <n>` command counts the DISTINCT prime factors of n (little-
# omega): 12=2^2*3 -> 2, 30=2*3*5 -> 3, 8=2^3 -> 1, 64=2^6 -> 1, primes -> 1,
# 1 -> 0. The complement of bigomega (which counts WITH multiplicity, so 8 -> 3
# there but 1 here). n=0 -> usage. Self-checking: python len(set(primes)).
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
OUT="${TMPDIR:-/tmp}/omega.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/omega.png" \
    "echo warmup" "omega 8" "omega 12" "omega 30" "omega 64" "omega 1" "omega 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "omega:|usage: omega" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def omega(n):
    ps = set()
    x = n
    d = 2
    while d * d <= x:
        while x % d == 0:
            ps.add(d)
            x //= d
        d += 1
    if x > 1:
        ps.add(x)
    return len(ps)

def line(n):
    return f"omega: {n} has {omega(n)} distinct prime factors"

for n in [8, 12, 30, 64, 1]:
    want = line(n)
    if want not in s:
        print(f"OMEGA TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable values -- distinct count, NOT with multiplicity (contrast bigomega)
assert line(8) == "omega: 8 has 1 distinct prime factors"    # 2^3 -> 1 (bigomega: 3)
assert line(64) == "omega: 64 has 1 distinct prime factors"   # 2^6 -> 1 (bigomega: 6)
assert line(12) == "omega: 12 has 2 distinct prime factors"   # 2,3
assert line(30) == "omega: 30 has 3 distinct prime factors"   # 2,3,5
assert line(1) == "omega: 1 has 0 distinct prime factors"     # empty

# n=0 must hit the usage line (n>=1 guard), NOT a count
if "usage: omega <n>" not in s:
    print("OMEGA TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "omega: 0 " in s:
    print("OMEGA TEST FAILED (n=0 should NOT produce a count)"); sys.exit(1)

print("OMEGA TEST PASSED  (8/64->1, 12->2, 30->3, 1->0 distinct vs python; complement of "
      "bigomega which counts multiplicity; n=0 -> usage)")
PY
