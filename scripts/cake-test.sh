#!/usr/bin/env bash
# Verify `cake <n>` computes the cake numbers C(n)=(n^3+5n+6)/6 (1,2,4,8,15,26,42,64,93,130,176,...) --
# max pieces of a cake with n planar cuts, the 3D lazy caterer. Tests n=0,3,4,5,10. Self-checking:
# python (n**3+5*n+6)//6.
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
OUT="${TMPDIR:-/tmp}/cake.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/cake.png" \
    "echo warmup" "cake 0" "cake 3" "cake 4" "cake 5" "cake 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "cake:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def c(n): return (n ** 3 + 5 * n + 6) // 6

for n in (0, 3, 4, 5, 10):
    want = f"cake: C({n}) = {c(n)}"
    if want not in s:
        print(f"CAKE TEST FAILED (missing '{want}')"); sys.exit(1)

assert [c(n) for n in range(11)] == [1, 2, 4, 8, 15, 26, 42, 64, 93, 130, 176]
print("CAKE TEST PASSED  (C(0)=1, C(3)=8, C(4)=15, C(5)=26, C(10)=176 vs python)")
PY
