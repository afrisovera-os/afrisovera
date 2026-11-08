#!/usr/bin/env bash
# Verify the `divisors` command reports tau(n) (divisor count) and sigma(n) (divisor
# sum) via sqrt-time trial division (deterministic): 12 -> 6 divisors sum 28; 28 ->
# 6 sum 56 (perfect); 97 -> 2 sum 98 (prime); 36 -> 9 sum 91 (perfect square, tests
# the e!=d guard against double-counting sqrt); n=0 -> usage.
set -uo pipefail

BIN="target/aarch64-unknown-none/debug/mansa-kernel"
cargo build -p mansa-kernel

qemu-system-aarch64 -M virt -cpu cortex-a72 -smp 1 -m 256M \
    -global virtio-mmio.force-legacy=false -device ramfb -display none \
    -serial tcp:127.0.0.1:4820,server,nowait \
    -qmp tcp:127.0.0.1:4821,server,nowait -kernel "$BIN" &
QPID=$!
. "$(dirname "$0")/lib/afrish-boot.sh"
afrish_wait_boot
OUT="${TMPDIR:-/tmp}/divisors.out"
python3 scripts/shell-demo.py 4820 4821 "${TMPDIR:-/tmp}/divisors.png" \
    "divisors 12" "divisors 28" "divisors 97" "divisors 36" "divisors 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "divisors:|usage: divisors" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")
want = [
    "divisors: 12 has 6 divisors, sum 28",
    "divisors: 28 has 6 divisors, sum 56",
    "divisors: 97 has 2 divisors, sum 98",
    "divisors: 36 has 9 divisors, sum 91",
    "usage: divisors <n>",
]
missing = [w for w in want if w not in s]
if missing:
    print(f"DIVISORS TEST FAILED (missing: {missing})"); sys.exit(1)
print("DIVISORS TEST PASSED  (12->6/28, 28->6/56, 97->2/98, 36->9/91; 0 rejected)")
PY
