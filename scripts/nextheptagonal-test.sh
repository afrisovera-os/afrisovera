#!/usr/bin/env bash
# Verify `nextheptagonal <n>` reports the SMALLEST heptagonal Hep(k)=k(5k-3)/2 (k>=1) >= n + index --
# the heptagonal member of the nextsquare/nextcube/nexttriangular/nextpentagonal/nexthexagonal family:
# 2->Hep(2)=7, 7->Hep(2)=7 (already), 8->Hep(3)=18, 0->Hep(1)=1, 35->Hep(5)=55.
# Distinct from heptagonal (membership). Self-checking: python smallest-k.
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
OUT="${TMPDIR:-/tmp}/nextheptagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/nextheptagonal.png" \
    "echo warmup" "nextheptagonal 2" "nextheptagonal 7" "nextheptagonal 8" \
    "nextheptagonal 0" "nextheptagonal 35" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "nextheptagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hepta(k): return k * (5 * k - 3) // 2

def want(n):                          # DERIVED reference: smallest k>=1 with Hep(k) >= n
    k = 1
    while hepta(k) < n:
        k += 1
    return f"nextheptagonal: smallest heptagonal >= {n} is Hep({k}) = {hepta(k)}"

for n in (2, 7, 8, 0, 35):
    w = want(n)
    if w not in s:
        print(f"NEXTHEPTAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "nextheptagonal: smallest heptagonal >= 2 is Hep(2) = 7" in s
assert "nextheptagonal: smallest heptagonal >= 7 is Hep(2) = 7" in s
assert "nextheptagonal: smallest heptagonal >= 8 is Hep(3) = 18" in s
assert "nextheptagonal: smallest heptagonal >= 0 is Hep(1) = 1" in s
assert "nextheptagonal: smallest heptagonal >= 35 is Hep(5) = 55" in s

print("NEXTHEPTAGONAL TEST PASSED  (2->Hep(2)=7, 7->Hep(2)=7, 8->Hep(3)=18, 0->Hep(1)=1, "
      "35->Hep(5)=55 vs python smallest-k)")
PY
