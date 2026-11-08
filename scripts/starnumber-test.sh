#!/usr/bin/env bash
# Verify the `starnumber <n>` command computes the nth (centered) star number
# S(n) = 6n(n-1)+1 -- a centred hexagram (Star of David) figurate number.
# S(1..6) = 1,13,37,73,121,181. Parallels centeredhex (3n(n-1)+1) with coefficient 6.
# Evaluated as 6*n*n - 6*n + 1 in u128 (underflow-safe for n=0, checked_mul guards).
# Self-checking: python computes the references (not hand-typed).
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
OUT="${TMPDIR:-/tmp}/starnumber.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 0 pins the underflow-safe edge (S(0)=1); 100 a larger value.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/starnumber.png" \
    "echo warmup" "starnumber 2" "starnumber 6" "starnumber 0" "starnumber 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "starnumber:|usage: starnumber" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sn(n):
    return 6 * n * n - 6 * n + 1

for n in [2, 6, 0, 100]:
    want = f"starnumber: S({n}) = {sn(n)}"
    if want not in s:
        print(f"STARNUMBER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results (computed, not hand-typed)
assert f"starnumber: S(2) = {sn(2)}" in s      # 13
assert f"starnumber: S(6) = {sn(6)}" in s      # 181
assert f"starnumber: S(0) = {sn(0)}" in s      # 1 (underflow-safe edge)
assert f"starnumber: S(100) = {sn(100)}" in s  # 59401

print("STARNUMBER TEST PASSED  (S(2)=13, S(6)=181, S(0)=1, S(100)=59401 vs python; "
      "6n(n-1)+1, parallels centeredhex with coefficient 6)")
PY
