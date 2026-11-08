#!/usr/bin/env bash
# Verify the `ncr` command computes the binomial coefficient C(n,r). Constants
# confirmed against Python's math.comb:
#   C(5,2)  = 10
#   C(52,5) = 2598960   (5-card poker hands)
#   C(3,5)  = 0         (r > n)
#   C(0,0)  = 1
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4782,server,nowait \
    -qmp tcp:127.0.0.1:4783,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/ncr.out"
python3 scripts/shell-demo.py 4782 4783 "${TMPDIR:-/tmp}/ncr.png" "ncr 5 2" "ncr 52 5" "ncr 3 5" "ncr 0 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "ncr: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["ncr: C(5, 2) = 10", "ncr: C(52, 5) = 2598960", "ncr: C(3, 5) = 0", "ncr: C(0, 0) = 1"]
missing = [w for w in want if w not in s]
if missing:
    print(f"NCR TEST FAILED (missing: {missing})"); sys.exit(1)
print("NCR TEST PASSED  (C(5,2)=10; C(52,5)=2598960; C(3,5)=0; C(0,0)=1)")
PY
