#!/usr/bin/env bash
# Verify the `totient` command computes Euler's phi(n) via the sqrt-time prime-
# factorization product formula (deterministic): phi(9)=6, phi(10)=4, phi(97)=96
# (prime -> n-1), phi(1000)=400; n=0 falls through to usage.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4796,server,nowait \
    -qmp tcp:127.0.0.1:4797,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/totient.out"
python3 scripts/shell-demo.py 4796 4797 "${TMPDIR:-/tmp}/totient.png" \
    "totient 9" "totient 10" "totient 97" "totient 1000" "totient 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "totient:|usage: totient" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "totient: phi(9) = 6",
    "totient: phi(10) = 4",
    "totient: phi(97) = 96",
    "totient: phi(1000) = 400",
    "usage: totient <n>",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"TOTIENT TEST FAILED (missing: {missing})"); sys.exit(1)
print("TOTIENT TEST PASSED  (phi(9)=6, phi(10)=4, phi(97)=96, phi(1000)=400; 0 rejected)")
PY
