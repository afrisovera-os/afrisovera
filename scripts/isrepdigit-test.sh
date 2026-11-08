#!/usr/bin/env bash
# Verify the `isrepdigit <n>` command classifies repdigits (monodigits): all of
# n's decimal digits are identical (7, 11, 55, 777, 999999, ...). Single-digit
# numbers including 0 trivially qualify. Distinct from maxdigit (extreme digits),
# digitcount (counts digits), and uniqchars (dedups a text string). Self-checking:
# python all(c == s[0] for c in str(n)).
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
OUT="${TMPDIR:-/tmp}/isrepdigit.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isrepdigit.png" \
    "echo warmup" "isrepdigit 777" "isrepdigit 121" "isrepdigit 999999" "isrepdigit 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isrepdigit:|usage: isrepdigit" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isrep(n):
    t = str(n)
    return all(c == t[0] for c in t)

# IS cases (with the shared digit = last digit)
for n in [777, 999999, 0]:
    assert isrep(n), f"python: {n} should be a repdigit"
    d = n % 10
    want = f"isrepdigit: {n} IS a repdigit (all digits are {d})"
    if want not in s:
        print(f"ISREPDIGIT TEST FAILED (missing '{want}')"); sys.exit(1)

# NOT case
assert not isrep(121)
if "isrepdigit: 121 is NOT a repdigit" not in s:
    print("ISREPDIGIT TEST FAILED (121 should be NOT)"); sys.exit(1)

# pin the single-digit and multi-digit cases explicitly
assert "isrepdigit: 777 IS a repdigit (all digits are 7)" in s
assert "isrepdigit: 0 IS a repdigit (all digits are 0)" in s   # 0 is a single digit

print("ISREPDIGIT TEST PASSED  (777/999999/0 IS, 121 NOT via digit walk vs python; "
      "single-digit 0 trivially IS; distinct from maxdigit/digitcount)")
PY
