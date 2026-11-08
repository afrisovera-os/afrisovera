#!/usr/bin/env bash
# Verify `sumtotient <n>` = Phi(n) = sum of phi(1..n): Phi(1)=1, Phi(5)=10, Phi(10)=32,
# Phi(100)=3044, Phi(0)=0; n > 100000 -> too large. Distinct from totient (single phi).
# Self-checking: python sums its own trial-division phi over 1..n.
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
OUT="${TMPDIR:-/tmp}/sumtotient.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/sumtotient.png" \
    "echo warmup" "sumtotient 1" "sumtotient 5" "sumtotient 10" \
    "sumtotient 100" "sumtotient 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "sumtotient:|usage: sumtotient" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def phi(m):
    result, p, mm = m, 2, m
    while p * p <= mm:
        if mm % p == 0:
            while mm % p == 0: mm //= p
            result -= result // p
        p += 1
    if mm > 1:
        result -= result // mm
    return result

def Phi(n):
    return sum(phi(k) for k in range(1, n + 1))      # DERIVED totient summatory

for n in [1, 5, 10, 100, 0]:
    want = f"sumtotient: Phi({n}) = {Phi(n)}"
    if want not in s:
        print(f"SUMTOTIENT TEST FAILED (n={n} missing '{want}')"); sys.exit(1)

assert "sumtotient: Phi(1) = 1" in s
assert "sumtotient: Phi(5) = 10" in s
assert "sumtotient: Phi(10) = 32" in s
assert "sumtotient: Phi(100) = 3044" in s
assert "sumtotient: Phi(0) = 0" in s

print("SUMTOTIENT TEST PASSED  (1->1, 5->10, 10->32, 100->3044, 0->0 vs python totient sum)")
PY
