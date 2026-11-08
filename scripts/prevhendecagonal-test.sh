#!/usr/bin/env bash
# Verify `prevhendecagonal <n>` reports the LARGEST hendecagonal Hen(k)=k(9k-7)/2 (k>=1:
# 1,11,30,58,95,141,...) that is <= n, with index k -- the mirror of nexthendecagonal, completing the
# 11-gonal pair. Tests 12->Hen(2)=11, 30->Hen(3)=30, 60->Hen(4)=58, 1->Hen(1)=1, and n<1->(none).
# Self-checking: python derives the largest hendecagonal <= n.
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
OUT="${TMPDIR:-/tmp}/prevhendecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevhendecagonal.png" \
    "echo warmup" "prevhendecagonal 12" "prevhendecagonal 30" "prevhendecagonal 60" \
    "prevhendecagonal 1" "prevhendecagonal 0" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevhendecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hendec(k): return k * (9 * k - 7) // 2

def prevhendec(n):                   # DERIVED reference: largest Hen(k) <= n, k>=1
    if n < 1: return None
    k = 1
    while hendec(k + 1) <= n: k += 1
    return k

for n in (12, 30, 60, 1):
    k = prevhendec(n)
    want = f"prevhendecagonal: largest hendecagonal <= {n} is Hen({k}) = {hendec(k)}"
    if want not in s:
        print(f"PREVHENDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

assert prevhendec(12) == 2 and hendec(2) == 11
assert prevhendec(30) == 3 and hendec(3) == 30
assert prevhendec(60) == 4 and hendec(4) == 58
assert prevhendec(1) == 1 and hendec(1) == 1
assert "prevhendecagonal: no hendecagonal <= 0 (none)" in s
print("PREVHENDECAGONAL TEST PASSED  (12/30/60/1 vs python largest-Hen<=n; 0->none; mirror of nexthendecagonal)")
PY
