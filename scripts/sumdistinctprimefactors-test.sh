#!/usr/bin/env bash
# Verify `sumdistinctprimefactors <n>` sums DISTINCT prime factors (A008472, sopf): 12->5 (2+3),
# 60->10 (2+3+5), 100->7 (2+5), 17->17 (prime), 2->2. n<=1 reported. Complement of sumprimefactors
# (with mult); distinct from radical (product)/omega (count). Self-checking: python trial-division sopf.
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
OUT="${TMPDIR:-/tmp}/sumdistinctprimefactors.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumdistinctprimefactors.png" \
    "echo warmup" "sumdistinctprimefactors 12" "sumdistinctprimefactors 60" \
    "sumdistinctprimefactors 100" "sumdistinctprimefactors 17" "sumdistinctprimefactors 2" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumdistinctprimefactors:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def sopf(n):                          # DERIVED reference
    total = 0
    d = 2
    while d * d <= n:
        if n % d == 0:
            total += d
            while n % d == 0:
                n //= d
        d += 1
    if n > 1:
        total += n
    return total

for n in [12, 60, 100, 17, 2]:
    w = f"sumdistinctprimefactors: {n} -> {sopf(n)}"
    if w not in s:
        print(f"SUMDISTINCTPRIMEFACTORS TEST FAILED (missing '{w}')"); sys.exit(1)

# pin the notable branches: repeated factor counted once, three primes, squared primes, prime, smallest
assert "sumdistinctprimefactors: 12 -> 5" in s      # 2+3 (not 2+2+3)
assert "sumdistinctprimefactors: 60 -> 10" in s      # 2+3+5
assert "sumdistinctprimefactors: 100 -> 7" in s      # 2+5 (not 2+2+5+5)
assert "sumdistinctprimefactors: 17 -> 17" in s      # prime
assert "sumdistinctprimefactors: 2 -> 2" in s

print("SUMDISTINCTPRIMEFACTORS TEST PASSED  (12->5, 60->10, 100->7, 17->17, 2->2 vs python sopf distinct)")
PY
