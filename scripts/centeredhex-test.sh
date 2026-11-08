#!/usr/bin/env bash
# Verify the `centeredhex <n>` command computes the nth centered hexagonal number
# CH(n) = 3n(n-1)+1 (a centre dot surrounded by hexagonal rings; the honeycomb count).
# CH(1..7) = 1,7,19,37,61,91,127. Distinct from the figurate MEMBERSHIP test hexagonal
# (which classifies n = k(2k-1)). Evaluated as 3*n*n - 3*n + 1 in u128 (underflow-safe
# for n=0, checked_mul guards overflow). Self-checking: python computes the references.
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
OUT="${TMPDIR:-/tmp}/centeredhex.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 0 pins the underflow-safe edge (CH(0)=1); 100 a larger value.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredhex.png" \
    "echo warmup" "centeredhex 4" "centeredhex 7" "centeredhex 0" "centeredhex 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredhex:|usage: centeredhex" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def ch(n):
    return 3 * n * n - 3 * n + 1

for n in [4, 7, 0, 100]:
    want = f"centeredhex: CH({n}) = {ch(n)}"
    if want not in s:
        print(f"CENTEREDHEX TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results (computed, not hand-typed)
assert f"centeredhex: CH(4) = {ch(4)}" in s      # 37
assert f"centeredhex: CH(7) = {ch(7)}" in s      # 127
assert f"centeredhex: CH(0) = {ch(0)}" in s      # 1 (underflow-safe edge)
assert f"centeredhex: CH(100) = {ch(100)}" in s  # 29701

print("CENTEREDHEX TEST PASSED  (CH(4)=37, CH(7)=127, CH(0)=1, CH(100)=29701 vs python; "
      "3n(n-1)+1, distinct from the hexagonal membership test)")
PY
