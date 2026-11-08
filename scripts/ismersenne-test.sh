#!/usr/bin/env bash
# Verify the `ismersenne <n>` command classifies Mersenne numbers: n = 2^p - 1
# for some p >= 1, i.e. n's binary form is all ones (1, 3, 7, 15, 31, 63, 127,
# ...). The test is n >= 1 AND (n & (n+1)) == 0, reporting the exponent p (bit
# length). Distinct from nextpow2 (rounds UP), ilog2 (FLOOR log2), and popcount
# (just the bit count). u64::MAX (itself 2^64-1) must NOT panic (wrapping_add).
# Self-checking: python computes (n & (n+1)) == 0.
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
OUT="${TMPDIR:-/tmp}/ismersenne.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# The u64::MAX case exercises the wrapping_add overflow guard; "alive" proves the
# shell survived (no panic).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/ismersenne.png" \
    "echo warmup" "ismersenne 31" "ismersenne 100" "ismersenne 18446744073709551615" "echo alive" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ismersenne:|usage: ismersenne|alive" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ismer(n):
    return n >= 1 and (n & (n + 1)) == 0

# IS cases (with the exponent = bit length)
for n in [31, 18446744073709551615]:
    assert ismer(n), f"python: {n} should be Mersenne"
    p = n.bit_length()
    want = f"ismersenne: {n} IS a Mersenne number (2^{p} - 1)"
    if want not in s:
        print(f"ISMERSENNE TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT case
assert not ismer(100)
if "ismersenne: 100 is NOT a Mersenne number (not all-ones in binary)" not in s:
    print("ISMERSENNE TEST FAILED (100 should be NOT)"); sys.exit(1)

# the u64::MAX case must not have panicked -- shell kept running
if "alive" not in s:
    print("ISMERSENNE TEST FAILED (shell did not survive u64::MAX -- possible panic)"); sys.exit(1)

print("ISMERSENNE TEST PASSED  (31 & u64::MAX=2^64-1 IS with exponent, 100 NOT vs "
      "python; u64::MAX handled without panic; distinct from nextpow2/ilog2/popcount)")
PY
