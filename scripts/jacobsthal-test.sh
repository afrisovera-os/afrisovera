#!/usr/bin/env bash
# Verify the `jacobsthal` command computes the nth Jacobsthal number (J0=0, J1=1,
# J(n)=J(n-1)+2*J(n-2) -> 0,1,1,3,5,11,21,43,85,171,341,...) in u64 with an
# overflow guard. DISTINCT from fib (prev1+prev2) and pell (2*prev1+prev2): the
# "2*" sits on the SECOND-back term (prev2). Self-checking: python recomputes
# J(n) by the same recurrence; the u64 boundary is J(65) fits / J(66) overflows
# (J grows ~2^n/3).
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
OUT="${TMPDIR:-/tmp}/jacobsthal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/jacobsthal.png" \
    "jacobsthal 0" "jacobsthal 1" "jacobsthal 2" "jacobsthal 3" "jacobsthal 5" \
    "jacobsthal 10" "jacobsthal 20" "jacobsthal 65" "jacobsthal 66" \
    "jacobsthal ab" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "jacobsthal:|usage: jacobsthal" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def J(n):
    if n == 0:
        return 0
    a, b = 0, 1
    for _ in range(2, n + 1):
        a, b = b, b + 2 * a
    return b

for n in [0, 1, 2, 3, 5, 10, 20, 65]:
    want = f"jacobsthal: J({n}) = {J(n)}"
    if want not in s:
        print(f"JACOBSTHAL TEST FAILED (missing '{want}')"); sys.exit(1)

# u64 boundary: J(65) fits, J(66) overflows.
M = 2**64 - 1
assert J(65) <= M and J(66) > M, "boundary assumption wrong"
if "jacobsthal: J(66) overflows u64" not in s:
    print("JACOBSTHAL TEST FAILED (missing J(66) overflow line)"); sys.exit(1)
if "usage: jacobsthal <n>" not in s:
    print("JACOBSTHAL TEST FAILED (missing usage line)"); sys.exit(1)

print("JACOBSTHAL TEST PASSED  (J0..J20 & J(65) exact vs python, J(66) overflow-guarded, bad->usage)")
PY
