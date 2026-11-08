#!/usr/bin/env bash
# Verify `compositorial <n>` = product of all composite numbers <= n (OEIS A036691): C(3)=1
# (empty), C(4)=4, C(6)=24, C(10)=17280, C(100) overflows u128. Composite analogue of
# primorial. Self-checking: python multiplies composites <= n with the same u128 overflow guard.
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
OUT="${TMPDIR:-/tmp}/compositorial.out"
# Warmup echo + sleep 8 keep the interactive capture from truncating; <=6 cmds.
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/compositorial.png" \
    "echo warmup" "compositorial 3" "compositorial 4" "compositorial 6" \
    "compositorial 10" "compositorial 100" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "compositorial:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def isp(k):
    if k < 2: return False
    if k % 2 == 0: return k == 2
    d = 3
    while d * d <= k:
        if k % d == 0: return False
        d += 2
    return True

U128 = (1 << 128) - 1
def line(n):
    p = 1
    for k in range(4, n + 1):
        if not isp(k):
            p *= k
    if p > U128:
        return f"compositorial: C({n}) overflowed u128"
    return f"compositorial: C({n}) = {p}"

for n in [3, 4, 6, 10, 100]:
    w = line(n)
    if w not in s:
        print(f"COMPOSITORIAL TEST FAILED (n={n} missing '{w}')"); sys.exit(1)

# pin the notable cases: empty product, first composite, 24, 17280, overflow
assert "compositorial: C(3) = 1" in s
assert "compositorial: C(4) = 4" in s
assert "compositorial: C(6) = 24" in s
assert "compositorial: C(10) = 17280" in s
assert "compositorial: C(100) overflowed u128" in s

print("COMPOSITORIAL TEST PASSED  (3->1, 4->4, 6->24, 10->17280, 100->overflow "
      "vs python composite product)")
PY
