#!/usr/bin/env bash
# Verify the `pow` command computes modular exponentiation b^e mod m by
# square-and-multiply. Deterministic, hand-checkable cases:
#   pow 2 10 1000 -> "pow: 2^10 mod 1000 = 24"   (1024 mod 1000)
#   pow 3 5 7     -> "pow: 3^5 mod 7 = 5"         (243 mod 7)
#   pow 7 0 13    -> "pow: 7^0 mod 13 = 1"        (identity)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4710,server,nowait \
    -qmp tcp:127.0.0.1:4711,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/pow.out"
python3 scripts/shell-demo.py 4710 4711 "${TMPDIR:-/tmp}/pow.png" "pow 2 10 1000" "pow 3 5 7" "pow 7 0 13" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pow: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["pow: 2^10 mod 1000 = 24", "pow: 3^5 mod 7 = 5", "pow: 7^0 mod 13 = 1"]
missing = [w for w in want if w not in s]
if missing:
    print(f"POW TEST FAILED (missing exact substring(s): {missing})"); sys.exit(1)
print("POW TEST PASSED  (2^10 mod 1000=24; 3^5 mod 7=5; 7^0 mod 13=1)")
PY
