#!/usr/bin/env bash
# Verify `woodall <n>` computes the nth Woodall number W(n) = n*2^n - 1 in u128:
# 1,7,23,63,159,... (OEIS A003261). W(5)=159, W(121) is the last that fits u128, W(122)
# overflows; W(0) is undefined (0*2^0-1 = -1). Distinct from cullen (n*2^n+1)/thabit.
# Self-checking: python computes every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/woodall.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/woodall.png" \
    "echo warmup" "woodall 0" "woodall 5" "woodall 121" "woodall 122" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "woodall" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: W(n)=n*2^n-1, u128 guard, W(0) undefined
    if n == 0:
        return "woodall: W(0) is undefined (0*2^0 - 1 = -1)"
    if n >= 128:
        return f"woodall({n}) overflows u128"
    v = n * (1 << n)
    if v > LIM:
        return f"woodall({n}) overflows u128"
    return f"woodall({n}) = {v - 1}"

for n in [0, 5, 121, 122]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"WOODALL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "woodall: W(0) is undefined (0*2^0 - 1 = -1)" in s
assert "woodall(5) = 159" in s
assert "woodall(121) = 321673174979949641242721308587843387391" in s
assert "woodall(122) overflows u128" in s

print("WOODALL TEST PASSED  (W(0) undefined, W(5)=159, W(121)=big, W(122) overflows u128 "
      "vs python n*2^n-1)")
PY
