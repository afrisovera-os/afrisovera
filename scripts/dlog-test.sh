#!/usr/bin/env bash
# Verify `dlog <g> <h> <p>` finds the smallest x>=0 with g^x = h (mod p), or reports none. Tests
# 3 4 7 -> 4, 2 3 5 -> 3, 3 1 7 -> 0, 5 2 23 -> (computed), 2 5 7 -> none (2 has order 3 mod 7).
# Self-checking: python walks powers of g.
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
OUT="${TMPDIR:-/tmp}/dlog.out"
python3 scripts/shell-demo.py 4998 4999 "${TMPDIR:-/tmp}/dlog.png" \
    "echo warmup" "dlog 3 4 7" "dlog 2 3 5" "dlog 3 1 7" "dlog 5 2 23" \
    "dlog 2 5 7" > "$OUT" 2>&1
kill "$QPID" 2>/dev/null
wait "$QPID" 2>/dev/null

grep -aE "dlog:" "$OUT" | sed 's/^/  /'

python3 - "$OUT" <<'PY'
import sys
s = open(sys.argv[1], "rb").read().decode("latin1")

def dlog(g, h, p):                   # DERIVED reference: smallest x with g^x==h mod p
    g, h, cur = g % p, h % p, 1 % p
    for x in range(p):
        if cur == h: return x
        cur = (cur * g) % p
    return None

for g, h, p in [(3, 4, 7), (2, 3, 5), (3, 1, 7), (5, 2, 23)]:
    x = dlog(g, h, p)
    want = f"dlog: {g}^{x} = {h} (mod {p})  ->  x = {x}"
    if want not in s:
        print(f"DLOG TEST FAILED (missing '{want}')"); sys.exit(1)

assert dlog(2, 5, 7) is None
if "dlog: no x with 2^x = 5 (mod 7)" not in s:
    print("DLOG TEST FAILED (missing no-solution report for 2 5 7)"); sys.exit(1)

assert dlog(3, 4, 7) == 4 and dlog(2, 3, 5) == 3 and dlog(3, 1, 7) == 0
print("DLOG TEST PASSED  (3^4=4/7, 2^3=3/5, 3^0=1/7, 5..2/23; 2^x=5/7 none vs python)")
PY
