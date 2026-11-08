#!/usr/bin/env bash
# Verify `centeredpentagonal <n>` computes CP(n) = 5n(n-1)/2 + 1 (a dot surrounded by pentagonal
# rings): CP(1..6) = 1, 6, 16, 31, 51, 76 (OEIS A005891). CP(100)=24751; u64-max overflows u128.
# Completes the centered-polygonal family (triangular/square/hex/star). Self-checking: python
# computes every reference with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/centeredpentagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredpentagonal.png" \
    "echo warmup" "centeredpentagonal 2" "centeredpentagonal 5" "centeredpentagonal 100" \
    "centeredpentagonal 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredpentagonal:|usage: centeredpentagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: (n*n - n)/2 * 5 + 1 in u128 with overflow guard
    sq = n * n
    if sq > LIM:
        return f"centeredpentagonal: CP({n}) overflowed u128"
    half = (sq - n) // 2
    t = half * 5
    if t > LIM:
        return f"centeredpentagonal: CP({n}) overflowed u128"
    return f"centeredpentagonal: CP({n}) = {t + 1}"

for n in [2, 5, 100, 18446744073709551615]:
    want = message(n)   # every value DERIVED by python, never hand-typed
    if want not in s:
        print(f"CENTEREDPENTAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches: small members + a larger value + the overflow branch
assert "centeredpentagonal: CP(2) = 6" in s
assert "centeredpentagonal: CP(5) = 51" in s
assert "centeredpentagonal: CP(100) = 24751" in s
assert "centeredpentagonal: CP(18446744073709551615) overflowed u128" in s

print("CENTEREDPENTAGONAL TEST PASSED  (CP(2)=6, CP(5)=51, CP(100)=24751, u64-max overflows "
      "vs python 5n(n-1)/2+1)")
PY
