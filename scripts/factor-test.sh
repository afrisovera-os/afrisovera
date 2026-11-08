#!/usr/bin/env bash
# Verify the `factor` command prime-factorizes integers by trial division. The
# output is fully deterministic, so exact-line assertions prove correctness:
#   factor 60 -> "factor: 60 = 2 2 3 5"   (composite, repeated + distinct primes)
#   factor 97 -> "factor: 97 = 97"        (a prime factors to itself)
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4694,server,nowait \
    -qmp tcp:127.0.0.1:4695,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/factor.out"
python3 scripts/shell-demo.py 4694 4695 "${TMPDIR:-/tmp}/factor.png" "factor 60" "factor 97" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "factor: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["factor: 60 = 2 2 3 5", "factor: 97 = 97"]
missing = [w for w in want if w not in s]
if missing:
    print(f"FACTOR TEST FAILED (missing exact line(s): {missing})"); sys.exit(1)
print("FACTOR TEST PASSED  (factor 60 = 2 2 3 5; factor 97 = 97)")
PY
