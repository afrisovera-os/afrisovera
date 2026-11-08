#!/usr/bin/env bash
# Verify `prevhexagonal <n>` reports the LARGEST hexagonal H(k)=k(2k-1) (k>=1) <= n + index -- the
# mirror of nexthexagonal: 7->H(2)=6, 6->H(2)=6 (already), 5->H(1)=1, 1->H(1)=1, 50->H(5)=45.
# Distinct from hexagonal (membership). Self-checking: python largest-k.
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
OUT="${TMPDIR:-/tmp}/prevhexagonal.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/prevhexagonal.png" \
    "echo warmup" "prevhexagonal 7" "prevhexagonal 6" "prevhexagonal 5" \
    "prevhexagonal 1" "prevhexagonal 50" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "prevhexagonal:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def hexa(k): return k * (2 * k - 1)

def want(n):                          # DERIVED reference: largest k>=1 with H(k) <= n
    k = 1
    while hexa(k + 1) <= n:
        k += 1
    return f"prevhexagonal: largest hexagonal <= {n} is H({k}) = {hexa(k)}"

for n in (7, 6, 5, 1, 50):
    w = want(n)
    if w not in s:
        print(f"PREVHEXAGONAL TEST FAILED (missing '{w}')"); sys.exit(1)

assert "prevhexagonal: largest hexagonal <= 7 is H(2) = 6" in s
assert "prevhexagonal: largest hexagonal <= 6 is H(2) = 6" in s
assert "prevhexagonal: largest hexagonal <= 5 is H(1) = 1" in s
assert "prevhexagonal: largest hexagonal <= 1 is H(1) = 1" in s
assert "prevhexagonal: largest hexagonal <= 50 is H(5) = 45" in s

print("PREVHEXAGONAL TEST PASSED  (7->H(2)=6, 6->H(2)=6, 5->H(1)=1, 1->H(1)=1, 50->H(5)=45 "
      "vs python largest-k)")
PY
