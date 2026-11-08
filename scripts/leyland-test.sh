#!/usr/bin/env bash
# Verify `leyland <x> <y>` computes the Leyland number L(x,y) = x^y + y^x in u128:
# L(2,3)=17, L(2,4)=32, L(10,10)=20000000000; L(100,100) overflows u128. Symmetric.
# Distinct from pow (modular)/cullen/woodall/thabit. Self-checking: python computes every
# reference value (never hand-typed).
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
OUT="${TMPDIR:-/tmp}/leyland.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/leyland.png" \
    "echo warmup" "leyland 2 3" "leyland 2 4" "leyland 10 10" "leyland 100 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "leyland:|usage: leyland" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
LIM = (1 << 128) - 1

def message(x, y):  # mirror the kernel: L(x,y)=x^y+y^x, u128 overflow guard
    a, b = x ** y, y ** x
    if a > LIM or b > LIM or a + b > LIM:
        return f"leyland: L({x}, {y}) overflows u128"
    return f"leyland: L({x}, {y}) = {a + b}"

for x, y in [(2, 3), (2, 4), (10, 10), (100, 100)]:
    want = message(x, y)   # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"LEYLAND TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "leyland: L(2, 3) = 17" in s
assert "leyland: L(2, 4) = 32" in s
assert "leyland: L(10, 10) = 20000000000" in s
assert "leyland: L(100, 100) overflows u128" in s

print("LEYLAND TEST PASSED  (L(2,3)=17, L(2,4)=32, L(10,10)=20000000000, L(100,100) "
      "overflows u128 vs python x^y+y^x)")
PY
