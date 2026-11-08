#!/usr/bin/env bash
# Verify the `isprime` command's primality predicate. Deterministic:
#   isprime 97   -> prime
#   isprime 100  -> not prime (divisible by 2)
#   isprime 7919 -> prime (the 1000th prime)
#   isprime 1    -> not prime
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4774,server,nowait \
    -qmp tcp:127.0.0.1:4775,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/isprime.out"
python3 scripts/shell-demo.py 4774 4775 "${TMPDIR:-/tmp}/isprime.png" "isprime 97" "isprime 100" "isprime 7919" "isprime 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isprime: " "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "isprime: 97 is prime",
    "isprime: 100 is not prime",
    "isprime: 7919 is prime",
    "isprime: 1 is not prime",
]
missing = [w for w in want if w not in s]
# Guard against false positives: 97/7919 must NOT say "not prime".
if "isprime: 97 is not prime" in s or "isprime: 7919 is not prime" in s:
    print("ISPRIME TEST FAILED (a known prime was reported composite)"); sys.exit(1)
if missing:
    print(f"ISPRIME TEST FAILED (missing: {missing})"); sys.exit(1)
print("ISPRIME TEST PASSED  (97 prime; 100 composite; 7919 prime; 1 not prime)")
PY
