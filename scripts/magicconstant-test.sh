#!/usr/bin/env bash
# Verify the `magicconstant <n>` command computes M(n) = n*(n^2+1)/2, the magic
# constant of an order-n magic square (M(3)=15, M(4)=34, M(5)=65, ...). Large n
# reports overflow instead of panicking. Self-checking: python computes
# n*(n**2+1)//2 (values derived by the loop, NOT hand-typed) and predicts the u128
# overflow boundary.
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
OUT="${TMPDIR:-/tmp}/magicconstant.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# n=3 (=15), n=10 (=505), n=1000000000 (fits u128), n=100000000000000 (overflows).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/magicconstant.png" \
    "echo warmup" "magicconstant 3" "magicconstant 10" "magicconstant 1000000000" "magicconstant 100000000000000" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "magicconstant\(|usage: magicconstant" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

UMAX = 2 ** 128 - 1
def magic(n):
    return n * (n * n + 1) // 2

for n in [3, 10, 1000000000]:
    # the kernel overflows only if the intermediate product n*(n^2+1) exceeds u128
    prod = n * (n * n + 1)
    assert prod <= UMAX, f"expected {n} to fit u128"
    want = f"magicconstant({n}) = {magic(n)}"      # value DERIVED by python, never hand-typed
    if want not in s:
        print(f"MAGICCONSTANT TEST FAILED (missing '{want}')"); sys.exit(1)

# n=1e14 overflows: n*(n^2+1) > u128 max -- python predicts the same boundary
n = 100000000000000
assert n * (n * n + 1) > UMAX
if "magicconstant(100000000000000) overflows u128" not in s:
    print("MAGICCONSTANT TEST FAILED (missing overflow line)"); sys.exit(1)

# small pins
assert "magicconstant(3) = 15" in s
assert "magicconstant(10) = 505" in s

print("MAGICCONSTANT TEST PASSED  (M(3)=15, M(10)=505, M(1e9) fits vs python, M(1e14) overflows; "
      "n(n^2+1)/2, u128 checked)")
PY
