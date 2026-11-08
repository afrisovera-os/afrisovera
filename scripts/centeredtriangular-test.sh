#!/usr/bin/env bash
# Verify `centeredtriangular <n>` computes CT(n) = 3n(n-1)/2 + 1 in u128: CT(1..6)=
# 1,4,10,19,31,46 (OEIS A005448). CT(5)=31, CT(10)=136, CT(100)=14851; u64-max overflows.
# Completes the centered-polygonal family. Self-checking: python computes every reference.
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
OUT="${TMPDIR:-/tmp}/centeredtriangular.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredtriangular.png" \
    "echo warmup" "centeredtriangular 5" "centeredtriangular 10" "centeredtriangular 100" "centeredtriangular 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredtriangular:|usage: centeredtriangular" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: (n*n - n)/2 * 3 + 1, u128 overflow guard
    sq = n * n
    if sq > LIM:
        return f"centeredtriangular: CT({n}) overflowed u128"
    t = ((sq - n) // 2) * 3
    if t > LIM:
        return f"centeredtriangular: CT({n}) overflowed u128"
    return f"centeredtriangular: CT({n}) = {t + 1}"

for n in [5, 10, 100, 18446744073709551615]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"CENTEREDTRIANGULAR TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "centeredtriangular: CT(5) = 31" in s
assert "centeredtriangular: CT(10) = 136" in s
assert "centeredtriangular: CT(100) = 14851" in s
assert "centeredtriangular: CT(18446744073709551615) overflowed u128" in s

print("CENTEREDTRIANGULAR TEST PASSED  (CT(5)=31, CT(10)=136, CT(100)=14851, u64-max "
      "overflows vs python 3n(n-1)/2+1)")
PY
