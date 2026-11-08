#!/usr/bin/env bash
# Verify `centeredsquare <n>` computes CS(n) = 2n(n-1)+1 in u128: CS(1..6)=1,5,13,25,41,61
# (OEIS A001844). CS(5)=41, CS(10)=181, CS(100)=19801; u64-max overflows u128. Part of the
# centered-polygonal family (centeredhex/starnumber). Self-checking: python computes refs.
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
OUT="${TMPDIR:-/tmp}/centeredsquare.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/centeredsquare.png" \
    "echo warmup" "centeredsquare 5" "centeredsquare 10" "centeredsquare 100" "centeredsquare 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "centeredsquare:|usage: centeredsquare" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: CS(n)=2n(n-1)+1 = 2n^2-2n+1, u128 overflow guard
    t = 2 * n * n
    if t > LIM:
        return f"centeredsquare: CS({n}) overflowed u128"
    return f"centeredsquare: CS({n}) = {t - 2 * n + 1}"

for n in [5, 10, 100, 18446744073709551615]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"CENTEREDSQUARE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "centeredsquare: CS(5) = 41" in s
assert "centeredsquare: CS(10) = 181" in s
assert "centeredsquare: CS(100) = 19801" in s
assert "centeredsquare: CS(18446744073709551615) overflowed u128" in s

print("CENTEREDSQUARE TEST PASSED  (CS(5)=41, CS(10)=181, CS(100)=19801, u64-max overflows "
      "vs python 2n(n-1)+1)")
PY
