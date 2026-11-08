#!/usr/bin/env bash
# Verify `prevdodecagonal <n>` reports the LARGEST dodecagonal Dod(k)=k(5k-4) (k>=1:
# 1,12,33,64,105,156,...) that is <= n, with index k -- the mirror of nextdodecagonal, completing the
# 12-gonal pair. Tests 13->Dod(2)=12, 33->Dod(3)=33, 100->Dod(4)=64, 1->Dod(1)=1, and n<1->(none).
# Self-checking: python derives the largest dodecagonal <= n.
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
OUT="${TMPDIR:-/tmp}/prevdodecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevdodecagonal.png" \
    "echo warmup" "prevdodecagonal 13" "prevdodecagonal 33" "prevdodecagonal 100" \
    "prevdodecagonal 1" "prevdodecagonal 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevdodecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dodec(k): return k * (5 * k - 4)

def prevdodec(n):                    # DERIVED reference: largest Dod(k) <= n, k>=1
    if n < 1: return None
    k = 1
    while dodec(k + 1) <= n: k += 1
    return k

for n in (13, 33, 100, 1):
    k = prevdodec(n)
    want = f"prevdodecagonal: largest dodecagonal <= {n} is Dod({k}) = {dodec(k)}"
    if want not in s:
        print(f"PREVDODECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

assert prevdodec(13) == 2 and dodec(2) == 12
assert prevdodec(33) == 3 and dodec(3) == 33
assert prevdodec(100) == 4 and dodec(4) == 64
assert prevdodec(1) == 1 and dodec(1) == 1
assert "prevdodecagonal: no dodecagonal <= 0 (none)" in s
print("PREVDODECAGONAL TEST PASSED  (13/33/100/1 vs python largest-Dod<=n; 0->none; mirror of nextdodecagonal)")
PY
