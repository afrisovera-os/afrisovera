#!/usr/bin/env bash
# Verify `stellaoctangula <n>` computes the nth stella octangula number n(2n^2-1) (OEIS
# A007588): S(1)=1, S(2)=14, S(3)=51, S(4)=124, S(10)=1990; the u64 max overflows u128.
# Self-checking: python n*(2*n*n-1) with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/stellaoctangula.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/stellaoctangula.png" \
    "echo warmup" "stellaoctangula 1" "stellaoctangula 2" "stellaoctangula 3" \
    "stellaoctangula 10" "stellaoctangula 18446744073709551615" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "stellaoctangula:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(n):
    v = n * (2 * n * n - 1)              # DERIVED stella octangula
    if v > U128:
        return f"stellaoctangula: S({n}) overflowed u128"
    return f"stellaoctangula: S({n}) = {v}"

for n in [1, 2, 3, 10, 18446744073709551615]:
    w = line(n)
    if w not in s:
        print(f"STELLAOCTANGULA TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable cases: base, second, third, tenth, u64-max overflow
assert "stellaoctangula: S(1) = 1" in s
assert "stellaoctangula: S(2) = 14" in s
assert "stellaoctangula: S(3) = 51" in s
assert "stellaoctangula: S(10) = 1990" in s
assert "stellaoctangula: S(18446744073709551615) overflowed u128" in s

print("STELLAOCTANGULA TEST PASSED  (1->1, 2->14, 3->51, 10->1990, u64-max overflows "
      "vs python n(2n^2-1))")
PY
