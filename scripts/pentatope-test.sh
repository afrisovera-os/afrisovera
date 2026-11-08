#!/usr/bin/env bash
# Verify the `pentatope <n>` command computes the nth pentatope number
# P(n) = n*(n+1)*(n+2)*(n+3)/24 -- the 4D simplex figurate number (sum of the first n
# tetrahedral numbers). P(1..7) = 1,5,15,35,70,126,210. The 4D analogue of tetrahedral
# (3D) and sumto (2D triangular). The quadruple product is computed in u128 (checked).
# Self-checking: python replicates the closed form.
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
OUT="${TMPDIR:-/tmp}/pentatope.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=5 real cmds.
# 0 pins the zero edge; 200 pins a larger value on the u128 product path.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/pentatope.png" \
    "echo warmup" "pentatope 5" "pentatope 7" "pentatope 200" "pentatope 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "pentatope:|usage: pentatope" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def p(n):
    return n * (n + 1) * (n + 2) * (n + 3) // 24

for n in [5, 7, 200, 0]:
    want = f"pentatope: P({n}) = {p(n)}"
    if want not in s:
        print(f"PENTATOPE TEST FAILED (missing '{want}')"); sys.exit(1)

# pin the notable results explicitly
assert "pentatope: P(5) = 70" in s
assert "pentatope: P(7) = 210" in s
assert "pentatope: P(200) = 68685050" in s     # 200*201*202*203/24
assert "pentatope: P(0) = 0" in s

print("PENTATOPE TEST PASSED  (P(5)=70, P(7)=210, P(200)=68685050, P(0)=0 vs python; "
      "n(n+1)(n+2)(n+3)/24, the 4D analogue of tetrahedral)")
PY
