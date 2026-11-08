#!/usr/bin/env bash
# Verify the `squarefree <n>` command classifies whether n (n>=1) is SQUAREFREE:
# no prime factor appears more than once, i.e. no perfect square > 1 divides n.
# 1,2,3,5,6,7,10,30,... yes; 4,8,9,12,18,100,... no. n=0 -> usage (n>=1 guard).
# Self-checking: python recomputes the verdict (no i*i divides n).
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
OUT="${TMPDIR:-/tmp}/squarefree.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/squarefree.png" \
    "echo warmup" "squarefree 1" "squarefree 6" "squarefree 10" "squarefree 30" \
    "squarefree 4" "squarefree 9" "squarefree 12" "squarefree 100" \
    "squarefree 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "squarefree:|usage: squarefree" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sf(n):
    i = 2
    while i * i <= n:
        if n % (i * i) == 0:
            return False
        i += 1
    return True

def line(n):
    return f"squarefree: {n} IS squarefree" if sf(n) else f"squarefree: {n} is NOT squarefree"

# squarefree: 1,6,10,30 ; not: 4,9,12,100
for n in [1, 6, 10, 30, 4, 9, 12, 100]:
    want = line(n)
    if want not in s:
        print(f"SQUAREFREE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin notable values explicitly
assert line(30) == "squarefree: 30 IS squarefree"   # 2*3*5, all distinct
assert line(100) == "squarefree: 100 is NOT squarefree"  # 2^2 * 5^2
assert line(1) == "squarefree: 1 IS squarefree"      # empty product

# n=0 must hit the usage line (n>=1 guard), NOT a verdict
if "usage: squarefree <n>" not in s:
    print("SQUAREFREE TEST FAILED (n=0 should report usage)"); sys.exit(1)
if "squarefree: 0 " in s:
    print("SQUAREFREE TEST FAILED (n=0 should NOT produce a verdict)"); sys.exit(1)

print("SQUAREFREE TEST PASSED  (1/6/10/30 yes, 4/9/12/100 no vs python; "
      "n=0 -> usage, no verdict)")
PY
