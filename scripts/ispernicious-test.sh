#!/usr/bin/env bash
# Verify the `ispernicious <n>` command classifies pernicious numbers: n's
# population count (number of 1-bits in its binary form) is a PRIME. e.g.
# 7 = 0b111 has 3 set bits (prime) IS; 4 = 0b100 has 1 (not prime) NOT; 9 = 0b1001
# has 2 (prime) IS. Distinct from popcount (only counts set bits) and isprime
# (tests n itself). Self-checking: python computes bin(n).count('1') primality.
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
OUT="${TMPDIR:-/tmp}/ispernicious.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ispernicious.png" \
    "echo warmup" "ispernicious 7" "ispernicious 4" "ispernicious 127" "ispernicious 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ispernicious:|usage: ispernicious" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isprime(k):
    if k < 2: return False
    i = 2
    while i * i <= k:
        if k % i == 0: return False
        i += 1
    return True

# (n) -> expected line built from its set-bit count and that count's primality
for n in [7, 4, 127, 1]:
    ones = bin(n).count("1")
    if isprime(ones):
        want = f"ispernicious: {n} IS pernicious ({ones} set bits, prime)"
    else:
        want = f"ispernicious: {n} is NOT pernicious ({ones} set bits, not prime)"
    if want not in s:
        print(f"ISPERNICIOUS TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable cases explicitly
assert "ispernicious: 7 IS pernicious (3 set bits, prime)" in s        # 0b111
assert "ispernicious: 4 is NOT pernicious (1 set bits, not prime)" in s # 0b100
assert "ispernicious: 127 IS pernicious (7 set bits, prime)" in s      # 0b1111111

print("ISPERNICIOUS TEST PASSED  (7/127 IS, 4/1 NOT via set-bit-count primality "
      "vs python; distinct from popcount and isprime)")
PY
