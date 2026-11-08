#!/usr/bin/env bash
# Verify `iscoprime <a> <b>` classifies whether two integers are coprime (gcd=1): 8 15 ->
# coprime, 12 18 -> NOT (gcd 6), 7 13 -> coprime, 6 9 -> NOT (gcd 3), 1 100 -> coprime.
# Distinct from gcd. Self-checking: python math.gcd.
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
OUT="${TMPDIR:-/tmp}/iscoprime.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/iscoprime.png" \
    "echo warmup" "iscoprime 8 15" "iscoprime 12 18" "iscoprime 7 13" \
    "iscoprime 6 9" "iscoprime 1 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "iscoprime:|usage: iscoprime" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def line(a, b):
    g = math.gcd(a, b)
    if g == 1:
        return f"iscoprime: {a} and {b} are coprime (gcd 1)"
    return f"iscoprime: {a} and {b} are NOT coprime (gcd {g})"

for a, b in [(8, 15), (12, 18), (7, 13), (6, 9), (1, 100)]:
    w = line(a, b)
    if w not in s:
        print(f"ISCOPRIME TEST FAILED (a={a} b={b} missing '{w}')"); sys.exit(1)

# pin the notable branches: coprime, not-coprime with gcd 6, two primes, gcd 3, 1-is-coprime
assert "iscoprime: 8 and 15 are coprime (gcd 1)" in s
assert "iscoprime: 12 and 18 are NOT coprime (gcd 6)" in s
assert "iscoprime: 7 and 13 are coprime (gcd 1)" in s
assert "iscoprime: 6 and 9 are NOT coprime (gcd 3)" in s
assert "iscoprime: 1 and 100 are coprime (gcd 1)" in s

print("ISCOPRIME TEST PASSED  (8/15 coprime, 12/18 not gcd6, 7/13 coprime, 6/9 not gcd3, "
      "1/100 coprime vs python math.gcd)")
PY
