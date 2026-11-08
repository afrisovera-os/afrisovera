#!/usr/bin/env bash
# Verify `lcm3 <a> <b> <c>` gives lcm of three integers: 4 6 8->24, 2 3 5->30, 6 10 15->30,
# 4 5 6->60, 7 11 13->1001. Three-arg extension of lcm. Self-checking: python math.lcm(a,b,c).
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
OUT="${TMPDIR:-/tmp}/lcm3.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lcm3.png" \
    "echo warmup" "lcm3 4 6 8" "lcm3 2 3 5" "lcm3 6 10 15" \
    "lcm3 4 5 6" "lcm3 7 11 13" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lcm3" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def want(a, b, c):                    # DERIVED reference
    return f"lcm3({a}, {b}, {c}) = {math.lcm(a, b, c)}"

for a, b, c in [(4, 6, 8), (2, 3, 5), (6, 10, 15), (4, 5, 6), (7, 11, 13)]:
    w = want(a, b, c)
    if w not in s:
        print(f"LCM3 TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: powers of 2, distinct primes, overlapping, chained, three primes
assert "lcm3(4, 6, 8) = 24" in s
assert "lcm3(2, 3, 5) = 30" in s
assert "lcm3(6, 10, 15) = 30" in s     # pairwise overlap collapses
assert "lcm3(4, 5, 6) = 60" in s
assert "lcm3(7, 11, 13) = 1001" in s    # product of 3 distinct primes

print("LCM3 TEST PASSED  (4 6 8->24, 2 3 5->30, 6 10 15->30, 4 5 6->60, 7 11 13->1001 "
      "vs python math.lcm(a,b,c))")
PY
