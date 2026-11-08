#!/usr/bin/env bash
# Verify `digitsumbase <b> <n>` sums the digits of n in base b: 16 255 -> 30 (FF), 2 7 -> 3
# (111), 10 123 -> 6, 16 256 -> 1 (100), 5 100 -> 4 (400). Generalizes digitsum (base 10).
# Self-checking: python repeatedly adds n%b (independent of the kernel loop).
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
OUT="${TMPDIR:-/tmp}/digitsumbase.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/digitsumbase.png" \
    "echo warmup" "digitsumbase 16 255" "digitsumbase 2 7" "digitsumbase 10 123" \
    "digitsumbase 16 256" "digitsumbase 5 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "digitsumbase:|usage: digitsumbase" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dsb(b, n):
    total = 0
    while n > 0:
        total += n % b; n //= b            # DERIVED digit sum in base b
    return total

for b, n in [(16, 255), (2, 7), (10, 123), (16, 256), (5, 100)]:
    want = f"digitsumbase: digit sum of {n} in base {b} is {dsb(b, n)}"
    if want not in s:
        print(f"DIGITSUMBASE TEST FAILED (b={b} n={n} missing '{want}')"); sys.exit(1)

# pin the notable cases
assert "digitsumbase: digit sum of 255 in base 16 is 30" in s   # FF
assert "digitsumbase: digit sum of 7 in base 2 is 3" in s       # 111
assert "digitsumbase: digit sum of 123 in base 10 is 6" in s
assert "digitsumbase: digit sum of 256 in base 16 is 1" in s    # 100
assert "digitsumbase: digit sum of 100 in base 5 is 4" in s     # 400

print("DIGITSUMBASE TEST PASSED  (16/255->30, 2/7->3, 10/123->6, 16/256->1, 5/100->4 "
      "vs python base-b digit sum)")
PY
