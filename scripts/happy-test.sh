#!/usr/bin/env bash
# Verify the `happy <n>` command classifies whether n is a HAPPY number:
# repeatedly replace n with the sum of the squares of its decimal digits; if this
# reaches 1 it is happy, otherwise it falls into the 4->16->37->...->4 cycle and
# is unhappy (0 is treated as unhappy: it maps to 0 forever). DISTINCT from
# digitsum (plain sum), digitprod (product), armstrong (digit^k sum), automorphic
# (n^2 ends in n), collatz (3n+1). Self-checking: python replicates the iteration.
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
OUT="${TMPDIR:-/tmp}/happy.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/happy.png" \
    "echo warmup" "happy 7" "happy 19" "happy 23" "happy 100" \
    "happy 4" "happy 2" "happy 0" "happy zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "happy:|usage: happy" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def is_happy(n):
    if n == 0:
        return False
    seen = set()
    while n != 1 and n not in seen:
        seen.add(n)
        n = sum(int(c) ** 2 for c in str(n))
    return n == 1

def line(n):
    if is_happy(n):
        return f"happy: {n} is a happy number"
    return f"happy: {n} is NOT a happy number (unhappy)"

# happy: 7, 19, 23, 100 ; unhappy: 4, 2, 0
for n in [7, 19, 23, 100, 4, 2, 0]:
    want = line(n)
    if want not in s:
        print(f"HAPPY TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity-pin the classic set + the 0 guard (must not hang / must be unhappy)
assert is_happy(7) and is_happy(19) and is_happy(100)
assert not is_happy(4) and not is_happy(2) and not is_happy(0)

if "usage: happy <n>" not in s:
    print("HAPPY TEST FAILED (missing usage line for bad arg)"); sys.exit(1)

print("HAPPY TEST PASSED  (7/19/23/100 happy, 4/2/0 unhappy vs python, 0 guarded, "
      "bad->usage)")
PY
