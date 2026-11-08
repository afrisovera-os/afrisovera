#!/usr/bin/env bash
# Verify `npr <n> <r>` computes permutations P(n,r) = n!/(n-r)!: P(5,2)=20, P(5,5)=120,
# P(10,3)=720, P(3,5)=0 (r>n), and a large case (50,40) overflows u128. Ordered companion of
# ncr. Self-checking: python multiplies the falling factorial and applies the same u128 guard.
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
OUT="${TMPDIR:-/tmp}/npr.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/npr.png" \
    "echo warmup" "npr 5 2" "npr 5 5" "npr 10 3" \
    "npr 3 5" "npr 50 40" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "npr:|usage: npr" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

U128 = (1 << 128) - 1
def line(n, r):
    if r > n:
        return f"npr: P({n}, {r}) = 0"
    v = 1
    for i in range(r):
        v *= (n - i)                                 # DERIVED falling-factorial product
    if v > U128:
        return f"npr: P({n}, {r}) overflows (n too large)"
    return f"npr: P({n}, {r}) = {v}"

for n, r in [(5, 2), (5, 5), (10, 3), (3, 5), (50, 40)]:
    w = line(n, r)
    if w not in s:
        print(f"NPR TEST FAILED (n={n} r={r} missing '{w}')"); sys.exit(1)

# pin the notable branches: basic, full n=r, mid, r>n zero, u128 overflow
assert "npr: P(5, 2) = 20" in s
assert "npr: P(5, 5) = 120" in s
assert "npr: P(10, 3) = 720" in s
assert "npr: P(3, 5) = 0" in s
assert "npr: P(50, 40) overflows (n too large)" in s

print("NPR TEST PASSED  (5 2->20, 5 5->120, 10 3->720, 3 5->0, 50 40->overflow "
      "vs python falling-factorial)")
PY
