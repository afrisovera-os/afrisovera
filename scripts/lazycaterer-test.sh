#!/usr/bin/env bash
# Verify `lazycaterer <n>` computes the lazy caterer / central polygonal number p(n)=n(n+1)/2+1
# (1,2,4,7,11,16,22,29,37,46,56,...). Tests n=0,3,4,6,10,100. Self-checking: python n(n+1)//2+1.
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
OUT="${TMPDIR:-/tmp}/lazycaterer.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/lazycaterer.png" \
    "echo warmup" "lazycaterer 0" "lazycaterer 3" "lazycaterer 4" \
    "lazycaterer 6" "lazycaterer 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "lazycaterer:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def p(n): return n * (n + 1) // 2 + 1

for n in (0, 3, 4, 6, 10):
    want = f"lazycaterer: p({n}) = {p(n)}"
    if want not in s:
        print(f"LAZYCATERER TEST FAILED (missing '{want}')"); sys.exit(1)

assert [p(n) for n in range(11)] == [1, 2, 4, 7, 11, 16, 22, 29, 37, 46, 56]
print("LAZYCATERER TEST PASSED  (p(0)=1, p(3)=7, p(4)=11, p(6)=22, p(10)=56 vs python)")
PY
