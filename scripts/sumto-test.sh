#!/usr/bin/env bash
# Verify the `sumto` command computes the nth triangular number n(n+1)/2.
# Constants confirmed against Python:
#   T(100)     = 5050
#   T(10)      = 55
#   T(1000000) = 500000500000   (exceeds u32; u128 keeps it exact)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4794,server,nowait \
    -qmp tcp:127.0.0.1:4795,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/sumto.out"
python3 scripts/shell-demo.py 4794 4795 "${TMPDIR:-/tmp}/sumto.png" "sumto 100" "sumto 10" "sumto 1000000" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumto: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "sumto: 1+2+...+100 = 5050",
    "sumto: 1+2+...+10 = 55",
    "sumto: 1+2+...+1000000 = 500000500000",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"SUMTO TEST FAILED (missing: {missing})"); sys.exit(1)
print("SUMTO TEST PASSED  (T(100)=5050; T(10)=55; T(1000000)=500000500000)")
PY
