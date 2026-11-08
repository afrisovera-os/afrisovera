#!/usr/bin/env bash
# Verify `nonagonal <n>` classifies whether n is a NONAGONAL (9-gonal) number Non(k)=k(7k-5)/2
# (k>=1: 1,9,24,46,75,111,...), reporting the index k. n is nonagonal iff 56n+25 is a perfect
# square with root == -5 (mod 14); then k=(root+5)/14. 9->k2, 46->k4, 10 NOT. Extends the
# figurate family past octagonal. Self-checking: python computes the same gate + k.
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
OUT="${TMPDIR:-/tmp}/nonagonal.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nonagonal.png" \
    "echo warmup" "nonagonal 1" "nonagonal 9" "nonagonal 46" "nonagonal 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nonagonal:|usage: nonagonal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys, math
s = open(sys.argv[1], "rb").read().decode("latin1")

def message(n):  # mirror the kernel gate exactly
    m = 56 * n + 25
    r = math.isqrt(m)
    if r * r == m and (r + 5) % 14 == 0:
        k = (r + 5) // 14
        return f"nonagonal: {n} IS nonagonal (Non({k}) = {n})"
    return f"nonagonal: {n} is NOT nonagonal"

for n in [1, 9, 46, 10]:
    want = message(n)   # DERIVED by python, never hand-typed
    if want not in s:
        print(f"NONAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable branches explicitly: k index derivation + a non-member
assert "nonagonal: 1 IS nonagonal (Non(1) = 1)" in s
assert "nonagonal: 9 IS nonagonal (Non(2) = 9)" in s
assert "nonagonal: 46 IS nonagonal (Non(4) = 46)" in s
assert "nonagonal: 10 is NOT nonagonal" in s

print("NONAGONAL TEST PASSED  (1=Non(1), 9=Non(2), 46=Non(4), 10 not a member "
      "vs python k(7k-5)/2 gate)")
PY
