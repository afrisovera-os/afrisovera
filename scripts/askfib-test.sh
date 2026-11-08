#!/usr/bin/env bash
# Verify the Asili assistant's NEW Fibonacci capability (docs/09 AI pillar): "ask the 10th fibonacci
# number"->55, "ask fibonacci 20"->6765, "ask fib 0"->0, "ask fib 1"->1, "ask fibonacci 7"->13.
# Iterative F(0)=0,F(1)=1, computed on-device, capped at n<=92 (F(93) overflows i64). Self-checking.
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
OUT="${TMPDIR:-/tmp}/askfib.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askfib.png" \
    "echo warmup" "ask the 10th fibonacci number" "ask fibonacci 20" "ask fib 0" \
    "ask fib 1" "ask fibonacci 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def fib(n):
    a, b = 0, 1
    for _ in range(n):
        a, b = b, a + b
    return a

# derived expectations: F(10)=55, F(20)=6765, F(0)=0, F(1)=1, F(7)=13
for n in (10, 20, 0, 1, 7):
    exp = f"[asili] = {fib(n)} (NTH FIBONACCI, ON-DEVICE)"
    if exp not in s:
        print(f"ASKFIB TEST FAILED (missing '{exp}' for n={n})"); sys.exit(1)

print("ASKFIB TEST PASSED  (10th->55, fibonacci 20->6765, fib 0->0, fib 1->1, fibonacci 7->13; "
      "on-device Asili Fibonacci)")
PY
