#!/usr/bin/env bash
# Verify the `primes` command lists primes up to N via the Sieve of Eratosthenes.
# The output is fully deterministic, so exact-line assertions prove correctness:
#   primes 30 -> "primes: 30 = 2 3 5 7 11 13 17 19 23 29"
#   primes 10 -> "primes: 10 = 2 3 5 7"
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4696,server,nowait \
    -qmp tcp:127.0.0.1:4697,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/primes.out"
python3 scripts/shell-demo.py 4696 4697 "${TMPDIR:-/tmp}/primes.png" "primes 30" "primes 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "primes: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = ["primes: 30 = 2 3 5 7 11 13 17 19 23 29", "primes: 10 = 2 3 5 7"]
missing = [w for w in want if w not in s]
if missing:
    print(f"PRIMES TEST FAILED (missing exact line(s): {missing})"); sys.exit(1)
print("PRIMES TEST PASSED  (primes 30 + primes 10 exact)")
PY
