#!/usr/bin/env bash
# Verify `nexthexagonal <n>` reports the SMALLEST hexagonal H(k)=k(2k-1) (k>=1) >= n + index --
# the hexagonal member of the nextsquare/nextcube/nexttriangular/nextpentagonal family:
# 2->H(2)=6, 6->H(2)=6 (already), 7->H(3)=15, 0->H(1)=1, 29->H(5)=45.
# Distinct from hexagonal (membership). Self-checking: python smallest-k.
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
OUT="${TMPDIR:-/tmp}/nexthexagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nexthexagonal.png" \
    "echo warmup" "nexthexagonal 2" "nexthexagonal 6" "nexthexagonal 7" \
    "nexthexagonal 0" "nexthexagonal 29" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nexthexagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hexa(k): return k * (2 * k - 1)

def want(n):                          # DERIVED reference: smallest k>=1 with H(k) >= n
    k = 1
    while hexa(k) < n:
        k += 1
    return f"nexthexagonal: smallest hexagonal >= {n} is H({k}) = {hexa(k)}"

for n in (2, 6, 7, 0, 29):
    w = want(n)
    if w not in s:
        print(f"NEXTHEXAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "nexthexagonal: smallest hexagonal >= 2 is H(2) = 6" in s
assert "nexthexagonal: smallest hexagonal >= 6 is H(2) = 6" in s
assert "nexthexagonal: smallest hexagonal >= 7 is H(3) = 15" in s
assert "nexthexagonal: smallest hexagonal >= 0 is H(1) = 1" in s
assert "nexthexagonal: smallest hexagonal >= 29 is H(5) = 45" in s

print("NEXTHEXAGONAL TEST PASSED  (2->H(2)=6, 6->H(2)=6, 7->H(3)=15, 0->H(1)=1, 29->H(5)=45 "
      "vs python smallest-k)")
PY
