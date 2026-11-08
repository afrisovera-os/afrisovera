#!/usr/bin/env bash
# Verify `isself <n>` classifies SELF (Colombian) numbers -- n with NO m such that m + digitsum(m) = n.
# Self numbers: 1,3,5,7,9,20,31,42,53,64,75,86,97. Tests members 20/31/97 and non-members 21/10.
# Self-checking: python brute-force over the generator g(m)=m+digitsum(m).
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
OUT="${TMPDIR:-/tmp}/isself.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/isself.png" \
    "echo warmup" "isself 20" "isself 31" "isself 97" "isself 21" "isself 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "isself:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def g(m): return m + sum(int(c) for c in str(m))

def is_self(n):                      # DERIVED reference: no m<n with g(m)==n
    return not any(g(m) == n for m in range(max(0, n - 200), n))

for n in (20, 31, 97):
    assert is_self(n), n
    want = f"isself: {n} IS a self number (no m with m + digitsum(m) = n)"
    if want not in s:
        print(f"ISSELF TEST FAILED (missing '{want}')"); sys.exit(1)

for n in (21, 10):
    assert not is_self(n), n
    want = f"isself: {n} is NOT a self number"
    if want not in s:
        print(f"ISSELF TEST FAILED (missing '{want}')"); sys.exit(1)

# sanity on the opening self-number sequence
assert [n for n in range(1, 100) if is_self(n)] == [1,3,5,7,9,20,31,42,53,64,75,86,97]
print("ISSELF TEST PASSED  (20/31/97 self, 21/10 not vs python generator)")
PY
