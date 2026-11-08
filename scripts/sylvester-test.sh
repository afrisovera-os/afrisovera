#!/usr/bin/env bash
# Verify the `sylvester` command computes the nth term of Sylvester's sequence
# (a(0)=2, a(n)=a(n-1)^2 - a(n-1) + 1 -> 2,3,7,43,1807,3263443,10650056950807,...)
# in u64 with an overflow guard. The FIRST NONLINEAR (quadratic) recurrence in the
# suite: it SQUARES the prior term, giving doubly-exponential growth, so it
# overflows u64 at the earliest n yet. Self-checking: python recomputes a(n) by
# the same recurrence; the u64 boundary is a(6) fits / a(7) overflows.
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
OUT="${TMPDIR:-/tmp}/sylvester.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sylvester.png" \
    "sylvester 0" "sylvester 1" "sylvester 2" "sylvester 3" "sylvester 4" \
    "sylvester 5" "sylvester 6" "sylvester 7" "sylvester zz" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sylvester:|usage: sylvester" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def a(n):
    x = 2
    for _ in range(n):
        x = x * (x - 1) + 1
    return x

for n in [0, 1, 2, 3, 4, 5, 6]:
    want = f"sylvester: a({n}) = {a(n)}"
    if want not in s:
        print(f"SYLVESTER TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: a(6) fits, a(7) overflows.
M = 2**64 - 1
assert a(6) <= M and a(7) > M, "boundary assumption wrong"
if "sylvester: a(7) overflows u64" not in s:
    print("SYLVESTER TEST FAILED (missing a(7) overflow line)"); sys.exit(1)
if "usage: sylvester <n>" not in s:
    print("SYLVESTER TEST FAILED (missing usage line)"); sys.exit(1)

print("SYLVESTER TEST PASSED  (a0..a6 exact vs python, a(7) overflow-guarded, bad->usage)")
PY
