#!/usr/bin/env bash
# Verify the `fermat <n>` command computes the nth Fermat number F(n) = 2^(2^n) + 1
# (F(0)=3, F(3)=257, F(5)=4294967297, F(6)=18446744073709551617). F(6) is the largest
# that fits u128; F(7) reports overflow. Distinct from mersenne (2^p-1). Self-checking:
# python computes 2**(2**n) + 1 (the big F(6) value is DERIVED by the loop).
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
OUT="${TMPDIR:-/tmp}/fermat.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 cmds.
# 3 (=257), 5 (=4294967297), 6 (largest that fits u128), 7 (overflow).
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/fermat.png" \
    "echo warmup" "fermat 3" "fermat 5" "fermat 6" "fermat 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "fermat\(|usage: fermat" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def fermat(n):
    return 2 ** (2 ** n) + 1

for n in [3, 5, 6]:
    want = f"fermat({n}) = {fermat(n)}"   # big F(6) value DERIVED by python, never hand-typed
    if want not in s:
        print(f"FERMAT TEST FAILED (missing '{want}')"); sys.exit(1)

# n=7 overflows u128 (F(7) = 2^128 + 1 > u128::MAX)
if "fermat(7) overflows u128 (F(7) = 2^128 + 1)" not in s:
    print("FERMAT TEST FAILED (missing overflow line)"); sys.exit(1)

# small pins
assert "fermat(3) = 257" in s
assert "fermat(5) = 4294967297" in s

print("FERMAT TEST PASSED  (F(3)=257, F(5)=4294967297, F(6) fits vs python, F(7) overflows; "
      "2^(2^n) + 1)")
PY
