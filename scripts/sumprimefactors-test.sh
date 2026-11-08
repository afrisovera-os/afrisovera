#!/usr/bin/env bash
# Verify `sumprimefactors <n>` sums prime factors WITH multiplicity (A001414, sopfr): 12->7 (2+2+3),
# 60->12 (2+2+3+5), 100->14 (2+2+5+5), 17->17 (prime), 2->2. n<=1 reported. Distinct from radical
# (product of distinct)/bigomega (count). Self-checking: python trial-division sopfr.
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
OUT="${TMPDIR:-/tmp}/sumprimefactors.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumprimefactors.png" \
    "echo warmup" "sumprimefactors 12" "sumprimefactors 60" "sumprimefactors 100" \
    "sumprimefactors 17" "sumprimefactors 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumprimefactors:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sopfr(n):                         # DERIVED reference
    total = 0
    d = 2
    while d * d <= n:
        while n % d == 0:
            total += d
            n //= d
        d += 1
    if n > 1:
        total += n
    return total

for n in [12, 60, 100, 17, 2]:
    w = f"sumprimefactors: {n} -> {sopfr(n)}"
    if w not in s:
        print(f"SUMPRIMEFACTORS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: repeated factor, four primes, two squared primes, prime, smallest
assert "sumprimefactors: 12 -> 7" in s       # 2+2+3
assert "sumprimefactors: 60 -> 12" in s       # 2+2+3+5
assert "sumprimefactors: 100 -> 14" in s      # 2+2+5+5
assert "sumprimefactors: 17 -> 17" in s       # prime
assert "sumprimefactors: 2 -> 2" in s

print("SUMPRIMEFACTORS TEST PASSED  (12->7, 60->12, 100->14, 17->17, 2->2 vs python sopfr with multiplicity)")
PY
