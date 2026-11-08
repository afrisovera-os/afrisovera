#!/usr/bin/env bash
# Verify `squarepyramidal <n>` computes SP(n) = n(n+1)(2n+1)/6 in u128 (sum of first n
# squares): SP(1..6)=1,5,14,30,55,91 (OEIS A000330). SP(5)=55, SP(10)=385, SP(100)=338350;
# u64-max overflows. Distinct from tetrahedral. Self-checking: python computes every ref.
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
OUT="${TMPDIR:-/tmp}/squarepyramidal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/squarepyramidal.png" \
    "echo warmup" "squarepyramidal 5" "squarepyramidal 10" "squarepyramidal 100" "squarepyramidal 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "squarepyramidal:|usage: squarepyramidal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: n(n+1)(2n+1)/6, u128 overflow guard on the products
    p = n * (n + 1)
    if p > LIM:
        return f"squarepyramidal: SP({n}) overflowed u128"
    p2 = p * (2 * n + 1)
    if p2 > LIM:
        return f"squarepyramidal: SP({n}) overflowed u128"
    return f"squarepyramidal: SP({n}) = {p2 // 6}"

for n in [5, 10, 100, 18446744073709551615]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"SQUAREPYRAMIDAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "squarepyramidal: SP(5) = 55" in s
assert "squarepyramidal: SP(10) = 385" in s
assert "squarepyramidal: SP(100) = 338350" in s
assert "squarepyramidal: SP(18446744073709551615) overflowed u128" in s

print("SQUAREPYRAMIDAL TEST PASSED  (SP(5)=55, SP(10)=385, SP(100)=338350, u64-max "
      "overflows vs python n(n+1)(2n+1)/6)")
PY
