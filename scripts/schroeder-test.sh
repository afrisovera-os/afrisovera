#!/usr/bin/env bash
# Verify `schroeder <n>` computes the large Schroeder numbers S(n)
# (1,2,6,22,90,394,1806,8558,41586,206098,1037718,...). Tests n=0,1,4,6,10. Self-checking: python
# convolution recurrence S(0)=1, S(n)=S(n-1)+sum S(k)S(n-1-k).
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
OUT="${TMPDIR:-/tmp}/schroeder.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/schroeder.png" \
    "echo warmup" "schroeder 0" "schroeder 1" "schroeder 4" \
    "schroeder 6" "schroeder 10" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "schroeder:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def schroeder(N):
    S = [0] * (N + 1)
    S[0] = 1
    for m in range(1, N + 1):
        S[m] = S[m - 1] + sum(S[k] * S[m - 1 - k] for k in range(m))
    return S

S = schroeder(10)
for n in (0, 1, 4, 6, 10):
    want = f"schroeder: S({n}) = {S[n]}"
    if want not in s:
        print(f"SCHROEDER TEST FAILED (missing '{want}')"); sys.exit(1)

assert S[:11] == [1, 2, 6, 22, 90, 394, 1806, 8558, 41586, 206098, 1037718]
print("SCHROEDER TEST PASSED  (S(0)=1, S(4)=90, S(6)=1806, S(10)=1037718 vs python)")
PY
