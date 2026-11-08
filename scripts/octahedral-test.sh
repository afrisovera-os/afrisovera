#!/usr/bin/env bash
# Verify `octahedral <n>` computes the nth octahedral number O(n) = n(2n^2+1)/3: O(1)=1,
# O(2)=6, O(5)=85, O(10)=670, O(100)=666700 (OEIS A005900), and the u64 maximum overflows
# u128. 3D companion of tetrahedral/squarepyramidal/pentatope. Self-checking: python computes
# n(2n^2+1)//3 with its own big ints and the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/octahedral.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/octahedral.png" \
    "echo warmup" "octahedral 1" "octahedral 5" "octahedral 10" \
    "octahedral 100" "octahedral 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "octahedral:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(n):
    val = n * (2 * n * n + 1)                 # DERIVED with python big ints
    if val > U128:                            # same guard as the u128 checked kernel path
        return f"octahedral: O({n}) overflowed u128"
    return f"octahedral: O({n}) = {val // 3}"

for n in [1, 5, 10, 100, 18446744073709551615]:
    w = line(n)
    if w not in s:
        print(f"OCTAHEDRAL TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable branches: base, small, ten, hundred, u64-max overflow
assert "octahedral: O(1) = 1" in s
assert "octahedral: O(5) = 85" in s
assert "octahedral: O(10) = 670" in s
assert "octahedral: O(100) = 666700" in s
assert "octahedral: O(18446744073709551615) overflowed u128" in s

print("OCTAHEDRAL TEST PASSED  (1->1, 5->85, 10->670, 100->666700, u64-max overflows "
      "vs python n(2n^2+1)//3)")
PY
