#!/usr/bin/env bash
# Verify the `isdigitsorted <n>` command classifies whether n's decimal digits are
# NON-DECREASING left to right (1234, 1122, 1119 IS; 4321, 1032 NOT). Single-digit
# numbers including 0 trivially qualify. A right-to-left digit walk: each more-
# significant digit must be <= the digit to its right. Distinct from isrepdigit
# (all digits EQUAL) and maxdigit (extreme digit values) -- this checks digit
# ORDER. Self-checking: python all(s[i] <= s[i+1] for i in range(len(s)-1)).
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
OUT="${TMPDIR:-/tmp}/isdigitsorted.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isdigitsorted.png" \
    "echo warmup" "isdigitsorted 1234" "isdigitsorted 4321" "isdigitsorted 1119" "isdigitsorted 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isdigitsorted:|usage: isdigitsorted" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def issorted(n):
    t = str(n)
    return all(t[i] <= t[i + 1] for i in range(len(t) - 1))

# IS cases
for n in [1234, 1119, 0]:
    assert issorted(n), f"python: {n} should be digit-sorted"
    want = f"isdigitsorted: {n} IS digit-sorted (digits non-decreasing)"
    if want not in s:
        print(f"ISDIGITSORTED TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT case
assert not issorted(4321)
if "isdigitsorted: 4321 is NOT digit-sorted" not in s:
    print("ISDIGITSORTED TEST FAILED (4321 should be NOT)"); sys.exit(1)

# pin the notable cases explicitly
assert "isdigitsorted: 1234 IS digit-sorted (digits non-decreasing)" in s
assert "isdigitsorted: 0 IS digit-sorted (digits non-decreasing)" in s   # single digit

print("ISDIGITSORTED TEST PASSED  (1234/1119/0 IS, 4321 NOT via right-to-left digit "
      "walk vs python; single-digit 0 trivially IS; distinct from isrepdigit/maxdigit)")
PY
