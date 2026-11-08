#!/usr/bin/env bash
# Verify `minprimefactor <n>` gives the smallest prime factor (A020639, lpf): 15->3, 100->2,
# 91->7 (7*13), 17->17 (prime->self), 2->2. n<=1 reported. Complement of maxprimefactor.
# Self-checking: python first divisor >= 2, else n.
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
OUT="${TMPDIR:-/tmp}/minprimefactor.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/minprimefactor.png" \
    "echo warmup" "minprimefactor 15" "minprimefactor 100" "minprimefactor 91" \
    "minprimefactor 17" "minprimefactor 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "minprimefactor:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def lpf(n):                           # DERIVED reference
    d = 2
    while d * d <= n:
        if n % d == 0:
            return d
        d += 1
    return n

for n in [15, 100, 91, 17, 2]:
    w = f"minprimefactor: {n} -> {lpf(n)}"
    if w not in s:
        print(f"MINPRIMEFACTOR TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: odd composite, even, two odd primes, prime (self), smallest prime
assert "minprimefactor: 15 -> 3" in s
assert "minprimefactor: 100 -> 2" in s
assert "minprimefactor: 91 -> 7" in s          # 7*13 -> 7
assert "minprimefactor: 17 -> 17" in s          # prime -> itself
assert "minprimefactor: 2 -> 2" in s

print("MINPRIMEFACTOR TEST PASSED  (15->3, 100->2, 91->7, 17->17, 2->2 vs python least prime factor)")
PY
