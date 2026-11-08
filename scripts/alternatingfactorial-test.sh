#!/usr/bin/env bash
# Verify `alternatingfactorial <n>` computes af(n) = n!-(n-1)!+..-+1! via af(0)=0,
# af(k)=k!-af(k-1) in u128: 0,1,1,5,19,101,619,... (OEIS A005165). af(5)=101,
# af(10)=3301819, af(34) fits u128, af(35) overflows. Distinct from the factorial
# family. Self-checking: python computes every reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/alternatingfactorial.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/alternatingfactorial.png" \
    "echo warmup" "alternatingfactorial 0" "alternatingfactorial 5" "alternatingfactorial 34" "alternatingfactorial 35" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "alternatingfactorial:|usage: alternatingfactorial" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(n):  # mirror the kernel: af(0)=0, af(k)=k!-af(k-1), u128 overflow guard
    if n == 0:
        return "alternatingfactorial: af(0) = 0"
    fact = 1; af = 0
    for k in range(1, n + 1):
        fact *= k
        if fact > LIM:
            return f"alternatingfactorial: af({n}) overflows u128"
        af = fact - af
    return f"alternatingfactorial: af({n}) = {af}"

for n in [0, 5, 34, 35]:
    want = message(n)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"ALTFACT TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "alternatingfactorial: af(0) = 0" in s
assert "alternatingfactorial: af(5) = 101" in s
assert "alternatingfactorial: af(34) = 286804646124557439494797475697635577019" in s
assert "alternatingfactorial: af(35) overflows u128" in s

print("ALTFACT TEST PASSED  (af(0)=0, af(5)=101, af(34)=big, af(35) overflows u128 "
      "vs python n!-(n-1)!+..)")
PY
