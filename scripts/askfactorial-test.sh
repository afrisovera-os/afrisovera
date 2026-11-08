#!/usr/bin/env bash
# Verify the Asili assistant's NEW factorial capability (docs/09 AI pillar): "ask 5 factorial"->120,
# "ask factorial of 6"->720, "ask 0 factorial"->1, "ask what is 10 factorial"->3628800, "ask 1
# factorial"->1. Iterative n!, capped n<=20 (21! overflows i64). Dispatched BEFORE try_factorize so
# "factorial" is not swallowed by the "factor" guard. Self-checking: python math.factorial.
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
OUT="${TMPDIR:-/tmp}/askfactorial.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/askfactorial.png" \
    "echo warmup" "ask 5 factorial" "ask factorial of 6" "ask 0 factorial" \
    "ask what is 10 factorial" "ask 1 factorial" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "\[asili\]" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

# derived expectations: 5!=120, 6!=720, 0!=1, 10!=3628800, 1!=1
for n in (5, 6, 0, 10, 1):
    exp = f"[asili] = {math.factorial(n)} (FACTORIAL, ON-DEVICE)"
    if exp not in s:
        print(f"ASKFACTORIAL TEST FAILED (missing '{exp}' for n={n})"); sys.exit(1)

print("ASKFACTORIAL TEST PASSED  (5!->120, factorial of 6->720, 0!->1, 10!->3628800, 1!->1; "
      "on-device Asili factorial, dispatched before factorize)")
PY
