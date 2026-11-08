#!/usr/bin/env bash
# Verify `centeredcube <n>` computes the nth centered cube number n^3+(n-1)^3 (OEIS A005898):
# C(1)=1, C(2)=9, C(3)=35, C(10)=1729 (Hardy-Ramanujan taxicab), C(100)=1970299; the u64 max
# overflows u128. Self-checking: python n**3+(n-1)**3 with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/centeredcube.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredcube.png" \
    "echo warmup" "centeredcube 1" "centeredcube 2" "centeredcube 3" \
    "centeredcube 10" "centeredcube 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredcube:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(n):
    v = n ** 3 + (n - 1) ** 3           # DERIVED centered cube
    if v > U128:
        return f"centeredcube: C({n}) overflowed u128"
    return f"centeredcube: C({n}) = {v}"

for n in [1, 2, 3, 10, 18446744073709551615]:
    w = line(n)
    if w not in s:
        print(f"CENTEREDCUBE TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable cases: base, small, third, the taxicab number, u64-max overflow
assert "centeredcube: C(1) = 1" in s
assert "centeredcube: C(2) = 9" in s
assert "centeredcube: C(3) = 35" in s
assert "centeredcube: C(10) = 1729" in s          # Hardy-Ramanujan
assert "centeredcube: C(18446744073709551615) overflowed u128" in s

print("CENTEREDCUBE TEST PASSED  (1->1, 2->9, 3->35, 10->1729 taxicab, u64-max overflows "
      "vs python n^3+(n-1)^3)")
PY
