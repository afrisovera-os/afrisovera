#!/usr/bin/env bash
# Verify `centeredheptagonal <n>` computes CH(n) = 7n(n-1)/2 + 1 (a dot surrounded by heptagonal
# rings): CH(1..6) = 1, 8, 22, 43, 71, 106 (OEIS A069099). CH(100)=34651; u64-max overflows u128.
# Extends the centered-polygonal family to k=7. Self-checking: python computes every reference
# with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/centeredheptagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredheptagonal.png" \
    "echo warmup" "centeredheptagonal 2" "centeredheptagonal 5" "centeredheptagonal 100" \
    "centeredheptagonal 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredheptagonal:|usage: centeredheptagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: (n*n - n)/2 * 7 + 1 in u128 with overflow guard
    sq = n * n
    if sq > LIM:
        return f"centeredheptagonal: CH({n}) overflowed u128"
    half = (sq - n) // 2
    t = half * 7
    if t > LIM:
        return f"centeredheptagonal: CH({n}) overflowed u128"
    return f"centeredheptagonal: CH({n}) = {t + 1}"

for n in [2, 5, 100, 18446744073709551615]:
    want = message(n)   # every value DERIVED by python, never hand-typed
    if want not in s:
        print(f"CENTEREDHEPTAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: small members + a larger value + the overflow branch
assert "centeredheptagonal: CH(2) = 8" in s
assert "centeredheptagonal: CH(5) = 71" in s
assert "centeredheptagonal: CH(100) = 34651" in s
assert "centeredheptagonal: CH(18446744073709551615) overflowed u128" in s

print("CENTEREDHEPTAGONAL TEST PASSED  (CH(2)=8, CH(5)=71, CH(100)=34651, u64-max overflows "
      "vs python 7n(n-1)/2+1)")
PY
