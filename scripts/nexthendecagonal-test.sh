#!/usr/bin/env bash
# Verify `nexthendecagonal <n>` reports the SMALLEST hendecagonal Hen(k)=k(9k-7)/2 (k>=1:
# 1,11,30,58,95,141,...) that is >= n, with index k -- the 11-gonal member of the next* family,
# pairing with the hendecagonal membership test. Tests 2->Hen(2)=11, 11->Hen(2)=11, 12->Hen(3)=30,
# 60->Hen(5)=95, 0->Hen(1)=1. Self-checking: python derives the smallest hendecagonal >= n.
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
OUT="${TMPDIR:-/tmp}/nexthendecagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nexthendecagonal.png" \
    "echo warmup" "nexthendecagonal 2" "nexthendecagonal 12" "nexthendecagonal 60" \
    "nexthendecagonal 0" "nexthendecagonal 11" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nexthendecagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hendec(k): return k * (9 * k - 7) // 2

def nexthendec(n):                   # DERIVED reference: smallest Hen(k) >= n, k>=1
    k = 1
    while hendec(k) < n: k += 1
    return k

for n in (2, 12, 60, 0, 11):
    k = nexthendec(n)
    want = f"nexthendecagonal: smallest hendecagonal >= {n} is Hen({k}) = {hendec(k)}"
    if want not in s:
        print(f"NEXTHENDECAGONAL TEST FAILED (missing '{want}')"); sys.exit(1)

assert nexthendec(2) == 2 and hendec(2) == 11
assert nexthendec(12) == 3 and hendec(3) == 30
assert nexthendec(60) == 5 and hendec(5) == 95
assert nexthendec(0) == 1 and hendec(1) == 1
assert nexthendec(11) == 2
print("NEXTHENDECAGONAL TEST PASSED  (2/12/60/0/11 vs python smallest-Hen>=n)")
PY
