#!/usr/bin/env bash
# Verify `amicable <n>` classifies whether n is part of an AMICABLE pair: two DISTINCT
# numbers where each is the sum of the OTHER's proper divisors, s(a)=b and s(b)=a, a != b.
# Classical pairs: 220/284 (s(220)=284, s(284)=220) and 1184/1210. PERFECT numbers (s(n)==n,
# e.g. 6, 28) are explicitly excluded -- they are a fixed point of s(), not a distinct partner.
# Self-checking: python re-derives s(n) via the same trial-division proper-divisor-sum.
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
OUT="${TMPDIR:-/tmp}/amicable.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/amicable.png" \
    "echo warmup" "amicable 220" "amicable 284" "amicable 1184" "amicable 6" \
    "amicable 13" "amicable 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "amicable:|usage: amicable" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def s_of(n):
    if n <= 1:
        return 0
    total = 1
    d = 2
    while d * d <= n:
        if n % d == 0:
            total += d
            e = n // d
            if e != d:
                total += e
        d += 1
    return total

def line(n):
    partner = s_of(n)
    back = s_of(partner)
    if partner != n and back == n:
        return f"amicable: {n} IS amicable -- paired with {partner}"
    return f"amicable: {n} is NOT amicable"

for n in [220, 284, 1184, 6, 13]:
    want = line(n)
    if want not in s:
        print(f"AMICABLE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert line(220) == "amicable: 220 IS amicable -- paired with 284"
assert line(284) == "amicable: 284 IS amicable -- paired with 220"
assert line(1184) == "amicable: 1184 IS amicable -- paired with 1210"
assert line(6) == "amicable: 6 is NOT amicable"    # perfect number excluded (s(6)==6)
assert line(13) == "amicable: 13 is NOT amicable"  # prime, s(13)=1

# n=0 must hit the usage line (n>=1 guard), NOT a verdict
if "usage: amicable <n>" not in s:
    print("AMICABLE TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "amicable: 0 " in s:
    print("AMICABLE TEST FAILED (n=0 should NOT produce a verdict)"); sys.exit(1)

print("AMICABLE TEST PASSED  (220/284 & 1184/1210 pairs, 6 perfect excluded, "
      "13 prime not amicable, n=0 -> usage) vs python")
PY
