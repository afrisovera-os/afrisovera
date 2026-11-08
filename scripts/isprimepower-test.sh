#!/usr/bin/env bash
# Verify the `isprimepower <n>` command classifies prime powers: n = p^k for a single
# prime p (7=7^1, 8=2^3, 9=3^2 IS; 12=2^2*3 and 36=2^2*3^2 NOT; n<2 out of range).
# Distinct from isperfectpower (any a^b: 36 IS a perfect power but NOT a prime power).
# Self-checking: python replicates the distinct-prime-count classification.
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
OUT="${TMPDIR:-/tmp}/isprimepower.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 8 (IS 2^3), 36 (NOT: perfect power but two distinct primes), 7 (IS 7^1), 1 (out of range).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isprimepower.png" \
    "echo warmup" "isprimepower 8" "isprimepower 36" "isprimepower 7" "isprimepower 1" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isprimepower:|usage: isprimepower" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):
    if n < 2:
        return f"isprimepower: {n} is NOT a prime power (needs n >= 2)"
    m = n; distinct = 0; base = 0; exp = 0; d = 2
    while d * d <= m:
        if m % d == 0:
            distinct += 1; base = d; exp = 0
            while m % d == 0:
                m //= d; exp += 1
        d += 1
    if m > 1:
        distinct += 1; base = m; exp = 1
    if distinct == 1:
        return f"isprimepower: {n} IS a prime power ({base}^{exp})"
    return f"isprimepower: {n} is NOT a prime power"

for n in [8, 36, 7, 1]:
    want = message(n)
    if want not in s:
        print(f"ISPRIMEPOWER TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly
assert "isprimepower: 8 IS a prime power (2^3)" in s
assert "isprimepower: 36 is NOT a prime power" in s   # perfect power (6^2) but two distinct primes
assert "isprimepower: 7 IS a prime power (7^1)" in s  # a prime itself
assert "isprimepower: 1 is NOT a prime power (needs n >= 2)" in s

print("ISPRIMEPOWER TEST PASSED  (8->2^3, 36->NOT (two primes), 7->7^1, 1->out of range vs python; "
      "n = p^k for a single prime)")
PY
